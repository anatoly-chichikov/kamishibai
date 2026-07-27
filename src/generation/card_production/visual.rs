//! Scene composition and manga rendering for one card.

use std::path::PathBuf;

use anyhow::{Result, anyhow};

use super::artifact_file;
use super::attempt_archive::{archived_reply, archived_sequence, latest_verdict};
use super::cost_accounting::{
    AccountingHealth, CostAccounting, CostRecorder, attach_scene_cost, visual_costs,
};
use super::gemini_media::{GeminiRecall, MeteredGemini};
use super::picture_recovery::{
    LocalImageRejection, NoopProgress, PictureRecovery, render_recomposition_with_fallback,
};
use super::picture_requests::RequestCountingImage;
use super::scene_attempt::{reserve_scene_attempt, scene_attempt_cursor};
use crate::gemini::GeminiAccess;
use crate::generation::artifact_cache::{Cache, IMAGE_ATTEMPTS_DIRECTORY, VISUAL_LOCK_TIMEOUT};
use crate::generation::manga::{
    BorderDetector, HiddenRecall, Illustration, ImageSource, MangaRenderRejection, MangaRenderer,
    RecallCard, RecallJudge, ShownRecall,
};
use crate::generation::{SceneComposer, visual_revision};
use crate::languages::LanguageCatalog;
use crate::session::{Artifact, ArtifactAttempt, ArtifactFile, CardCell, CardDraft};

const IMAGE_ATTEMPTS_PER_ARTIFACT: usize = 1;

type GeminiIllustration = Illustration<SceneComposer<MeteredGemini>, MangaRenderer<GeminiRecall>>;

/// Produces scenes and pictures while preserving visual recovery state.
#[derive(Clone)]
pub(super) struct VisualProduction {
    cache: PathBuf,
    catalog: LanguageCatalog,
    access: GeminiAccess,
    state: VisualState,
}

#[derive(Clone)]
struct VisualState {
    pictures: PictureRecovery,
    costs: CostAccounting,
}

impl VisualState {
    fn new(costs: CostAccounting) -> Self {
        Self {
            pictures: PictureRecovery::default(),
            costs,
        }
    }
}

impl VisualProduction {
    /// Bind visual production to languages, Gemini, cache, and accounting.
    #[must_use]
    pub(super) fn new(
        cache: PathBuf,
        catalog: LanguageCatalog,
        access: GeminiAccess,
        costs: CostAccounting,
    ) -> Self {
        Self {
            cache,
            catalog,
            access,
            state: VisualState::new(costs),
        }
    }

    /// Compose a scene attributed to one stable card slot.
    pub(super) fn scene(&self, slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
        let attempt = self.generate(
            slot,
            draft,
            Artifact::Scene,
            draft.artifacts().scene().tally().done(),
            false,
            |illustration, sentence, target, progress, _accounting| {
                illustration.scene_only(sentence, target, progress)
            },
        );
        match self.cell(draft).cache().visual(visual_revision()) {
            Ok(cache) => archived_reply(attempt, &cache),
            Err(_) => attempt,
        }
    }

    /// Render a picture attributed to one stable card slot.
    pub(super) fn picture(&self, slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
        let cache = match self.cell(draft).cache().visual(visual_revision()) {
            Ok(cache) => cache,
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        };
        let key = cache.path();
        let done = draft.artifacts().picture().tally().done();
        let recover = match self.state.pictures.prepare(key.as_path(), done) {
            Ok(recover) => recover,
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        };
        let cursor = match scene_attempt_cursor(&cache, draft.artifacts().scene().tally().done()) {
            Ok(cursor) => cursor,
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        };
        let recover = cursor.recompose(recover);
        let archived = archived_sequence(&cache);
        let attempt = if recover {
            let fallback_cache = cache.clone();
            self.generate(
                slot,
                draft,
                Artifact::Picture,
                draft.artifacts().scene().tally().done(),
                true,
                move |illustration, sentence, target, progress, accounting| {
                    render_recomposition_with_fallback(
                        &fallback_cache,
                        accounting,
                        progress,
                        |progress| {
                            illustration.picture_with_recomposed_scene(sentence, target, progress)
                        },
                        |progress| illustration.picture_only(sentence, target, progress),
                    )
                },
            )
        } else {
            self.generate(
                slot,
                draft,
                Artifact::Picture,
                draft.artifacts().scene().tally().done(),
                false,
                |illustration, sentence, target, progress, _accounting| {
                    illustration.picture_only(sentence, target, progress)
                },
            )
        };
        let local_rejection = attempt
            .error()
            .and_then(|error| error.downcast_ref::<MangaRenderRejection>())
            .map(|rejection| LocalImageRejection::from_category(rejection.category()));
        if let Err(error) = self
            .state
            .pictures
            .observe(key.as_path(), done, local_rejection)
        {
            return ArtifactAttempt::unmetered(Err(error));
        }
        judged(attempt, &cache, archived)
    }

