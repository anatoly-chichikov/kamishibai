//! Live implementation of the UI-neutral card workflow.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
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
    Cache, ILLUSTRATION_COST_FILE, ILLUSTRATION_FILE, IMAGE_ATTEMPTS_DIRECTORY, META_COST_FILE,
    PICTURE_REQUESTS_FILE, ROOT_STAGE_LOCK_TIMEOUT, RootStage, SCENE_ATTEMPT_FILE, SCENE_COST_FILE,
    SCENE_FILE, VISUAL_LOCK_TIMEOUT, VOICE_COST_FILE, VOICE_FILE, VisualGuard,
};
use crate::generation::manga::{
    BorderDetector, HiddenRecall, Illustration, ImageSource, MangaRenderRejection, MangaRenderer,
    Progress as SceneProgress, RecallCard, RecallJudge, RecallReview, ShownRecall,
};
use crate::generation::speech::Audio;
use crate::generation::{
    SceneComposer, SceneSource, Speaker, render_audio_prompt, visual_revision,
};
use crate::languages::{LanguageCatalog, catalog, naming};
use crate::report::{CardSheet, Thumbnail};
use crate::runtime::locations::{LocationArgs, Locations, SystemContext};
use crate::session::{
    ARTIFACT_ATTEMPT_CEILING, Artifact, ArtifactAttempt, ArtifactFile, BulkCorrection,
    CachedUnderstanding, CardCell, CardCorrection, CardDraft, CardMeta, CardMetaCache,
    CardMetaGeneration, CardRevision, CostRecord, GenerationCost, LanguagePair, RawInputBatch,
    Understanding, Understood, WordCandidate, to_entry,
};
use crate::vocabulary::VocabularyEntry;

const IMAGE_STYLE: &str = "max-width: 100%; height: auto; border-radius: 10px";
const IMAGE_ATTEMPTS_PER_ARTIFACT: usize = 1;

type LiveIllustration = Illustration<SceneComposer<MeteredGemini>, MangaRenderer<GeminiRecall>>;

/// Where the live generator looks for the Gemini API key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KeyLookup {
    /// Interactive flow: use the key validated and saved through Welcome.
    Saved,
    /// Console flow: `GEMINI_API_KEY` wins, falling back to the saved key.
    Environment,
}

/// Live card generator backed by Gemini and the on-disk cache.
#[derive(Clone)]
pub(super) struct LiveCardGenerator {
    cache: PathBuf,
    output: PathBuf,
    catalog: LanguageCatalog,
    state: LiveState,
}

#[derive(Clone, Debug)]
struct LiveState {
    keys: KeyLookup,
    pictures: PictureRecovery,
    costs: Option<SessionCostScope>,
    slot: Option<usize>,
}

impl LiveState {
    fn new(keys: KeyLookup) -> Self {
        Self {
            keys,
            pictures: PictureRecovery::default(),
            costs: None,
            slot: None,
        }
    }
}

impl LiveCardGenerator {
    /// Build a live card generator for the interactive flow (saved key only).
    pub(super) fn new(cache: PathBuf, output: PathBuf) -> Self {
        Self {
            cache,
            output,
            catalog: catalog(),
            state: LiveState::new(KeyLookup::Saved),
        }
    }

