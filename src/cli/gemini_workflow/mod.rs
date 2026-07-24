//! Gemini-backed implementation of the UI-neutral card workflow.
//!
//! The root coordinates card stages. Child modules own cost accounting,
//! provider ports, durable picture-request budgets, and visual recovery.

mod cost_accounting;
mod gemini_media;
mod picture_requests;
mod visual_generation;

use std::fs;
use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use time::OffsetDateTime;
use time::format_description::parse as parse_time;

use super::card_workflow::{
    CardGeneration, DeckPublishing, KeyValidation, PublishPhase, PublishProgress,
};
use super::session::SessionCostScope;
use crate::anki::{CardModel, StableId, VocabularyDeck, VocabularyNote};
use crate::config::default_store;
use crate::gemini::{GeminiClient, HttpTransport};
use crate::generation::artifact_cache::{
    Cache, ILLUSTRATION_FILE, IMAGE_ATTEMPTS_DIRECTORY, ROOT_STAGE_LOCK_TIMEOUT, RootStage,
    VISUAL_LOCK_TIMEOUT, VOICE_FILE,
};
use crate::generation::manga::{
    BorderDetector, HiddenRecall, Illustration, ImageSource, MangaRenderRejection, MangaRenderer,
    RecallCard, RecallJudge, ShownRecall,
};
use crate::generation::speech::Audio;
use crate::generation::{SceneComposer, render_audio_prompt, visual_revision};
use crate::languages::{LanguageCatalog, catalog, naming};
use crate::report::{CardSheet, Thumbnail};
use crate::runtime::locations::{LocationArgs, Locations, SystemContext};
use crate::session::{
    Artifact, ArtifactAttempt, ArtifactFile, BulkCorrection, CachedUnderstanding, CardCell,
    CardCorrection, CardDraft, CardMeta, CardMetaCache, CardMetaGeneration, CardRevision,
    GenerationCost, LanguagePair, RawInputBatch, Understanding, Understood, WordCandidate,
    to_entry,
};
use crate::vocabulary::VocabularyEntry;

const IMAGE_STYLE: &str = "max-width: 100%; height: auto; border-radius: 10px";
const IMAGE_ATTEMPTS_PER_ARTIFACT: usize = 1;

type GeminiIllustration = Illustration<SceneComposer<MeteredGemini>, MangaRenderer<GeminiRecall>>;

/// Where the Gemini workflow looks for its API key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KeyLookup {
    /// Interactive flow: use the key validated and saved through Welcome.
    Saved,
    /// Console flow: `GEMINI_API_KEY` wins, falling back to the saved key.
    Environment,
}

/// Executes the complete card workflow through Gemini and the on-disk cache.
#[derive(Clone)]
pub(super) struct GeminiCardWorkflow {
    cache: PathBuf,
    output: PathBuf,
    catalog: LanguageCatalog,
    state: WorkflowState,
}

#[derive(Clone, Debug)]
struct WorkflowState {
    keys: KeyLookup,
    pictures: PictureRecovery,
    costs: Option<SessionCostScope>,
    slot: Option<usize>,
}

impl WorkflowState {
    fn new(keys: KeyLookup) -> Self {
        Self {
            keys,
            pictures: PictureRecovery::default(),
            costs: None,
            slot: None,
        }
    }
}

impl GeminiCardWorkflow {
    /// Build a Gemini workflow for the interactive flow using the saved key.
    pub(super) fn new(cache: PathBuf, output: PathBuf) -> Self {
        Self {
            cache,
            output,
            catalog: catalog(),
            state: WorkflowState::new(KeyLookup::Saved),
        }
    }

    /// Build a Gemini workflow for the console flow, where `GEMINI_API_KEY`
    /// is the documented key source and wins over any saved preference.
    pub(super) fn for_console(cache: PathBuf, output: PathBuf) -> Self {
        Self {
            cache,
            output,
            catalog: catalog(),
            state: WorkflowState::new(KeyLookup::Environment),
        }
    }

    /// Return the generator attributed to one persistent session cost scope.
    pub(super) fn with_session_costs(mut self, costs: SessionCostScope) -> Self {
        self.state.costs = Some(costs);
        self
    }

    fn in_slot(&self, slot: usize) -> Self {
        let mut scoped = self.clone();
        scoped.state.slot = Some(slot);
        scoped
    }

    fn cost_recorder(&self, cache: Cache, artifact: Artifact) -> CostRecorder {
        self.cost_recorder_with_accounting(cache, artifact, AccountingHealth::default())
    }