    fn generate<F>(
        &self,
        slot: usize,
        draft: &CardDraft,
        artifact: Artifact,
        fallback: u8,
        recompose: bool,
        render: F,
    ) -> ArtifactAttempt<ArtifactFile>
    where
        F: FnOnce(
            &GeminiIllustration,
            &str,
            &str,
            &mut NoopProgress,
            &AccountingHealth,
        ) -> Result<(String, bool)>,
    {
        let Some(meta) = draft.meta() else {
            return ArtifactAttempt::unmetered(Err(anyhow!(
                "meta must be ready before {}",
                artifact.label()
            )));
        };
        let learning = draft.pair().learning();
        let cache = match self.cell(draft).cache().visual(visual_revision()) {
            Ok(cache) => cache,
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        };
        let accounting = AccountingHealth::default();
        let scene_costs = self.state.costs.guarded(
            cache.clone(),
            Artifact::Scene,
            Some(slot),
            accounting.clone(),
        );
        let picture_costs = self.state.costs.guarded(
            cache.clone(),
            Artifact::Picture,
            Some(slot),
            accounting.clone(),
        );
        let _guard = match cache.hold_visual(VISUAL_LOCK_TIMEOUT) {
            Ok(guard) => guard,
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        };
        let scene_attempt = match reserve_scene_attempt(&cache, artifact, fallback, recompose) {
            Ok(attempt) => attempt,
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        };
        let illustration = match self.illustration(
            draft,
            cache.clone(),
            scene_costs.clone(),
            picture_costs.clone(),
            scene_attempt,
            accounting.clone(),
        ) {
            Ok(illustration) => illustration,
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        };
        let mut progress = NoopProgress;
        let result = render(
            &illustration,
            meta.target_sentence(),
            learning,
            &mut progress,
            &accounting,
        )
        .and_then(|(filename, cached)| {
            let path = illustration.filepath(filename.as_str())?;
            Ok((filename, path, cached))
        });
        match result {
            Ok((filename, path, cached)) => {
                let (cost, related) =
                    match visual_costs(artifact, cached, &scene_costs, &picture_costs) {
                        Ok(costs) => costs,
                        Err(error) => return ArtifactAttempt::unmetered(Err(error)),
                    };
                attach_scene_cost(
                    ArtifactAttempt::new(Ok(artifact_file(filename, path, cached, cost)), cost),
                    artifact,
                    related,
                )
            }
            Err(error) => {
                let (cost, related) =
                    match visual_costs(artifact, false, &scene_costs, &picture_costs) {
                        Ok(costs) => costs,
                        Err(cost_error) => return ArtifactAttempt::unmetered(Err(cost_error)),
                    };
                attach_scene_cost(ArtifactAttempt::new(Err(error), cost), artifact, related)
            }
        }
    }

    fn illustration(
        &self,
        draft: &CardDraft,
        cache: Cache,
        scene_costs: CostRecorder,
        picture_costs: CostRecorder,
        scene_attempt: u8,
        accounting: AccountingHealth,
    ) -> Result<GeminiIllustration> {
        let meta = draft
            .meta()
            .ok_or_else(|| anyhow!("meta must be ready before illustration"))?;
        let learning = self.catalog.item(draft.pair().learning())?;
        let known = self.catalog.item(draft.pair().known())?;
        let client = self.access.client()?;
        let scene_client = MeteredGemini::new(client.clone(), scene_costs);
        let recall = GeminiRecall::new(
            client.clone(),
            RecallCard::new(
                ShownRecall::new(
                    known.prompt,
                    meta.source_sentence(),
                    meta.source_highlight(),
                    meta.source_hint(),
                ),
                HiddenRecall::from_profile(&learning, draft.term(), meta.target_sentence()),
            ),
            picture_costs.clone(),
        );
        let picture_client = RequestCountingImage::guarded(
            MeteredGemini::new(client, picture_costs),
            cache.clone(),
            accounting,
        );
        let renderer =
            production_renderer(picture_client, recall, BorderDetector::new(6, 24, 240, 10));
        let renderer = renderer.with_attempt_archive(cache.filepath(IMAGE_ATTEMPTS_DIRECTORY)?);
        Ok(Illustration::new(
            cache,
            SceneComposer::new(
                scene_client,
                learning.prompt.as_str(),
                draft.term(),
                scene_attempt,
            ),
            renderer,
        ))
    }

    fn cell(&self, draft: &CardDraft) -> CardCell {
        CardCell::new(
            self.cache.clone(),
            draft.pair(),
            draft.term(),
            draft.understanding(),
        )
    }
}

/// Attach the archived verdict of one picture attempt that the provider judged.
///
/// A verdict only belongs to this attempt when the archive actually grew: an
/// attempt that failed before reaching the provider keeps the plain error, so
/// the shell never blames a fresh failure on an older rejected picture.
pub(super) fn judged(
    attempt: ArtifactAttempt<ArtifactFile>,
    cache: &Cache,
    archived: usize,
) -> ArtifactAttempt<ArtifactFile> {
    if attempt.error().is_none() {
        return attempt;
    }
    match latest_verdict(cache).filter(|verdict| verdict.sequence() > archived) {
        Some(verdict) => attempt.with_fault(verdict.fault()),
        None => attempt,
    }
}

/// Build the one-request renderer used by one outer artifact attempt.
pub(super) fn production_renderer<C, J>(
    client: C,
    recall: J,
    border: BorderDetector,
) -> MangaRenderer<J>
where
    C: ImageSource + 'static,
    J: RecallJudge,
{
    MangaRenderer::new(client, IMAGE_ATTEMPTS_PER_ARTIFACT, recall, border)
}