    /// Build a live card generator for the console flow, where `GEMINI_API_KEY`
    /// is the documented key source and wins over any saved preference.
    pub(super) fn for_console(cache: PathBuf, output: PathBuf) -> Self {
        Self {
            cache,
            output,
            catalog: catalog(),
            state: LiveState::new(KeyLookup::Environment),
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
    ) -> Result<LiveIllustration> {
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
            &LiveIllustration,
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

impl Understanding for LiveCardGenerator {
    fn understand(&self, raw: &RawInputBatch, my: &str) -> Result<Understood> {
        CachedUnderstanding::new(self.client()?, self.cache.clone()).understand(raw, my)
    }
}

impl BulkCorrection for LiveCardGenerator {
    fn correct_bulk(
        &self,
        candidate: &WordCandidate,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<crate::session::SenseCorrection> {
        self.client()?.correct_bulk(candidate, comment, pair)
    }
}

impl CardMetaGeneration for LiveCardGenerator {
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

impl CardCorrection for LiveCardGenerator {
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

impl KeyValidation for LiveCardGenerator {
    fn check_key(&self, key: &str) -> Result<()> {
        GeminiClient::new(key, HttpTransport::new()).validate_key()
    }
}

impl CardGeneration for LiveCardGenerator {
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

impl DeckPublishing for LiveCardGenerator {
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

fn hold_visuals(mut visuals: Vec<Cache>, timeout: Duration) -> Result<Vec<VisualGuard>> {
    visuals.sort_by_key(Cache::path);
    visuals.dedup_by(|left, right| left.path() == right.path());
    visuals
        .iter()
        .map(|visual| visual.hold_visual(timeout))
        .collect()
}

struct NoopProgress;

impl SceneProgress for NoopProgress {
    fn step(&mut self, _name: &str) {}

    fn done(&mut self, _name: &str, _label: &str, _path: Option<&Path>) {}
}

fn render_recomposition_with_fallback<T, P, R, F, E>(
    cache: &Cache,
    accounting: &AccountingHealth,
    progress: &mut P,
    recompose: R,
    fallback: F,
) -> std::result::Result<T, E>
where
    R: FnOnce(&mut P) -> std::result::Result<T, E>,
    F: FnOnce(&mut P) -> std::result::Result<T, E>,
    E: From<anyhow::Error>,
{
    let before = accounting
        .record(picture_request_total(cache))
        .map_err(E::from)?;
    let original = match recompose(progress) {
        Ok(rendered) => return Ok(rendered),
        Err(error) => error,
    };
    if accounting.failed() {
        return Err(original);
    }
    let after = accounting
        .record(picture_request_total(cache))
        .map_err(E::from)?;
    if after != before {
        return Err(original);
    }
    match fallback(progress) {
        Ok(rendered) => Ok(rendered),
        Err(error) if accounting.failed() => Err(error),
        Err(error)
            if accounting
                .record(picture_request_total(cache))
                .map_err(E::from)?
                != after =>
        {
            Err(error)
        }
        Err(_) => Err(original),
    }
}

#[derive(Clone, Debug, Default)]
struct PictureRecovery {
    states: Arc<Mutex<BTreeMap<PathBuf, PictureRecoveryState>>>,
}

impl PictureRecovery {
    fn prepare(&self, path: &Path, done: u8) -> Result<bool> {
        let persisted = persisted_local_rejections(path)?;
        let mut states = self
            .states
            .lock()
            .map_err(|_| anyhow!("picture recovery state lock is poisoned"))?;
        if done == 0 {
            states.insert(
                path.to_path_buf(),
                PictureRecoveryState {
                    observed_attempts: 0,
                    rejections: persisted,
                },
            );
        } else {
            let state = states
                .entry(path.to_path_buf())
                .or_insert_with(|| PictureRecoveryState {
                    observed_attempts: done,
                    rejections: persisted,
                });
            if state.observed_attempts != done {
                *state = PictureRecoveryState {
                    observed_attempts: done,
                    rejections: persisted,
                };
            } else {
                state.rejections = state.rejections.synchronized(persisted);
            }
        }
        Ok(states
            .get(path)
            .is_some_and(|state| state.rejections.recompose()))
    }

    fn observe(
        &self,
        path: &Path,
        done: u8,
        local_rejection: Option<LocalImageRejection>,
    ) -> Result<()> {
        let mut states = self
            .states
            .lock()
            .map_err(|_| anyhow!("picture recovery state lock is poisoned"))?;
        let state = states.entry(path.to_path_buf()).or_default();
        if state.observed_attempts != done {
            *state = PictureRecoveryState {
                observed_attempts: done,
                rejections: LocalImageRejections::default(),
            };
        }
        state.observed_attempts = done.saturating_add(1);
        if let Some(rejection) = local_rejection {
            state.rejections = state.rejections.pushed(rejection);
        }
        Ok(())
    }
}

fn persisted_local_rejections(path: &Path) -> Result<LocalImageRejections> {
    let directory = path.join(IMAGE_ATTEMPTS_DIRECTORY);
    if !directory.is_dir() {
        return Ok(LocalImageRejections::default());
    }
    let mut verdicts = fs::read_dir(directory)?
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let sequence = name
                .strip_prefix("attempt-")?
                .strip_suffix(".json")?
                .parse::<usize>()
                .ok()?;
            Some((sequence, entry.path()))
        })
        .collect::<Vec<_>>();
    verdicts.sort_by_key(|(sequence, _)| *sequence);
    verdicts.iter().try_fold(
        LocalImageRejections::default(),
        |rejections, (_, verdict)| {
            let value = fs::read(verdict)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
            let local_rejection = value.as_ref().and_then(LocalImageRejection::from_verdict);
            Ok(local_rejection.map_or(rejections, |rejection| rejections.pushed(rejection)))
        },
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LocalImageRejection {
    Color,
    Topology,
    RecallText,
    Border,
    LegacyGutter,
    Other,
}

impl LocalImageRejection {
    fn from_category(category: &str) -> Self {
        match category {
            "color" => Self::Color,
            "topology" => Self::Topology,
            "ocr" | "recall_text" => Self::RecallText,
            "border" => Self::Border,
            "legacy_gutter" => Self::LegacyGutter,
            _ => Self::Other,
        }
    }

    fn from_verdict(value: &serde_json::Value) -> Option<Self> {
        if value.get("status").and_then(serde_json::Value::as_str) != Some("rejected") {
            return None;
        }
        value
            .get("category")
            .and_then(serde_json::Value::as_str)
            .map(Self::from_category)
            .or_else(|| {
                matches!(
                    value.get("reason").and_then(serde_json::Value::as_str),
                    Some(
                        "Registered panel topology was not detected"
                            | "Unexpected internal gutter in one-panel layout"
                    )
                )
                .then_some(Self::Topology)
            })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct LocalImageRejections {
    recent: [Option<LocalImageRejection>; 2],
}

impl LocalImageRejections {
    fn pushed(self, rejection: LocalImageRejection) -> Self {
        Self {
            recent: [self.recent[1], Some(rejection)],
        }
    }

    fn recompose(&self) -> bool {
        matches!(
            self.recent,
            [Some(LocalImageRejection::Topology), Some(_)]
                | [Some(_), Some(LocalImageRejection::Topology)]
                | [
                    Some(LocalImageRejection::Border),
                    Some(LocalImageRejection::Border)
                ]
        )
    }

    fn synchronized(self, persisted: Self) -> Self {
        if persisted.len() > self.len() {
            persisted
        } else {
            self
        }
    }

    fn len(&self) -> usize {
        self.recent.iter().flatten().count()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SceneAttemptCursor {
    committed: Option<u8>,
    archived: Option<u8>,
    attempted: Option<u8>,
}

impl SceneAttemptCursor {
    fn has_rejected_recomposition(&self) -> bool {
        self.committed
            .zip(self.attempted)
            .is_some_and(|(committed, attempted)| attempted > committed)
    }

    fn recompose(&self, requested: bool) -> bool {
        let unrendered = self
            .committed
            .is_some_and(|committed| self.archived.is_none_or(|archived| committed > archived));
        self.has_rejected_recomposition() || requested && !unrendered
    }

    fn current(&self, fallback: u8) -> u8 {
        self.committed.or(self.attempted).unwrap_or(fallback)
    }

    fn next(&self, fallback: u8) -> Result<u8> {
        self.attempted.map_or(Ok(fallback), |attempted| {
            attempted
                .checked_add(1)
                .map(|next| next.max(fallback))
                .ok_or_else(|| anyhow!("scene attempt index overflow"))
        })
    }
}

fn scene_attempt_cursor(cache: &Cache, fallback: u8) -> Result<SceneAttemptCursor> {
    let committed = if cache.exists(SCENE_FILE) {
        let scene = serde_json::from_slice::<serde_json::Value>(
            fs::read(cache.path().join(SCENE_FILE))?.as_slice(),
        )?;
        scene_attempt_index(&scene)?.or(Some(fallback))
    } else {
        None
    };
    let reserved = load_scene_attempt(cache)?;
    let directory = cache.path().join(IMAGE_ATTEMPTS_DIRECTORY);
    let mut attempted = committed.into_iter().chain(reserved).max();
    let mut archived = None;
    if directory.is_dir() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(String::from) else {
                continue;
            };
            if !name.starts_with("attempt-") || !name.ends_with(".scene.json") {
                continue;
            }
            let Ok(bytes) = fs::read(entry.path()) else {
                continue;
            };
            let Ok(scene) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            if let Some(index) = scene_attempt_index(&scene)? {
                archived = archived.into_iter().chain(Some(index)).max();
            }
        }
    }
    attempted = attempted.into_iter().chain(archived).max();
    Ok(SceneAttemptCursor {
        committed,
        archived,
        attempted,
    })
}

fn reserve_scene_attempt(
    cache: &Cache,
    artifact: Artifact,
    fallback: u8,
    recompose: bool,
) -> Result<u8> {
    let cursor = scene_attempt_cursor(cache, fallback)?;
    let advance = match artifact {
        Artifact::Scene => !cache.exists(SCENE_FILE),
        Artifact::Picture => recompose && !cache.exists(ILLUSTRATION_FILE),
        Artifact::Meta | Artifact::Sound => {
            bail!("scene attempt reservation requires scene or picture")
        }
    };
    let selected = if advance {
        cursor.next(fallback)?
    } else {
        cursor.current(fallback)
    };
    store_scene_attempt(cache, selected)?;
    Ok(selected)
}

fn load_scene_attempt(cache: &Cache) -> Result<Option<u8>> {
    if !cache.exists(SCENE_ATTEMPT_FILE) {
        return Ok(None);
    }
    let value = serde_json::from_slice::<serde_json::Value>(
        fs::read(cache.path().join(SCENE_ATTEMPT_FILE))?.as_slice(),
    )?;
    value
        .get("scene_attempt_index")
        .and_then(serde_json::Value::as_u64)
        .map(u8::try_from)
        .transpose()
        .map_err(anyhow::Error::from)
}

fn store_scene_attempt(cache: &Cache, attempt: u8) -> Result<()> {
    if let Some(current) = load_scene_attempt(cache)? {
        if current > attempt {
            bail!("scene attempt cursor cannot move backwards from {current} to {attempt}");
        }
        if current == attempt {
            return Ok(());
        }
    }
    let staged = cache.stage(".scene-attempt.json")?;
    let result = serde_json::to_vec_pretty(&serde_json::json!({
        "scene_attempt_index": attempt
    }))
    .map_err(anyhow::Error::from)
    .and_then(|bytes| fs::write(&staged, bytes).map_err(anyhow::Error::from))
    .and_then(|()| cache.commit(&staged, SCENE_ATTEMPT_FILE));
    if result.is_err() {
        let _ = fs::remove_file(&staged);
    }
    result
}

fn scene_attempt_index(scene: &serde_json::Value) -> Result<Option<u8>> {
    scene
        .pointer("/manga_panel/meta/layout_selection/scene_attempt_index")
        .and_then(serde_json::Value::as_u64)
        .map(u8::try_from)
        .transpose()
        .map_err(anyhow::Error::from)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PictureRecoveryState {
    observed_attempts: u8,
    rejections: LocalImageRejections,
}

#[derive(Clone, Debug)]
struct SessionCostAttribution {
    scope: SessionCostScope,
    slot: usize,
}

impl SessionCostAttribution {
    fn new(scope: SessionCostScope, slot: usize) -> Self {
        Self { scope, slot }
    }

    fn charge(&self, artifact: Artifact, delta: GenerationCost) -> Result<()> {
        self.scope.charge(self.slot, artifact, delta).map(|_| ())
    }
}

#[derive(Clone, Debug)]
struct AccountingHealth {
    failed: Rc<std::cell::Cell<bool>>,
}

impl AccountingHealth {
    fn new(failed: Rc<std::cell::Cell<bool>>) -> Self {
        Self { failed }
    }

    fn record<T>(&self, result: Result<T>) -> Result<T> {
        if result.is_err() {
            self.failed.set(true);
        }
        result
    }

    fn failed(&self) -> bool {
        self.failed.get()
    }
}

impl Default for AccountingHealth {
    fn default() -> Self {
        Self::new(Rc::new(std::cell::Cell::new(false)))
    }
}

#[derive(Clone, Debug)]
struct CostState {
    observed: Rc<RefCell<Option<CostRecord>>>,
    accounting: AccountingHealth,
}

impl CostState {
    fn new(accounting: AccountingHealth) -> Self {
        Self {
            observed: Rc::new(RefCell::new(None)),
            accounting,
        }
    }
}

#[derive(Clone, Debug)]
struct CostRecorder {
    cache: Cache,
    artifact: Artifact,
    state: CostState,
    session: Option<SessionCostAttribution>,
}

impl CostRecorder {
    #[cfg(test)]
    fn new(cache: Cache, artifact: Artifact) -> Self {
        Self::attributed(cache, artifact, None)
    }

    #[cfg(test)]
    fn attributed(
        cache: Cache,
        artifact: Artifact,
        session: Option<SessionCostAttribution>,
    ) -> Self {
        Self::guarded(cache, artifact, session, AccountingHealth::default())
    }

    fn guarded(
        cache: Cache,
        artifact: Artifact,
        session: Option<SessionCostAttribution>,
        accounting: AccountingHealth,
    ) -> Self {
        Self {
            cache,
            artifact,
            state: CostState::new(accounting),
            session,
        }
    }

    fn push(&self, record: CostRecord) -> Result<()> {
        if record.requests() == 0 {
            return Ok(());
        }
        let result = self
            .observe(&record)
            .and_then(|()| store_cost(&self.cache, self.artifact, &record).map(|_| ()));
        self.state.accounting.record(result)
    }

    fn push_correction(&self, record: CostRecord) -> Result<()> {
        if record.requests() == 0 {
            return Ok(());
        }
        let result = self
            .observe(&record)
            .and_then(|()| persist_correction_cost(&self.cache, &record));
        self.state.accounting.record(result)
    }

    fn observe(&self, record: &CostRecord) -> Result<()> {
        if let Some(session) = self.session.as_ref() {
            session.charge(self.artifact, record.cost())?;
        }
        let aggregate = self
            .state
            .observed
            .borrow()
            .as_ref()
            .map(|current| current.merged(record))
            .unwrap_or_else(|| record.clone());
        *self.state.observed.borrow_mut() = Some(aggregate);
        Ok(())
    }

    fn current(&self, cached: bool) -> Result<Option<GenerationCost>> {
        if cached {
            return Ok(None);
        }
        Ok(self.state.observed.borrow().as_ref().map(CostRecord::cost))
    }

    fn cumulative(&self, cached: bool) -> Result<Option<GenerationCost>> {
        self.current(cached)
    }
}

fn picture_request_total(cache: &Cache) -> Result<u32> {
    load_picture_request_counter(cache).map(|counter| counter.requests)
}

const PICTURE_REQUEST_COUNTER_SCHEMA: &str = "kamishibai.picture-request-counter";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct PictureRequestCounter {
    schema: String,
    version: u8,
    requests: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    series_requests: Option<u32>,
}

impl PictureRequestCounter {
    fn new(requests: u32, series_requests: u32) -> Self {
        Self {
            schema: String::from(PICTURE_REQUEST_COUNTER_SCHEMA),
            version: 1,
            requests,
            series_requests: Some(series_requests),
        }
    }

    fn reserved(&self) -> Result<Self> {
        let series = self.series_requests.unwrap_or(self.requests);
        let ceiling = u32::from(ARTIFACT_ATTEMPT_CEILING);
        if series >= ceiling {
            bail!("picture request series exhausted its {ceiling}-attempt ceiling");
        }
        Ok(Self::new(
            self.requests
                .checked_add(1)
                .ok_or_else(|| anyhow!("picture request counter overflow"))?,
            series
                .checked_add(1)
                .ok_or_else(|| anyhow!("picture request series counter overflow"))?,
        ))
    }

    fn restarted(&self) -> Self {
        Self::new(self.requests, 0)
    }

    fn validated(self) -> Result<Self> {
        if self.schema != PICTURE_REQUEST_COUNTER_SCHEMA || self.version != 1 {
            bail!("picture request counter has an unsupported schema");
        }
        if self.series_requests.unwrap_or(self.requests) > self.requests {
            bail!("picture request series exceeds its total request count");
        }
        Ok(self)
    }
}

fn load_picture_request_counter(cache: &Cache) -> Result<PictureRequestCounter> {
    if !cache.exists(PICTURE_REQUESTS_FILE) {
        return Ok(PictureRequestCounter::new(0, 0));
    }
    let path = cache.filepath(PICTURE_REQUESTS_FILE)?;
    serde_json::from_slice::<PictureRequestCounter>(fs::read(path)?.as_slice())?.validated()
}

fn store_picture_request_counter(cache: &Cache, counter: &PictureRequestCounter) -> Result<()> {
    let staged = cache.stage(".requests.json")?;
    let result = serde_json::to_vec_pretty(counter)
        .map_err(anyhow::Error::from)
        .and_then(|json| fs::write(&staged, json).map_err(anyhow::Error::from))
        .and_then(|()| cache.commit(&staged, PICTURE_REQUESTS_FILE));
    if result.is_err() {
        let _ = fs::remove_file(&staged);
    }
    result
}

pub(in crate::cli) fn reserve_picture_request(cache: &Cache) -> Result<()> {
    let counter = load_picture_request_counter(cache)?.reserved()?;
    store_picture_request_counter(cache, &counter)
}

pub(in crate::cli) fn restart_picture_request_series(cache: &Cache) -> Result<()> {
    if cache.exists(PICTURE_REQUESTS_FILE) {
        store_picture_request_counter(cache, &load_picture_request_counter(cache)?.restarted())?;
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct RequestCountingImage<C> {
    client: C,
    cache: Cache,
    accounting: AccountingHealth,
}

impl<C> RequestCountingImage<C> {
    #[cfg(test)]
    fn new(client: C, cache: Cache) -> Self {
        Self::guarded(client, cache, AccountingHealth::default())
    }

    fn guarded(client: C, cache: Cache, accounting: AccountingHealth) -> Self {
        Self {
            client,
            cache,
            accounting,
        }
    }
}

impl<C> ImageSource for RequestCountingImage<C>
where
    C: ImageSource,
{
    fn image(&self, prompt: &str) -> Result<Vec<u8>> {
        self.accounting
            .record(reserve_picture_request(&self.cache))?;
        self.client.image(prompt)
    }
}

#[derive(Clone, Debug)]
struct MeteredGemini {
    client: GeminiClient<HttpTransport>,
    costs: CostRecorder,
}

impl MeteredGemini {
    fn new(client: GeminiClient<HttpTransport>, costs: CostRecorder) -> Self {
        Self { client, costs }
    }
}

impl SceneSource for MeteredGemini {
    fn scene(
        &self,
        language: &str,
        term: &str,
        sentence: &str,
        target: &str,
        attempt: u8,
    ) -> Result<serde_json::Value> {
        self.client
            .scene_observed(language, term, sentence, target, attempt, |cost| {
                self.costs.push(cost)
            })
    }
}

impl ImageSource for MeteredGemini {
    fn image(&self, prompt: &str) -> Result<Vec<u8>> {
        self.client
            .image_observed(prompt, |cost| self.costs.push(cost))
    }
}

impl Speaker for MeteredGemini {
    fn speech(&self, prompt: &str, text: &str) -> Result<Vec<u8>> {
        self.client
            .speech_observed(prompt, text, |cost| self.costs.push(cost))
    }
}

#[derive(Clone, Debug)]
struct GeminiRecall {
    client: GeminiClient<HttpTransport>,
    card: RecallCard,
    costs: CostRecorder,
}

impl GeminiRecall {
    fn new(client: GeminiClient<HttpTransport>, card: RecallCard, costs: CostRecorder) -> Self {
        Self {
            client,
            card,
            costs,
        }
    }
}

impl RecallJudge for GeminiRecall {
    fn review(&self, image: &[u8]) -> Result<RecallReview> {
        self.client
            .review_recall_observed(&self.card, image_mime(image)?, image, |cost| {
                self.costs.push(cost)
            })
    }
}

fn image_mime(image: &[u8]) -> Result<&'static str> {
    match image::guess_format(image)? {
        image::ImageFormat::Jpeg => Ok("image/jpeg"),
        image::ImageFormat::Png => Ok("image/png"),
        image::ImageFormat::WebP => Ok("image/webp"),
        image::ImageFormat::Gif => Ok("image/gif"),
        format => bail!("unsupported recall-review image format {format:?}"),
    }
}

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

#[cfg(test)]
fn load_cost(cache: &Cache, artifact: Artifact) -> Result<Option<GenerationCost>> {
    Ok(load_cost_record(cache, artifact)?.map(|record| record.cost()))
}

fn load_cost_record(cache: &Cache, artifact: Artifact) -> Result<Option<CostRecord>> {
    let filename = cost_filename(artifact);
    if !cache.exists(filename) {
        return Ok(None);
    }
    let path = cache.filepath(filename)?;
    let text = fs::read_to_string(path)?;
    Ok(Some(serde_json::from_str::<CostRecord>(text.as_str())?))
}

fn store_cost(cache: &Cache, artifact: Artifact, record: &CostRecord) -> Result<CostRecord> {
    if record.requests() == 0 {
        return Ok(load_cost_record(cache, artifact)?.unwrap_or_else(|| record.clone()));
    }
    let merged = load_cost_record(cache, artifact)?
        .map(|existing| existing.merged(record))
        .unwrap_or_else(|| record.clone());
    let staged = cache.stage(".cost.json")?;
    let result = serde_json::to_string_pretty(&merged)
        .map_err(anyhow::Error::from)
        .and_then(|json| fs::write(&staged, json).map_err(anyhow::Error::from))
        .and_then(|()| cache.commit(&staged, cost_filename(artifact)));
    if result.is_err() {
        let _ = fs::remove_file(&staged);
    }
    result?;
    Ok(merged)
}

fn persist_correction_cost(cache: &Cache, record: &CostRecord) -> Result<()> {
    let _guard = cache.hold_root_stage(RootStage::Meta, ROOT_STAGE_LOCK_TIMEOUT)?;
    store_cost(cache, Artifact::Meta, record)?;
    Ok(())
}

fn visual_costs(
    artifact: Artifact,
    cached: bool,
    scene: &CostRecorder,
    picture: &CostRecorder,
) -> Result<(Option<GenerationCost>, Option<GenerationCost>)> {
    match artifact {
        Artifact::Scene => Ok((scene.cumulative(cached)?, None)),
        Artifact::Picture => Ok((picture.cumulative(cached)?, scene.current(cached)?)),
        Artifact::Meta | Artifact::Sound => {
            panic!("visual cost settlement requires scene or picture")
        }
    }
}

fn attach_scene_cost<T>(
    attempt: ArtifactAttempt<T>,
    artifact: Artifact,
    scene: Option<GenerationCost>,
) -> ArtifactAttempt<T> {
    match (artifact, scene) {
        (Artifact::Picture, Some(cost)) => attempt.with_related_cost(Artifact::Scene, cost),
        (_, _) => attempt,
    }
}

fn cost_filename(artifact: Artifact) -> &'static str {
    match artifact {
        Artifact::Meta => META_COST_FILE,
        Artifact::Scene => SCENE_COST_FILE,
        Artifact::Picture => ILLUSTRATION_COST_FILE,
        Artifact::Sound => VOICE_COST_FILE,
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
mod tests {
    use std::cell::Cell;
    use std::io::Cursor;
    use std::process::{Command, Stdio};
    use std::time::Instant;

    use image::{DynamicImage, GrayImage, ImageFormat, Luma};
    use serde_json::Value;
    use tempfile::TempDir;

    use super::*;
    use crate::generation::manga::{Renderer, Translator};
    use crate::session::ArtifactCosts;

    #[derive(Clone, Debug)]
    struct CountingImageSource {
        calls: Rc<Cell<usize>>,
        image: Vec<u8>,
    }

    impl CountingImageSource {
        fn new(image: Vec<u8>) -> Self {
            Self {
                calls: Rc::new(Cell::new(0)),
                image,
            }
        }

        fn calls(&self) -> usize {
            self.calls.get()
        }
    }

    #[derive(Clone, Debug)]
    struct FailingImageSource {
        calls: Rc<Cell<usize>>,
        error: &'static str,
    }

    impl FailingImageSource {
        fn new(error: &'static str) -> Self {
            Self {
                calls: Rc::new(Cell::new(0)),
                error,
            }
        }

        fn calls(&self) -> usize {
            self.calls.get()
        }
    }

    impl ImageSource for FailingImageSource {
        fn image(&self, _prompt: &str) -> Result<Vec<u8>> {
            self.calls.set(self.calls.get() + 1);
            bail!(self.error)
        }
    }

    #[derive(Clone, Debug)]
    struct UsageFreeImageSource {
        costs: CostRecorder,
        image: Vec<u8>,
    }

    impl UsageFreeImageSource {
        fn new(costs: CostRecorder, image: Vec<u8>) -> Self {
            Self { costs, image }
        }
    }

    impl ImageSource for UsageFreeImageSource {
        fn image(&self, _prompt: &str) -> Result<Vec<u8>> {
            self.costs.push(CostRecord::new(
                "gemini-3.1-flash-image",
                0,
                0,
                0,
                0,
                GenerationCost::zero(),
            ))?;
            Ok(self.image.clone())
        }
    }

    #[derive(Clone, Debug)]
    struct PaidImageSource {
        costs: CostRecorder,
        image: Vec<u8>,
    }

    impl ImageSource for PaidImageSource {
        fn image(&self, _prompt: &str) -> Result<Vec<u8>> {
            self.costs.push(CostRecord::new(
                "gemini-3.1-flash-image",
                1,
                40,
                10,
                50,
                GenerationCost::from_nanos(900_000),
            ))?;
            Ok(self.image.clone())
        }
    }

    impl ImageSource for CountingImageSource {
        fn image(&self, _prompt: &str) -> Result<Vec<u8>> {
            self.calls.set(self.calls.get() + 1);
            Ok(self.image.clone())
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct RejectingRecall;

    impl RecallJudge for RejectingRecall {
        fn review(&self, _image: &[u8]) -> Result<RecallReview> {
            Ok(serde_json::from_value(serde_json::json!({
                "decision": "REJECT",
                "evidence": [{
                    "reading": "ANSWER",
                    "location": "center",
                    "kind": "FOCUS"
                }],
                "reason": "The focus answer is visible"
            }))?)
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct AcceptingRecall;

    impl RecallJudge for AcceptingRecall {
        fn review(&self, _image: &[u8]) -> Result<RecallReview> {
            Ok(serde_json::from_value(serde_json::json!({
                "decision": "ALLOW",
                "evidence": [],
                "reason": "No answer-bearing writing is visible"
            }))?)
        }
    }

    #[derive(Clone, Debug)]
    struct PaidRecall {
        costs: CostRecorder,
    }

    impl RecallJudge for PaidRecall {
        fn review(&self, _image: &[u8]) -> Result<RecallReview> {
            self.costs.push(CostRecord::new(
                "gemini-3.5-flash-lite",
                1,
                400,
                25,
                425,
                GenerationCost::from_nanos(50_000),
            ))?;
            AcceptingRecall.review(&[])
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct FailingTranslator;

    impl Translator for FailingTranslator {
        fn translate(&self, _sentence: &str, _target: &str) -> Result<Value> {
            bail!("scene composition failed")
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
    enum RecoveryFailure {
        #[error("recomposition failed")]
        Recomposition,
        #[error("fallback failed")]
        Fallback,
        #[error("image provider failed")]
        Provider,
        #[error("accounting failed")]
        Accounting,
    }

    impl From<anyhow::Error> for RecoveryFailure {
        fn from(_error: anyhow::Error) -> Self {
            Self::Accounting
        }
    }

    fn image_bytes(image: GrayImage) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageLuma8(image)
            .write_to(&mut bytes, ImageFormat::Png)
            .expect("test image must encode");
        bytes.into_inner()
    }

    fn valid_image() -> Vec<u8> {
        let mut image = GrayImage::from_pixel(32, 32, Luma([255]));
        for y in 2..30 {
            for x in 2..30 {
                image.put_pixel(x, y, Luma([0]));
            }
        }
        image_bytes(image)
    }

    fn renderable_scene() -> Value {
        serde_json::json!({
            "manga_panel": {
                "canvas": {
                    "width": 1024,
                    "height": 1024
                },
                "panel_layout": {
                    "active_layout": {
                        "template_id": "splash-1-v1"
                    }
                },
                "page_design": {
                    "special_device": {
                        "kind": "none"
                    }
                },
                "panels": [{
                    "id": "p1",
                    "bounds": {"x": 16, "y": 16, "width": 992, "height": 992},
                    "scene": {
                        "description": "One grounded subject performs a visible action",
                        "camera": {
                            "shot_scale": "medium",
                            "viewpoint": "objective",
                            "angle": "eye_level",
                            "depth_plan": "layered"
                        },
                        "lighting": "controlled high-value contrast"
                    }
                }]
            }
        })
    }

    fn picture_requests(cache: &Cache) -> u32 {
        load_picture_request_counter(cache)
            .expect("picture request counter must decode")
            .requests
    }

    fn picture_series_requests(cache: &Cache) -> u32 {
        let counter =
            load_picture_request_counter(cache).expect("picture request counter must decode");
        counter.series_requests.unwrap_or(counter.requests)
    }

    #[derive(Clone, Debug)]
    struct PersistingSpeaker {
        costs: CostRecorder,
    }

    impl Speaker for PersistingSpeaker {
        fn speech(&self, _prompt: &str, _text: &str) -> Result<Vec<u8>> {
            self.costs.push(CostRecord::new(
                "gemini-2.5-flash-preview-tts",
                1,
                20,
                40,
                60,
                GenerationCost::from_nanos(700_000),
            ))?;
            Ok(vec![0, 0])
        }
    }

    #[test]
    fn production_renderer_spends_one_image_call_per_artifact_attempt() {
        let source =
            CountingImageSource::new(image_bytes(GrayImage::from_pixel(16, 16, Luma([0]))));
        let renderer = production_renderer(
            source.clone(),
            RejectingRecall,
            BorderDetector::new(2, 6, 240, 2),
        );
        let result = renderer.render(&renderable_scene(), &mut NoopProgress);
        assert_eq!(
            (result.is_err(), source.calls()),
            (true, 1),
            "one outer artifact attempt multiplied into multiple image calls"
        );
    }

    #[test]
    fn pre_provider_scene_cache_and_recomposition_failures_spend_no_picture_request() {
        let home = TempDir::new().expect("tempdir must be created");
        let scene_cache = Cache::new("scene", home.path());
        let cache_cache = Cache::new("cache", home.path());
        let recompose_cache = Cache::new("recompose", home.path());
        let scene_source = CountingImageSource::new(valid_image());
        let cache_source = CountingImageSource::new(valid_image());
        let recompose_source = CountingImageSource::new(valid_image());
        let scene = Illustration::new(
            scene_cache.clone(),
            FailingTranslator,
            MangaRenderer::new(
                RequestCountingImage::new(scene_source.clone(), scene_cache.clone()),
                1,
                AcceptingRecall,
                BorderDetector::new(2, 6, 240, 2),
            ),
        );
        let cached = Illustration::new(
            cache_cache.clone(),
            FailingTranslator,
            MangaRenderer::new(
                RequestCountingImage::new(cache_source.clone(), cache_cache.clone()),
                1,
                AcceptingRecall,
                BorderDetector::new(2, 6, 240, 2),
            ),
        );
        let recompose = Illustration::new(
            recompose_cache.clone(),
            FailingTranslator,
            MangaRenderer::new(
                RequestCountingImage::new(recompose_source.clone(), recompose_cache.clone()),
                1,
                AcceptingRecall,
                BorderDetector::new(2, 6, 240, 2),
            ),
        );
        let scene_result = scene.scene_only("sentence", "en", &mut NoopProgress);
        let cache_result = cached.picture_only("sentence", "en", &mut NoopProgress);
        let recompose_result =
            recompose.picture_with_recomposed_scene("sentence", "en", &mut NoopProgress);
        assert_eq!(
            (
                scene_result.is_err(),
                cache_result.is_err(),
                recompose_result.is_err(),
                scene_source.calls(),
                cache_source.calls(),
                recompose_source.calls(),
                picture_requests(&scene_cache),
                picture_requests(&cache_cache),
                picture_requests(&recompose_cache),
            ),
            (true, true, true, 0, 0, 0, 0, 0, 0),
            "a pre-provider failure consumed or recorded an image request"
        );
    }

    #[test]
    fn pre_provider_recomposition_failure_falls_back_once_to_the_committed_scene() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("visual", home.path());
        let accounting = AccountingHealth::default();
        let fallback = Cell::new(0_u8);
        let mut progress = ();
        let result: std::result::Result<&str, RecoveryFailure> = render_recomposition_with_fallback(
            &cache,
            &accounting,
            &mut progress,
            |_| Err(RecoveryFailure::Recomposition),
            |_| {
                fallback.set(fallback.get().saturating_add(1));
                reserve_picture_request(&cache)?;
                Ok("committed")
            },
        );
        assert_eq!(
            (result.ok(), fallback.get(), picture_requests(&cache)),
            (Some("committed"), 1, 1),
            "pre-provider recomposition failure did not produce exactly one committed-scene image"
        );
    }

    #[test]
    fn recomposition_image_failure_never_falls_back_to_the_committed_scene() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("visual", home.path());
        let accounting = AccountingHealth::default();
        let fallback = Cell::new(0_u8);
        let mut progress = ();
        let result: std::result::Result<&str, RecoveryFailure> = render_recomposition_with_fallback(
            &cache,
            &accounting,
            &mut progress,
            |_| {
                reserve_picture_request(&cache)?;
                Err(RecoveryFailure::Provider)
            },
            |_| {
                fallback.set(fallback.get().saturating_add(1));
                Ok("committed")
            },
        );
        assert_eq!(
            (result.err(), fallback.get(), picture_requests(&cache),),
            (Some(RecoveryFailure::Provider), 0, 1),
            "an image failure triggered an extra fallback provider call"
        );
    }

    #[test]
    fn fallback_failure_before_the_provider_returns_the_recomposition_error() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("visual", home.path());
        let accounting = AccountingHealth::default();
        let fallback = Cell::new(0_u8);
        let mut progress = ();
        let result: std::result::Result<&str, RecoveryFailure> = render_recomposition_with_fallback(
            &cache,
            &accounting,
            &mut progress,
            |_| Err(RecoveryFailure::Recomposition),
            |_| {
                fallback.set(fallback.get().saturating_add(1));
                Err(RecoveryFailure::Fallback)
            },
        );
        assert_eq!(
            (result.err(), fallback.get(), picture_requests(&cache)),
            (Some(RecoveryFailure::Recomposition), 1, 0),
            "a pre-provider fallback failure hid the original recomposition diagnosis"
        );
    }

    #[test]
    fn fallback_image_failure_returns_the_fallback_provider_error() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("visual", home.path());
        let accounting = AccountingHealth::default();
        let mut progress = ();
        let result: std::result::Result<&str, RecoveryFailure> = render_recomposition_with_fallback(
            &cache,
            &accounting,
            &mut progress,
            |_| Err(RecoveryFailure::Recomposition),
            |_| {
                reserve_picture_request(&cache)?;
                Err(RecoveryFailure::Provider)
            },
        );
        assert_eq!(
            (result.err(), picture_requests(&cache)),
            (Some(RecoveryFailure::Provider), 1),
            "a fallback image failure was replaced by a stale scene diagnosis"
        );
    }

    #[test]
    fn cost_recording_failure_prevents_committed_scene_fallback() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("visual", home.path());
        let accounting = AccountingHealth::default();
        let costs = CostRecorder::guarded(
            Cache::failing("costs", home.path(), 0),
            Artifact::Scene,
            None,
            accounting.clone(),
        );
        let fallback = Cell::new(0_u8);
        let mut progress = ();
        let result: Result<&str> = render_recomposition_with_fallback(
            &cache,
            &accounting,
            &mut progress,
            |_| {
                costs
                    .push(CostRecord::new(
                        "gemini-3.6-flash",
                        1,
                        100,
                        20,
                        120,
                        GenerationCost::from_nanos(300_000),
                    ))
                    .map_err(|error| error.context("scene composition request failed"))?;
                Ok("recomposed")
            },
            |_| {
                fallback.set(fallback.get().saturating_add(1));
                Ok("committed")
            },
        );
        assert_eq!(
            (
                result.is_err(),
                fallback.get(),
                picture_requests(&cache),
                accounting.failed(),
            ),
            (true, 0, 0, true),
            "durable cost failure was hidden by a committed-scene fallback"
        );
    }

    #[test]
    fn request_recording_failure_prevents_committed_scene_fallback() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::failing("visual", home.path(), 0);
        let accounting = AccountingHealth::default();
        let fallback = Cell::new(0_u8);
        let source = CountingImageSource::new(valid_image());
        let image =
            RequestCountingImage::guarded(source.clone(), cache.clone(), accounting.clone());
        let mut progress = ();
        let result: Result<&str> = render_recomposition_with_fallback(
            &cache,
            &accounting,
            &mut progress,
            |_| {
                image.image("compiled image prompt")?;
                Ok("recomposed")
            },
            |_| {
                fallback.set(fallback.get().saturating_add(1));
                Ok("committed")
            },
        );
        assert_eq!(
            (
                result.is_err(),
                source.calls(),
                fallback.get(),
                picture_requests(&cache),
                accounting.failed(),
            ),
            (true, 0, 0, 0, true),
            "durable picture reservation failure was hidden by a committed-scene fallback"
        );
    }

    #[test]
    fn transport_failure_spends_one_picture_request() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("visual", home.path());
        let source = FailingImageSource::new("transport failed");
        let image = RequestCountingImage::new(source.clone(), cache.clone());
        let result = image.image("compiled image prompt");
        assert_eq!(
            (result.is_err(), source.calls(), picture_requests(&cache)),
            (true, 1, 1),
            "a transport failure was not counted exactly once"
        );
    }

    #[test]
    fn non_success_response_spends_one_picture_request() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("visual", home.path());
        let source = FailingImageSource::new("INVALID_ARGUMENT: request rejected");
        let image = RequestCountingImage::new(source.clone(), cache.clone());
        let result = image.image("compiled image prompt");
        assert_eq!(
            (result.is_err(), source.calls(), picture_requests(&cache)),
            (true, 1, 1),
            "a non-success provider response was not counted exactly once"
        );
    }

    #[test]
    fn successful_response_without_usage_spends_one_picture_request() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("visual", home.path());
        let costs = CostRecorder::new(cache.clone(), Artifact::Picture);
        let source = UsageFreeImageSource::new(costs, valid_image());
        let image = RequestCountingImage::new(source, cache.clone());
        let result = image.image("compiled image prompt");
        assert_eq!(
            (
                result.is_ok(),
                picture_requests(&cache),
                cache.exists(ILLUSTRATION_COST_FILE),
            ),
            (true, 1, false),
            "missing usage metadata erased the provider request or invented a cost record"
        );
    }

    #[test]
    fn undecodable_response_spends_one_picture_request() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("visual", home.path());
        let source = CountingImageSource::new(b"not an image".to_vec());
        let renderer = MangaRenderer::new(
            RequestCountingImage::new(source.clone(), cache.clone()),
            1,
            AcceptingRecall,
            BorderDetector::new(2, 6, 240, 2),
        );
        let result = renderer.render(&renderable_scene(), &mut NoopProgress);
        assert_eq!(
            (result.is_err(), source.calls(), picture_requests(&cache)),
            (true, 1, 1),
            "an undecodable response was not counted exactly once"
        );
    }

    #[test]
    fn validation_rejection_spends_one_picture_request() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("visual", home.path());
        let source = CountingImageSource::new(valid_image());
        let renderer = MangaRenderer::new(
            RequestCountingImage::new(source.clone(), cache.clone()),
            1,
            RejectingRecall,
            BorderDetector::new(2, 6, 240, 2),
        );
        let result = renderer.render(&renderable_scene(), &mut NoopProgress);
        assert_eq!(
            (result.is_err(), source.calls(), picture_requests(&cache)),
            (true, 1, 1),
            "a rejected image response was not counted exactly once"
        );
    }

    #[test]
    fn accepted_response_spends_one_picture_request() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("visual", home.path());
        let source = CountingImageSource::new(valid_image());
        let renderer = MangaRenderer::new(
            RequestCountingImage::new(source.clone(), cache.clone()),
            1,
            AcceptingRecall,
            BorderDetector::new(2, 6, 240, 2),
        );
        let result = renderer.render(&renderable_scene(), &mut NoopProgress);
        assert_eq!(
            (result.is_ok(), source.calls(), picture_requests(&cache)),
            (true, 1, 1),
            "an accepted image response was not counted exactly once"
        );
    }

    #[test]
    fn picture_cost_includes_recall_review_without_inflating_image_request_count() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("visual", home.path());
        let costs = CostRecorder::new(cache.clone(), Artifact::Picture);
        let source = PaidImageSource {
            costs: costs.clone(),
            image: valid_image(),
        };
        let renderer = MangaRenderer::new(
            RequestCountingImage::new(source, cache.clone()),
            1,
            PaidRecall {
                costs: costs.clone(),
            },
            BorderDetector::new(2, 6, 240, 2),
        );
        let result = renderer.render(&renderable_scene(), &mut NoopProgress);
        let record = load_cost_record(&cache, Artifact::Picture)
            .expect("picture cost must decode")
            .expect("picture cost must exist");
        assert_eq!(
            (
                result.is_ok(),
                picture_requests(&cache),
                record.requests(),
                record.model().to_string(),
                record.cost(),
            ),
            (
                true,
                1,
                2,
                String::from("gemini-3.1-flash-image,gemini-3.5-flash-lite"),
                GenerationCost::from_nanos(950_000),
            ),
            "recall review cost was hidden from Picture or counted as another image generation"
        );
    }

    #[test]
    fn picture_request_ceiling_survives_a_fresh_generator_instance() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("visual", home.path());
        let source = FailingImageSource::new("transport failed");
        let first = RequestCountingImage::new(source.clone(), cache.clone());
        for _ in 0..3 {
            let _ = first.image("compiled image prompt");
        }
        let restarted = RequestCountingImage::new(source.clone(), cache.clone());
        let fourth = restarted.image("compiled image prompt");
        assert_eq!(
            (
                fourth.is_err(),
                source.calls(),
                picture_requests(&cache),
                picture_series_requests(&cache),
            ),
            (true, 3, 3, 3),
            "a fresh generator instance expanded the durable picture ceiling"
        );
    }

    #[test]
    fn legacy_picture_counter_defaults_to_one_unfinished_series() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("visual", home.path());
        fs::write(
            cache
                .filepath(PICTURE_REQUESTS_FILE)
                .expect("counter path must resolve"),
            br#"{"schema":"kamishibai.picture-request-counter","version":1,"requests":3}"#,
        )
        .expect("legacy counter must be written");
        let source = CountingImageSource::new(valid_image());
        let image = RequestCountingImage::new(source.clone(), cache.clone());
        let result = image.image("compiled image prompt");
        assert_eq!(
            (
                result.is_err(),
                source.calls(),
                picture_requests(&cache),
                picture_series_requests(&cache),
            ),
            (true, 0, 3, 3),
            "a legacy counter silently opened an unauthorized picture series"
        );
    }

    #[test]
    fn picture_counter_write_failure_prevents_the_provider_call() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::failing("visual", home.path(), 0);
        let source = CountingImageSource::new(valid_image());
        let image = RequestCountingImage::new(source.clone(), cache.clone());
        let result = image.image("compiled image prompt");
        assert_eq!(
            (
                result.is_err(),
                source.calls(),
                picture_requests(&cache),
                cache.exists(PICTURE_REQUESTS_FILE),
            ),
            (true, 0, 0, false),
            "the provider was called after its durable request reservation failed"
        );
    }

    #[test]
    fn two_recall_text_rejections_keep_the_third_attempt_on_the_current_scene() {
        let recovery = PictureRecovery::default();
        let path = Path::new("cards/local-rejections");
        let first = recovery
            .prepare(path, 0)
            .expect("first attempt must prepare");
        recovery
            .observe(path, 0, Some(LocalImageRejection::RecallText))
            .expect("first local rejection must record");
        let second = recovery
            .prepare(path, 1)
            .expect("second attempt must prepare");
        recovery
            .observe(path, 1, Some(LocalImageRejection::RecallText))
            .expect("second local rejection must record");
        let third = recovery
            .prepare(path, 2)
            .expect("third attempt must prepare");
        assert_eq!(
            (first, second, third),
            (false, false, false),
            "recall-text failures discarded a scene that could still render without text"
        );
    }

    #[test]
    fn two_border_rejections_enable_third_attempt_recomposition() {
        let recovery = PictureRecovery::default();
        let path = Path::new("cards/repeated-border");
        recovery
            .prepare(path, 0)
            .expect("first attempt must prepare");
        recovery
            .observe(path, 0, Some(LocalImageRejection::Border))
            .expect("first border rejection must record");
        recovery.prepare(path, 1).expect("retry must prepare");
        recovery
            .observe(path, 1, Some(LocalImageRejection::Border))
            .expect("second border rejection must record");
        assert!(
            recovery
                .prepare(path, 2)
                .expect("third attempt must prepare"),
            "repeated border failures did not advance the third picture to a fresh scene"
        );
    }

    #[test]
    fn two_color_rejections_keep_the_third_attempt_on_the_current_scene() {
        let recovery = PictureRecovery::default();
        let path = Path::new("cards/repeated-color");
        recovery
            .prepare(path, 0)
            .expect("first attempt must prepare");
        recovery
            .observe(path, 0, Some(LocalImageRejection::Color))
            .expect("first color rejection must record");
        recovery.prepare(path, 1).expect("retry must prepare");
        recovery
            .observe(path, 1, Some(LocalImageRejection::Color))
            .expect("second color rejection must record");
        assert!(
            !recovery
                .prepare(path, 2)
                .expect("third attempt must prepare"),
            "repeated color failures discarded a scene whose composition was not implicated"
        );
    }

    #[test]
    fn mixed_border_then_ocr_rejections_keep_the_third_attempt_on_the_current_scene() {
        let temporary = TempDir::new().expect("tempdir must be created");
        write_rejection(temporary.path(), 1, "border");
        write_rejection(temporary.path(), 2, "ocr");
        assert!(
            !PictureRecovery::default()
                .prepare(temporary.path(), 2)
                .expect("mixed local verdicts must decode"),
            "border then OCR discarded a scene that could still render cleanly"
        );
    }

    #[test]
    fn mixed_topology_then_ocr_rejections_enable_third_attempt_recomposition() {
        let temporary = TempDir::new().expect("tempdir must be created");
        write_rejection(temporary.path(), 1, "topology");
        write_rejection(temporary.path(), 2, "ocr");
        assert!(
            PictureRecovery::default()
                .prepare(temporary.path(), 2)
                .expect("mixed local verdicts must decode"),
            "topology evidence followed by OCR did not advance the third picture to a fresh scene"
        );
    }

    #[test]
    fn two_topology_rejections_enable_third_attempt_recomposition() {
        let temporary = TempDir::new().expect("tempdir must be created");
        write_rejection(temporary.path(), 1, "topology");
        write_rejection(temporary.path(), 2, "topology");
        assert!(
            PictureRecovery::default()
                .prepare(temporary.path(), 2)
                .expect("topology verdicts must decode"),
            "repeated topology rejections did not recompose the third picture attempt"
        );
    }

    #[test]
    fn one_topology_rejection_keeps_the_second_attempt_on_the_current_scene() {
        let temporary = TempDir::new().expect("tempdir must be created");
        write_rejection(temporary.path(), 1, "topology");
        assert!(
            !PictureRecovery::default()
                .prepare(temporary.path(), 1)
                .expect("topology verdict must decode"),
            "one noisy topology verdict discarded the scene before an image retry"
        );
    }

    #[test]
    fn persisted_topology_rejection_keeps_the_second_picture_on_the_committed_scene_slot() {
        let temporary = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("visual", temporary.path());
        let scene = serde_json::json!({
            "manga_panel": {"meta": {"layout_selection": {"scene_attempt_index": 0}}}
        });
        fs::write(
            cache.filepath(SCENE_FILE).expect("scene path must resolve"),
            serde_json::to_vec(&scene).expect("scene provenance must encode"),
        )
        .expect("scene provenance must be written");
        let attempts = cache.path().join(IMAGE_ATTEMPTS_DIRECTORY);
        fs::create_dir_all(attempts.as_path()).expect("attempt journal must be created");
        fs::write(
            attempts.join("attempt-0001.scene.json"),
            serde_json::to_vec(&scene).expect("attempt scene must encode"),
        )
        .expect("attempt scene must be written");
        write_rejection(cache.path().as_path(), 1, "topology");
        let recover = PictureRecovery::default()
            .prepare(cache.path().as_path(), 1)
            .expect("persisted topology verdict must decode");
        let selected = reserve_scene_attempt(&cache, Artifact::Picture, 0, recover)
            .expect("second picture must retain the committed scene");
        assert_eq!(
            (
                recover,
                selected,
                load_scene_attempt(&cache).expect("cursor must decode")
            ),
            (false, 0, Some(0)),
            "a restarted second picture advanced after only one topology verdict"
        );
    }

    #[test]
    fn provider_failure_does_not_count_as_a_local_rejection() {
        let temporary = TempDir::new().expect("tempdir must be created");
        let recovery = PictureRecovery::default();
        recovery
            .prepare(temporary.path(), 0)
            .expect("first attempt must prepare");
        recovery
            .observe(temporary.path(), 0, Some(LocalImageRejection::Border))
            .expect("first local rejection must record");
        recovery
            .prepare(temporary.path(), 1)
            .expect("provider attempt must prepare");
        recovery
            .observe(temporary.path(), 1, None)
            .expect("provider failure must record");
        assert!(
            !recovery
                .prepare(temporary.path(), 2)
                .expect("third attempt must prepare"),
            "one provider failure was misclassified as a second local rejection"
        );
    }

    #[test]
    fn persisted_decode_failure_does_not_count_as_a_local_rejection() {
        let temporary = TempDir::new().expect("tempdir must be created");
        write_rejection(temporary.path(), 1, "border");
        write_verdict(temporary.path(), 2, "error", "transport_or_decode");
        assert!(
            !PictureRecovery::default()
                .prepare(temporary.path(), 2)
                .expect("persisted image outcomes must decode"),
            "a decode failure was misclassified as a second local rejection"
        );
    }

    #[test]
    fn topology_then_ocr_recomposition_survives_a_fresh_generator_process() {
        let temporary = TempDir::new().expect("tempdir must be created");
        write_rejection(temporary.path(), 1, "topology");
        write_rejection(temporary.path(), 2, "ocr");
        assert!(
            PictureRecovery::default()
                .prepare(temporary.path(), 2)
                .expect("persisted local verdicts must decode"),
            "a restarted generator forgot topology evidence before the third picture"
        );
    }

    #[test]
    fn repeated_border_recomposition_survives_a_fresh_generator_process() {
        let temporary = TempDir::new().expect("tempdir must be created");
        write_rejection(temporary.path(), 1, "border");
        write_rejection(temporary.path(), 2, "border");
        assert!(
            PictureRecovery::default()
                .prepare(temporary.path(), 2)
                .expect("persisted border verdicts must decode"),
            "a restarted generator forgot repeated border evidence before the third picture"
        );
    }

    /// Persist one deterministic local image-rejection verdict for restart tests.
    fn write_rejection(path: &Path, sequence: usize, category: &str) {
        write_verdict(path, sequence, "rejected", category);
    }

    /// Persist one deterministic image-attempt verdict for recovery tests.
    fn write_verdict(path: &Path, sequence: usize, status: &str, category: &str) {
        let attempts = path.join(IMAGE_ATTEMPTS_DIRECTORY);
        fs::create_dir_all(attempts.as_path()).expect("attempt journal must be created");
        fs::write(
            attempts.join(format!("attempt-{sequence:04}.json")),
            serde_json::to_vec(&serde_json::json!({
                "sequence": sequence,
                "status": status,
                "category": category,
                "reason": "deterministic image-attempt verdict"
            }))
            .expect("attempt verdict must encode"),
        )
        .expect("attempt verdict must be written");
    }

    #[test]
    fn malformed_persisted_verdict_does_not_invent_a_second_rejection() {
        let temporary = TempDir::new().expect("tempdir must be created");
        let attempts = temporary.path().join(IMAGE_ATTEMPTS_DIRECTORY);
        fs::create_dir_all(attempts.as_path()).expect("attempt journal must be created");
        fs::write(
            attempts.join("attempt-0001.json"),
            serde_json::to_vec(&serde_json::json!({
                "status": "rejected",
                "category": "ocr"
            }))
            .expect("attempt verdict must encode"),
        )
        .expect("attempt verdict must be written");
        fs::write(attempts.join("attempt-0002.json"), b"{")
            .expect("broken verdict must be written");
        assert!(
            !PictureRecovery::default()
                .prepare(temporary.path(), 0)
                .expect("a broken verdict must degrade to one known rejection"),
            "a partial verdict invented a second local image rejection"
        );
    }

    #[test]
    fn scene_recovery_advances_from_persisted_layout_attempt() {
        let temporary = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("visual", temporary.path());
        fs::write(
            cache.filepath(SCENE_FILE).expect("scene path must resolve"),
            serde_json::to_vec(&serde_json::json!({
                "manga_panel": {"meta": {"layout_selection": {"scene_attempt_index": 4}}}
            }))
            .expect("scene provenance must encode"),
        )
        .expect("scene provenance must be written");
        assert_eq!(
            scene_attempt_cursor(&cache, 0)
                .expect("scene provenance must decode")
                .committed,
            Some(4_u8),
            "a restarted recovery returned to an already failed layout slot"
        );
    }

    #[test]
    fn rejected_recomposition_advances_beyond_the_archived_layout_attempt() {
        let temporary = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("visual", temporary.path());
        fs::write(
            cache.filepath(SCENE_FILE).expect("scene path must resolve"),
            serde_json::to_vec(&serde_json::json!({
                "manga_panel": {"meta": {"layout_selection": {"scene_attempt_index": 4}}}
            }))
            .expect("scene provenance must encode"),
        )
        .expect("scene provenance must be written");
        let attempts = cache.path().join(IMAGE_ATTEMPTS_DIRECTORY);
        fs::create_dir_all(attempts.as_path()).expect("attempt archive must be created");
        fs::write(
            attempts.join("attempt-0007.scene.json"),
            serde_json::to_vec(&serde_json::json!({
                "manga_panel": {"meta": {"layout_selection": {"scene_attempt_index": 5}}}
            }))
            .expect("rejected scene provenance must encode"),
        )
        .expect("rejected scene provenance must be written");
        let cursor = scene_attempt_cursor(&cache, 0).expect("attempt archive must decode");
        assert_eq!(
            (
                cursor.has_rejected_recomposition(),
                cursor.current(0),
                cursor.next(0).expect("alternate slot must select"),
            ),
            (true, 4, 6),
            "regeneration repeated an already rejected scene alternate"
        );
    }

    #[test]
    fn fresh_worker_advances_beyond_three_durably_reserved_scene_failures() {
        let temporary = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("visual", temporary.path());
        let first = reserve_scene_attempt(&cache, Artifact::Scene, 0, false)
            .expect("first attempt must reserve");
        let second = reserve_scene_attempt(&cache, Artifact::Scene, 1, false)
            .expect("second attempt must reserve");
        let third = reserve_scene_attempt(&cache, Artifact::Scene, 2, false)
            .expect("third attempt must reserve");
        let restarted = reserve_scene_attempt(&cache, Artifact::Scene, 0, false)
            .expect("fresh worker must reserve");
        let stored = load_scene_attempt(&cache).expect("cursor must decode");
        assert_eq!(
            (first, second, third, restarted, stored),
            (0, 1, 2, 3, Some(3)),
            "a fresh worker reused a scene slot whose composer call already failed"
        );
    }

    #[test]
    fn newly_committed_scene_is_rendered_before_old_rejections_recompose_again() {
        let cursor = SceneAttemptCursor {
            committed: Some(6),
            archived: Some(5),
            attempted: Some(6),
        };
        assert!(
            !cursor.recompose(true),
            "a crash-safe committed scene was skipped before its first image attempt"
        );
    }

    #[test]
    fn picture_recovery_state_is_isolated_by_visual_cache_path() {
        let recovery = PictureRecovery::default();
        let first = Path::new("cards/first");
        let second = Path::new("cards/second");
        recovery.prepare(first, 0).expect("first card must prepare");
        recovery
            .observe(first, 0, Some(LocalImageRejection::Topology))
            .expect("first card outcome must record");
        recovery
            .prepare(first, 1)
            .expect("first retry must prepare");
        recovery
            .observe(first, 1, Some(LocalImageRejection::Topology))
            .expect("first retry outcome must record");
        recovery
            .prepare(second, 0)
            .expect("second card must prepare");
        recovery
            .observe(second, 0, Some(LocalImageRejection::Topology))
            .expect("second card outcome must record");
        assert_eq!(
            (
                recovery
                    .prepare(first, 2)
                    .expect("first third attempt must prepare"),
                recovery
                    .prepare(second, 1)
                    .expect("second retry must prepare"),
            ),
            (true, false),
            "one card's local rejection count contaminated another visual cache"
        );
    }

    #[test]
    fn a_fresh_picture_tally_resets_stale_recovery_state() {
        let recovery = PictureRecovery::default();
        let path = Path::new("cards/rerolled");
        recovery
            .prepare(path, 0)
            .expect("first attempt must prepare");
        recovery
            .observe(path, 0, Some(LocalImageRejection::RecallText))
            .expect("first outcome must record");
        recovery.prepare(path, 1).expect("retry must prepare");
        recovery
            .observe(path, 1, Some(LocalImageRejection::RecallText))
            .expect("retry outcome must record");
        let reset = recovery.prepare(path, 0).expect("reroll must reset");
        recovery
            .observe(path, 0, Some(LocalImageRejection::RecallText))
            .expect("new first outcome must record");
        assert_eq!(
            (
                reset,
                recovery.prepare(path, 1).expect("new retry must prepare")
            ),
            (false, false),
            "a rerolled card inherited the previous picture series"
        );
    }

    #[test]
    fn duplicate_visual_paths_hold_one_lock_without_deadlocking() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("cards/test", home.path())
            .visual("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .expect("visual cache must resolve");
        let guards = hold_visuals(vec![cache.clone(), cache], Duration::ZERO)
            .expect("duplicate visual paths must acquire one lock");
        assert_eq!(
            guards.len(),
            1,
            "duplicate visual paths acquired the same non-reentrant lock twice"
        );
    }

    #[test]
    fn correction_observer_persists_the_exact_billed_request() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("cards/test", home.path());
        let record = CostRecord::new(
            "gemini-3.6-flash",
            1,
            100,
            20,
            120,
            GenerationCost::from_nanos(300_000),
        );
        persist_correction_cost(&cache, &record).expect("correction cost must persist");
        assert_eq!(
            load_cost_record(&cache, Artifact::Meta)
                .expect("meta cost must decode")
                .map(|stored| (stored.requests(), stored.cost())),
            Some((1, GenerationCost::from_nanos(300_000))),
            "correction observer discarded or inflated its exact request"
        );
    }

    #[test]
    fn provider_observer_journals_session_spend_before_lifetime_sidecar_failure() {
        let home = TempDir::new().expect("tempdir must be created");
        let scope = SessionCostScope::for_run(home.path(), "fr-1", "created-a");
        scope.overlay(&[]).expect("session journal must seed");
        let recorder = CostRecorder::attributed(
            Cache::failing("cards/test", home.path(), 0),
            Artifact::Picture,
            Some(SessionCostAttribution::new(scope.clone(), 0)),
        );
        let result = recorder.push(CostRecord::new(
            "gemini-3.1-flash-image",
            1,
            100,
            20,
            120,
            GenerationCost::from_nanos(700_000),
        ));
        assert_eq!(
            (
                result.is_err(),
                scope
                    .absolute(0, ArtifactCosts::default())
                    .expect("journal must remain readable")
                    .cost(Artifact::Picture),
            ),
            (true, Some(GenerationCost::from_nanos(700_000))),
            "lifetime sidecar failure happened before session spend became durable"
        );
    }

    #[test]
    fn correction_cost_waits_for_the_stable_meta_lease() {
        let Some(root) = std::env::var_os("KAMISHIBAI_CORRECTION_LOCK_ROOT") else {
            let home = TempDir::new().expect("tempdir must be created");
            let cache = Cache::new("cards/test", home.path());
            let guard = cache
                .hold_root_stage(RootStage::Meta, Duration::ZERO)
                .expect("meta lease must be acquired");
            let mut child = Command::new(
                std::env::current_exe().expect("test binary must resolve"),
            )
            .args([
                "cli::live_generator::tests::correction_cost_waits_for_the_stable_meta_lease",
                "--exact",
                "--nocapture",
            ])
            .env("KAMISHIBAI_CORRECTION_LOCK_ROOT", home.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("cost writer child must spawn");
            std::thread::sleep(Duration::from_millis(100));
            let waited = child
                .try_wait()
                .expect("child state must be observable")
                .is_none()
                && !cache.exists(META_COST_FILE);
            drop(guard);
            let deadline = Instant::now() + Duration::from_secs(5);
            let succeeded = loop {
                if let Some(status) = child.try_wait().expect("child state must be observable") {
                    break status.success();
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break false;
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            assert_eq!(
                (waited, succeeded, cache.exists(META_COST_FILE)),
                (true, true, true),
                "correction cost bypassed the stable meta lease or failed after it released"
            );
            return;
        };
        let cache = Cache::new("cards/test", PathBuf::from(root));
        let record = CostRecord::new(
            "gemini-3.6-flash",
            1,
            100,
            20,
            120,
            GenerationCost::from_nanos(300_000),
        );
        let stored = persist_correction_cost(&cache, &record);
        assert!(
            stored.is_ok() && cache.exists(META_COST_FILE),
            "child failed to persist billed correction cost under the meta lease"
        );
    }

    #[test]
    fn cached_artifacts_do_not_report_historical_cost() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("cards/test", home.path());
        let record = CostRecord::new(
            "gemini-3.6-flash",
            1,
            100,
            20,
            120,
            GenerationCost::from_nanos(300_000),
        );
        store_cost(&cache, Artifact::Sound, &record).expect("cost must persist");
        let costs = CostRecorder::new(cache, Artifact::Sound);
        assert_eq!(
            (
                costs.cumulative(true).expect("cache cost must settle"),
                costs.cumulative(false).expect("run cost must settle"),
            ),
            (None, None),
            "cache hits must not count historical Gemini cost as current spend"
        );
    }

    #[test]
    fn fresh_artifacts_report_current_request_cost() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("cards/test", home.path());
        let record = CostRecord::new(
            "gemini-3.6-flash",
            1,
            100,
            20,
            120,
            GenerationCost::from_nanos(300_000),
        );
        let costs = CostRecorder::new(cache, Artifact::Sound);
        costs.push(record).expect("cost must persist");
        assert_eq!(
            costs.cumulative(false).expect("cost must settle"),
            Some(GenerationCost::from_nanos(300_000)),
            "fresh Gemini requests must report their current spend"
        );
    }

    #[test]
    fn a_new_run_does_not_inherit_historical_sidecar_cost() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("cards/test", home.path());
        let historical = CostRecord::new(
            "gemini-3.6-flash",
            2,
            200,
            40,
            240,
            GenerationCost::from_nanos(900_000),
        );
        let current = CostRecord::new(
            "gemini-3.6-flash",
            1,
            100,
            20,
            120,
            GenerationCost::from_nanos(300_000),
        );
        store_cost(&cache, Artifact::Sound, &historical).expect("historical cost must persist");
        let costs = CostRecorder::new(cache.clone(), Artifact::Sound);
        costs.push(current).expect("current cost must persist");
        assert_eq!(
            (
                costs.cumulative(false).expect("run cost must load"),
                load_cost(&cache, Artifact::Sound).expect("lifetime cost must load"),
            ),
            (
                Some(GenerationCost::from_nanos(300_000)),
                Some(GenerationCost::from_nanos(1_200_000)),
            ),
            "a fresh generation run inherited the card's lifetime sidecar spend"
        );
    }

    #[test]
    fn fresh_artifacts_report_accumulated_retry_cost() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("cards/test", home.path());
        let first = CostRecord::new(
            "gemini-3.6-flash",
            1,
            100,
            20,
            120,
            GenerationCost::from_nanos(300_000),
        );
        let second = CostRecord::new(
            "gemini-3.6-flash",
            1,
            40,
            10,
            50,
            GenerationCost::from_nanos(135_000),
        );
        let costs = CostRecorder::new(cache, Artifact::Sound);
        costs.push(first).expect("first cost must persist");
        costs.push(second).expect("retry cost must persist");
        assert_eq!(
            costs.cumulative(false).expect("retry cost must settle"),
            Some(GenerationCost::from_nanos(435_000)),
            "fresh retry success must report all successful Gemini requests for the artifact"
        );
    }

    #[test]
    fn one_operation_reports_all_observed_request_cost() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("cards/test", home.path());
        let first = CostRecord::new(
            "gemini-3.6-flash",
            1,
            100,
            20,
            120,
            GenerationCost::from_nanos(300_000),
        );
        let second = CostRecord::new(
            "gemini-3.6-flash",
            1,
            40,
            10,
            50,
            GenerationCost::from_nanos(135_000),
        );
        let costs = CostRecorder::new(cache, Artifact::Sound);
        costs.push(first).expect("first cost must persist");
        let first_cost = costs.cumulative(false).expect("first cost must settle");
        let unmetered = costs.cumulative(false).expect("cost must remain");
        costs.push(second).expect("second cost must persist");
        let second_cost = costs.cumulative(false).expect("second cost must settle");
        assert_eq!(
            (first_cost, unmetered, second_cost),
            (
                Some(GenerationCost::from_nanos(300_000)),
                Some(GenerationCost::from_nanos(300_000)),
                Some(GenerationCost::from_nanos(435_000)),
            ),
            "one provider operation did not return all observed request spend"
        );
    }

    #[test]
    fn missing_usage_records_do_not_report_zero_costs() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("cards/test", home.path());
        let record = CostRecord::new("gemini-3.6-flash", 0, 0, 0, 0, GenerationCost::zero());
        let costs = CostRecorder::new(cache, Artifact::Sound);
        costs.push(record.clone()).expect("zero usage must settle");
        let fresh = costs.cumulative(false).expect("zero usage must settle");
        costs.push(record).expect("zero usage retry must settle");
        let retry = costs
            .cumulative(false)
            .expect("zero usage retry must settle");
        assert_eq!(
            (fresh, retry),
            (None, None),
            "missing Gemini usage metadata must leave the request cost absent"
        );
    }