    fn cost_recorder_with_accounting(
        &self,
        cache: Cache,
        artifact: Artifact,
        accounting: AccountingHealth,
    ) -> CostRecorder {
        let session = self
            .state
            .costs
            .clone()
            .zip(self.state.slot)
            .map(|(scope, slot)| SessionCostAttribution::new(scope, slot));
        CostRecorder::guarded(cache, artifact, session, accounting)
    }

    fn client(&self) -> Result<GeminiClient<HttpTransport>> {
        let saved_key = default_store(&SystemContext)
            .ok()
            .and_then(|store| store.read().ok())
            .and_then(|prefs| prefs.api_key);
        match self.state.keys {
            KeyLookup::Saved => GeminiClient::from_saved(saved_key.as_deref()),
            KeyLookup::Environment => GeminiClient::from_env_or_saved(saved_key.as_deref()),
        }
    }

    fn meta_cache(&self) -> CardMetaCache {
        CardMetaCache::new(self.cache.clone())
    }

    fn meta_attempt(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
    ) -> ArtifactAttempt<(CardMeta, Option<ArtifactFile>)> {
        let cache = CardCell::new(self.cache.clone(), pair, term, understanding).cache();
        let _guard = match cache.hold_root_stage(RootStage::Meta, ROOT_STAGE_LOCK_TIMEOUT) {
            Ok(guard) => guard,
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        };
        match self.meta_cache().load(term, understanding, pair) {
            Ok(Some(meta)) => {
                let result = self
                    .store_card_meta_unlocked(term, understanding, pair, &meta)
                    .map(|file| (meta, Some(file)));
                return ArtifactAttempt::unmetered(result);
            }
            Ok(None) => {}
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        }
        let client = match self.client() {
            Ok(client) => client,
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        };
        let costs = self.cost_recorder(cache, Artifact::Meta);
        let result = client
            .generate_card_meta_observed(term, understanding, pair, |record| costs.push(record))
            .and_then(|meta| {
                self.store_card_meta_unlocked(term, understanding, pair, &meta)
                    .map(|file| (meta, Some(file)))
            });
        match costs.cumulative(false) {
            Ok(cost) => ArtifactAttempt::new(result, cost),
            Err(error) => ArtifactAttempt::unmetered(Err(error)),
        }
    }

    fn correction_attempt(
        &self,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
    ) -> ArtifactAttempt<CardRevision> {
        let client = match self.client() {
            Ok(client) => client,
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        };
        let cache = CardCell::new(
            self.cache.clone(),
            pair,
            draft.term(),
            draft.understanding(),
        )
        .cache();
        let costs = self.cost_recorder(cache, Artifact::Meta);
        let result =
            client.correct_card_observed(draft, comment, pair, |cost| costs.push_correction(cost));
        match costs.current(false) {
            Ok(delta) => ArtifactAttempt::new(result, delta),
            Err(error) => ArtifactAttempt::unmetered(Err(error)),
        }
    }

    fn store_card_meta_unlocked(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        meta: &CardMeta,
    ) -> Result<ArtifactFile> {
        let (filename, path, cached) = self.meta_cache().store(term, understanding, pair, meta)?;
        Ok(artifact_file(filename, path, cached, None))
    }

    fn cell(&self, draft: &CardDraft) -> CardCell {
        CardCell::new(
            self.cache.clone(),
            draft.pair(),
            draft.term(),
            draft.understanding(),
        )
    }