    #[test]
    fn recomposition_persists_scene_and_picture_spend_in_separate_sidecars() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("cards/test", home.path());
        let scene = CostRecord::new(
            "gemini-3.6-flash",
            1,
            100,
            20,
            120,
            GenerationCost::from_nanos(300_000),
        );
        let picture = CostRecord::new(
            "gemini-3.1-flash-image",
            1,
            40,
            10,
            50,
            GenerationCost::from_nanos(900_000),
        );
        let scene_costs = CostRecorder::new(cache.clone(), Artifact::Scene);
        let picture_costs = CostRecorder::new(cache.clone(), Artifact::Picture);
        scene_costs.push(scene).expect("scene cost must persist");
        picture_costs
            .push(picture)
            .expect("picture cost must persist");
        let costs = visual_costs(Artifact::Picture, false, &scene_costs, &picture_costs)
            .expect("visual costs must load");
        assert_eq!(
            (
                costs,
                load_cost_record(&cache, Artifact::Scene)
                    .expect("scene cost must decode")
                    .map(|record| (record.model().to_string(), record.requests())),
                load_cost_record(&cache, Artifact::Picture)
                    .expect("picture cost must decode")
                    .map(|record| (record.model().to_string(), record.requests())),
            ),
            (
                (
                    Some(GenerationCost::from_nanos(900_000)),
                    Some(GenerationCost::from_nanos(300_000)),
                ),
                Some((String::from("gemini-3.6-flash"), 1)),
                Some((String::from("gemini-3.1-flash-image"), 1)),
            ),
            "recomposition mixed the scene request into picture accounting"
        );
    }

    #[test]
    fn metering_refuses_to_continue_after_cost_persistence_fails() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::failing("cards/test", home.path(), 0);
        let costs = CostRecorder::new(cache.clone(), Artifact::Picture);
        let record = CostRecord::new(
            "gemini-3.1-flash-image",
            1,
            40,
            10,
            50,
            GenerationCost::from_nanos(900_000),
        );
        let result = costs.push(record);
        assert_eq!(
            (result.is_err(), cache.exists(ILLUSTRATION_COST_FILE)),
            (true, false),
            "a cost persistence failure was hidden from the provider boundary"
        );
    }

    #[test]
    fn cost_persistence_failure_prevents_the_artifact_commit() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::failing("cards/test", home.path(), 0);
        let costs = CostRecorder::new(cache.clone(), Artifact::Sound);
        let audio = Audio::new(cache.clone(), "Read {text}", PersistingSpeaker { costs });
        let result = audio.generate("hello");
        assert_eq!(
            (result.is_err(), cache.exists(VOICE_FILE)),
            (true, false),
            "an audio artifact committed after its usage record failed to persist"
        );
    }
}