    fn audio(&self, draft: &CardDraft, costs: CostRecorder) -> Result<Audio<MeteredGemini>> {
        let item = self.catalog.item(draft.pair().learning())?;
        Ok(Audio::new(
            self.cell(draft).cache(),
            render_audio_prompt(item.prompt.as_str()),
            MeteredGemini::new(self.client()?, costs),
        ))
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
        let client = self.client()?;
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
                HiddenRecall::new(
                    learning.prompt.clone(),
                    draft.term(),
                    meta.target_sentence(),
                ),
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

    fn generate_visual<F>(
        &self,
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
        let scene_costs =
            self.cost_recorder_with_accounting(cache.clone(), Artifact::Scene, accounting.clone());
        let picture_costs = self.cost_recorder_with_accounting(
            cache.clone(),
            Artifact::Picture,
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
}

impl Understanding for GeminiCardWorkflow {
    fn understand(&self, raw: &RawInputBatch, my: &str) -> Result<Understood> {
        CachedUnderstanding::new(self.client()?, self.cache.clone()).understand(raw, my)
    }
}

impl BulkCorrection for GeminiCardWorkflow {
    fn correct_bulk(
        &self,
        candidate: &WordCandidate,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<crate::session::SenseCorrection> {
        self.client()?.correct_bulk(candidate, comment, pair)
    }
}

impl CardMetaGeneration for GeminiCardWorkflow {
    fn generate_card_meta(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
    ) -> Result<CardMeta> {
        self.meta_attempt(term, understanding, pair)
            .into_result()
            .map(|(meta, _file)| meta)
    }
}

impl CardCorrection for GeminiCardWorkflow {
    fn correct_card(
        &self,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<CardRevision> {
        self.correction_attempt(draft, comment, pair).into_result()
    }

    fn correct_card_accounted(
        &self,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
    ) -> ArtifactAttempt<CardRevision> {
        self.correction_attempt(draft, comment, pair)
    }
}

impl KeyValidation for GeminiCardWorkflow {
    fn check_key(&self, key: &str) -> Result<()> {
        GeminiClient::new(key, HttpTransport::new()).validate_key()
    }
}

impl CardGeneration for GeminiCardWorkflow {
    fn generate_meta(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
    ) -> ArtifactAttempt<(CardMeta, Option<ArtifactFile>)> {
        self.meta_attempt(term, understanding, pair)
    }

    fn generate_meta_in(
        &self,
        slot: usize,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
    ) -> ArtifactAttempt<(CardMeta, Option<ArtifactFile>)> {
        self.in_slot(slot).meta_attempt(term, understanding, pair)
    }

    fn generate_scene(&self, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
        self.generate_visual(
            draft,
            Artifact::Scene,
            draft.artifacts().scene().tally().done(),
            false,
            |illustration, sentence, target, progress, _accounting| {
                illustration.scene_only(sentence, target, progress)
            },
        )
    }

    fn generate_scene_in(&self, slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
        self.in_slot(slot).generate_scene(draft)
    }

    fn generate_picture(&self, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
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
        let attempt = if recover {
            let fallback_cache = cache.clone();
            self.generate_visual(
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
            self.generate_visual(
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
        attempt
    }

    fn generate_picture_in(&self, slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
        self.in_slot(slot).generate_picture(draft)
    }

    fn generate_sound(&self, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
        let Some(meta) = draft.meta() else {
            return ArtifactAttempt::unmetered(Err(anyhow!("meta must be ready before sound")));
        };
        let cache = self.cell(draft).cache();
        let _guard = match cache.hold_root_stage(RootStage::Voice, ROOT_STAGE_LOCK_TIMEOUT) {
            Ok(guard) => guard,
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        };
        if cache.exists(VOICE_FILE) {
            let path = match cache.filepath(VOICE_FILE) {
                Ok(path) => path,
                Err(error) => return ArtifactAttempt::unmetered(Err(error)),
            };
            return ArtifactAttempt::unmetered(Ok(artifact_file(
                String::from(VOICE_FILE),
                path,
                true,
                None,
            )));
        }
        let costs = self.cost_recorder(cache.clone(), Artifact::Sound);
        let result = (|| {
            let audio = self.audio(draft, costs.clone())?;
            audio
                .generate(meta.target_sentence())
                .and_then(|(filename, cached)| {
                    let path = audio.filepath(filename.as_str())?;
                    Ok((filename, path, cached))
                })
        })();
        match result {
            Ok((filename, path, cached)) => {
                let cost = match costs.cumulative(cached) {
                    Ok(cost) => cost,
                    Err(error) => return ArtifactAttempt::unmetered(Err(error)),
                };
                ArtifactAttempt::new(Ok(artifact_file(filename, path, cached, cost)), cost)
            }
            Err(error) => {
                let cost = match costs.cumulative(false) {
                    Ok(cost) => cost,
                    Err(cost_error) => return ArtifactAttempt::unmetered(Err(cost_error)),
                };
                ArtifactAttempt::new(Err(error), cost)
            }
        }
    }

    fn generate_sound_in(&self, slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
        self.in_slot(slot).generate_sound(draft)
    }

    fn correct_card_in(
        &self,
        slot: usize,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
    ) -> ArtifactAttempt<CardRevision> {
        self.in_slot(slot).correction_attempt(draft, comment, pair)
    }

    fn store_card_meta(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        meta: &CardMeta,
    ) -> Result<ArtifactFile> {
        let cache = CardCell::new(self.cache.clone(), pair, term, understanding).cache();
        let _guard = cache.hold_root_stage(RootStage::Meta, ROOT_STAGE_LOCK_TIMEOUT)?;
        self.store_card_meta_unlocked(term, understanding, pair, meta)
    }
}

impl DeckPublishing for GeminiCardWorkflow {
    fn publish_deck(
        &self,
        drafts: &[CardDraft],
        progress: &dyn PublishProgress,
    ) -> Result<(String, String, String)> {
        progress.advance(PublishPhase::Deck);
        fs::create_dir_all(&self.output)?;
        let completed = drafts
            .iter()
            .filter(|draft| draft.artifacts().all_ready())
            .collect::<Vec<_>>();
        let entries: Vec<VocabularyEntry> = completed
            .iter()
            .copied()
            .map(to_entry)
            .collect::<Result<Vec<_>>>()?;
        if entries.is_empty() {
            bail!("no completed cards to publish");
        }
        let decknaming = naming(None, entries.as_slice());
        let model = CardModel::new().model();
        let mut container = VocabularyDeck::new(
            StableId::new(decknaming.name.as_str()).value(),
            decknaming.name.as_str(),
            VocabularyNote::new(model),
            Vec::<(PathBuf, String)>::new(),
        );
        let mut report = CardSheet::new();
        let visuals = completed
            .iter()
            .copied()
            .map(|draft| self.cell(draft).cache().visual(visual_revision()))
            .collect::<Result<Vec<_>>>()?;
        let _guards = hold_visuals(visuals, VISUAL_LOCK_TIMEOUT)?;
        for draft in completed.iter().copied() {
            let entry = to_entry(draft)?;
            let cell = self.cell(draft);
            let cache = cell.cache();
            let visual = cache.visual(visual_revision())?;
            let voice = cell.media_name("wav");
            let image = cell.media_name("jpg");
            let voice_path = cache.filepath(VOICE_FILE)?;
            let image_path = visual.filepath(ILLUSTRATION_FILE)?;
            container.attach(voice_path, voice.as_str());
            container.attach(image_path.clone(), image.as_str());
            container.add(
                &entry,
                format!("[sound:{voice}]").as_str(),
                format!("<img src='{image}' style='{IMAGE_STYLE}'>").as_str(),
            );
            report.append(&entry, Some(image_path));
        }
        let stamp = release_stamp()?;
        let prefix = decknaming.prefix.to_uppercase();
        let apkg = self.output.join(format!("{prefix}_{stamp}.apkg"));
        container.save(&apkg)?;
        progress.advance(PublishPhase::Report);
        let pdf = self.output.join(format!("{prefix}_{stamp}.pdf"));
        report.save(&pdf, &Thumbnail::new(1024))?;
        Ok((
            apkg.to_string_lossy().into_owned(),
            pdf.to_string_lossy().into_owned(),
            self.output.to_string_lossy().into_owned(),
        ))
    }
}

use cost_accounting::{
    AccountingHealth, CostRecorder, SessionCostAttribution, attach_scene_cost, visual_costs,
};
use gemini_media::{GeminiRecall, MeteredGemini};
#[cfg(test)]
pub(in crate::cli) use picture_requests::reserve_picture_request;
pub(in crate::cli) use picture_requests::restart_picture_request_series;
use picture_requests::{RequestCountingImage, picture_request_total};
use visual_generation::{
    LocalImageRejection, NoopProgress, PictureRecovery, hold_visuals,
    render_recomposition_with_fallback, reserve_scene_attempt, scene_attempt_cursor,
};

/// Resolve the default directory for generated decks and reports.
pub(super) fn default_output() -> Result<PathBuf> {
    Locations::new(LocationArgs::default(), SystemContext).output()
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format_unit(bytes, 1024, "KB")
    } else {
        format_unit(bytes, 1024 * 1024, "MB")
    }
}

fn artifact_file(
    filename: String,
    path: PathBuf,
    cached: bool,
    cost: Option<GenerationCost>,
) -> ArtifactFile {
    let size = fs::metadata(&path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let file = ArtifactFile::new(filename, path, format_size(size), cached);
    match cost {
        Some(cost) => file.with_cost(cost),
        None => file,
    }
}

fn format_unit(bytes: u64, unit: u64, suffix: &str) -> String {
    let whole = bytes / unit;
    let tenth = bytes % unit * 10 / unit;
    format!("{whole}.{tenth} {suffix}")
}

fn release_stamp() -> Result<String> {
    Ok(OffsetDateTime::now_utc()
        .format(parse_time("[year]-[month]-[day]_[hour][minute][second]")?.as_slice())?)
}

fn production_renderer<C, J>(client: C, recall: J, border: BorderDetector) -> MangaRenderer<J>
where
    C: ImageSource + 'static,
    J: RecallJudge,
{
    MangaRenderer::new(client, IMAGE_ATTEMPTS_PER_ARTIFACT, recall, border)
}

#[cfg(test)]
mod tests;
