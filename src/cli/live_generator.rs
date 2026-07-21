//! Live implementation of the UI-neutral card workflow.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use time::OffsetDateTime;
use time::format_description::parse as parse_time;

use super::card_workflow::{
    CardGeneration, DeckPublishing, KeyValidation, PublishPhase, PublishProgress,
};
use crate::anki::{CardModel, StableId, VocabularyDeck, VocabularyNote};
use crate::config::default_store;
use crate::gemini::{GeminiClient, HttpTransport};
use crate::generation::artifact_cache::{
    Cache, ILLUSTRATION_COST_FILE, ILLUSTRATION_FILE, META_COST_FILE, SCENE_COST_FILE,
    VISUAL_LOCK_TIMEOUT, VOICE_COST_FILE, VOICE_FILE, VisualGuard,
};
use crate::generation::manga::TextEnsemble;
use crate::generation::manga::{
    BorderDetector, Illustration, ImageSource, MangaRenderer, Progress as SceneProgress,
    TextDetector,
};
use crate::generation::speech::Audio;
use crate::generation::{
    SceneComposer, SceneSource, Speaker, render_audio_prompt, visual_revision,
};
use crate::languages::{LanguageCatalog, catalog, naming};
use crate::report::{CardSheet, Thumbnail};
use crate::runtime::locations::{LocationArgs, Locations, SystemContext};
use crate::session::{
    Artifact, ArtifactFile, BulkCorrection, CachedUnderstanding, CardCell, CardCorrection,
    CardDraft, CardMeta, CardMetaCache, CardMetaGeneration, CardRevision, CostRecord,
    GenerationCost, LanguagePair, RawInputBatch, Understanding, Understood, WordCandidate,
    to_entry,
};
use crate::vocabulary::VocabularyEntry;

const IMAGE_STYLE: &str = "max-width: 100%; height: auto; border-radius: 10px";
const IMAGE_ATTEMPTS_PER_ARTIFACT: usize = 1;

type LiveText = TextEnsemble<TextDetector>;
type LiveIllustration = Illustration<SceneComposer<MeteredGemini>, MangaRenderer<LiveText>>;

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
    keys: KeyLookup,
}

impl LiveCardGenerator {
    /// Build a live card generator for the interactive flow (saved key only).
    pub(super) fn new(cache: PathBuf, output: PathBuf) -> Self {
        Self {
            cache,
            output,
            catalog: catalog(),
            keys: KeyLookup::Saved,
        }
    }

    /// Build a live card generator for the console flow, where `GEMINI_API_KEY`
    /// is the documented key source and wins over any saved preference.
    pub(super) fn for_console(cache: PathBuf, output: PathBuf) -> Self {
        Self {
            cache,
            output,
            catalog: catalog(),
            keys: KeyLookup::Environment,
        }
    }

    fn client(&self) -> Result<GeminiClient<HttpTransport>> {
        let saved_key = default_store(&SystemContext)
            .ok()
            .and_then(|store| store.read().ok())
            .and_then(|prefs| prefs.api_key);
        match self.keys {
            KeyLookup::Saved => GeminiClient::from_saved(saved_key.as_deref()),
            KeyLookup::Environment => GeminiClient::from_env_or_saved(saved_key.as_deref()),
        }
    }

    fn meta_cache(&self) -> CardMetaCache {
        CardMetaCache::new(self.cache.clone())
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
        costs: CostRecorder,
    ) -> Result<LiveIllustration> {
        let item = self.catalog.item(draft.pair().learning())?;
        let client = self.client()?;
        let metered = MeteredGemini::new(client, costs);
        let text = manga_text(item.ocr.as_str(), self.cache.as_path());
        let renderer =
            production_renderer(metered.clone(), text, BorderDetector::new(6, 24, 240, 10));
        let renderer = renderer.with_attempt_archive(cache.filepath("attempts")?);
        Ok(Illustration::new(
            cache,
            SceneComposer::new(metered.clone(), item.prompt.as_str(), draft.term()),
            renderer,
        ))
    }

    fn generate_visual<F>(
        &self,
        draft: &CardDraft,
        artifact: Artifact,
        render: F,
    ) -> Result<ArtifactFile>
    where
        F: FnOnce(&LiveIllustration, &str, &str, &mut NoopProgress) -> Result<(String, bool)>,
    {
        let meta = draft
            .meta()
            .ok_or_else(|| anyhow!("meta must be ready before {}", artifact.label()))?;
        let learning = draft.pair().learning();
        let costs = CostRecorder::default();
        let cache = self.cell(draft).cache().visual(visual_revision())?;
        let illustration = self.illustration(draft, cache.clone(), costs.clone())?;
        let _guard = cache.hold_visual(VISUAL_LOCK_TIMEOUT)?;
        let mut progress = NoopProgress;
        let result = render(
            &illustration,
            meta.target_sentence(),
            learning,
            &mut progress,
        )
        .and_then(|(filename, cached)| {
            let path = illustration.filepath(filename.as_str())?;
            Ok((filename, path, cached))
        });
        let record = costs.aggregate();
        match result {
            Ok((filename, path, cached)) => {
                let cost = cost_for(&cache, artifact, cached, record);
                Ok(artifact_file(filename, path, cached, cost))
            }
            Err(error) => {
                store_retry_cost(&cache, artifact, record);
                Err(error)
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
        if let Some(meta) = self.meta_cache().load(term, understanding, pair)? {
            return Ok(meta);
        }
        let cache = CardCell::new(self.cache.clone(), pair, term, understanding).cache();
        self.client()?
            .generate_card_meta_observed(term, understanding, pair, |cost| {
                store_cost(&cache, Artifact::Meta, &cost);
            })
    }
}

impl CardCorrection for LiveCardGenerator {
    fn correct_card(
        &self,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<CardRevision> {
        let (revision, cost) = self.client()?.correct_card_metered(draft, comment, pair)?;
        let cache = CardCell::new(
            self.cache.clone(),
            pair,
            revision.term(),
            revision.understanding(),
        )
        .cache();
        store_cost(&cache, Artifact::Meta, &cost);
        Ok(revision)
    }
}

impl KeyValidation for LiveCardGenerator {
    fn check_key(&self, key: &str) -> Result<()> {
        GeminiClient::new(key, HttpTransport::new()).validate_key()
    }
}

impl CardGeneration for LiveCardGenerator {
    fn generate_scene(&self, draft: &CardDraft) -> Result<ArtifactFile> {
        self.generate_visual(
            draft,
            Artifact::Scene,
            |illustration, sentence, target, progress| {
                illustration.scene_only(sentence, target, progress)
            },
        )
    }

    fn generate_picture(&self, draft: &CardDraft) -> Result<ArtifactFile> {
        self.generate_visual(
            draft,
            Artifact::Picture,
            |illustration, sentence, target, progress| {
                illustration.picture_only(sentence, target, progress)
            },
        )
    }

    fn generate_sound(&self, draft: &CardDraft) -> Result<ArtifactFile> {
        let meta = draft
            .meta()
            .ok_or_else(|| anyhow!("meta must be ready before sound"))?;
        let costs = CostRecorder::default();
        let cache = self.cell(draft).cache();
        let audio = self.audio(draft, costs.clone())?;
        let result = audio
            .generate(meta.target_sentence())
            .and_then(|(filename, cached)| {
                let path = audio.filepath(filename.as_str())?;
                Ok((filename, path, cached))
            });
        let record = costs.aggregate();
        match result {
            Ok((filename, path, cached)) => {
                let cost = cost_for(&cache, Artifact::Sound, cached, record);
                Ok(artifact_file(filename, path, cached, cost))
            }
            Err(error) => {
                store_retry_cost(&cache, Artifact::Sound, record);
                Err(error)
            }
        }
    }

    fn store_card_meta(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        meta: &CardMeta,
    ) -> Result<ArtifactFile> {
        let (filename, path, cached) = self.meta_cache().store(term, understanding, pair, meta)?;
        let cache = CardCell::new(self.cache.clone(), pair, term, understanding).cache();
        let cost = if cached {
            None
        } else {
            load_cost(&cache, Artifact::Meta)
        };
        Ok(artifact_file(filename, path, cached, cost))
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

#[derive(Clone, Debug, Default)]
struct CostRecorder {
    records: Rc<RefCell<Vec<CostRecord>>>,
}

impl CostRecorder {
    fn push(&self, record: CostRecord) {
        if record.requests() == 0 {
            return;
        }
        self.records.borrow_mut().push(record);
    }

    fn aggregate(&self) -> Option<CostRecord> {
        CostRecord::aggregate(self.records.borrow().as_slice())
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
    ) -> Result<serde_json::Value> {
        self.client
            .scene_observed(language, term, sentence, target, |cost| {
                self.costs.push(cost)
            })
    }
}

impl ImageSource for MeteredGemini {
    fn image(&self, scene: &serde_json::Value) -> Result<Vec<u8>> {
        self.client
            .image_observed(scene, |cost| self.costs.push(cost))
    }
}

impl Speaker for MeteredGemini {
    fn speech(&self, prompt: &str, text: &str) -> Result<Vec<u8>> {
        self.client
            .speech_observed(prompt, text, |cost| self.costs.push(cost))
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

fn cost_for(
    cache: &Cache,
    artifact: Artifact,
    cached: bool,
    record: Option<CostRecord>,
) -> Option<GenerationCost> {
    if cached {
        return None;
    }
    if let Some(record) = record {
        if record.requests() == 0 {
            return load_cost(cache, artifact);
        }
        return Some(store_cost(cache, artifact, &record).cost());
    }
    load_cost(cache, artifact)
}

fn load_cost(cache: &Cache, artifact: Artifact) -> Option<GenerationCost> {
    load_cost_record(cache, artifact).map(|record| record.cost())
}

fn load_cost_record(cache: &Cache, artifact: Artifact) -> Option<CostRecord> {
    let filename = cost_filename(artifact);
    if !cache.exists(filename) {
        return None;
    }
    let path = cache.filepath(filename).ok()?;
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str::<CostRecord>(text.as_str()).ok()
}

fn store_cost(cache: &Cache, artifact: Artifact, record: &CostRecord) -> CostRecord {
    if record.requests() == 0 {
        return load_cost_record(cache, artifact).unwrap_or_else(|| record.clone());
    }
    let merged = load_cost_record(cache, artifact)
        .map(|existing| existing.merged(record))
        .unwrap_or_else(|| record.clone());
    let Ok(staged) = cache.stage(".cost.json") else {
        return merged;
    };
    let result = serde_json::to_string_pretty(&merged)
        .map_err(anyhow::Error::from)
        .and_then(|json| fs::write(&staged, json).map_err(anyhow::Error::from))
        .and_then(|()| cache.commit(&staged, cost_filename(artifact)));
    if result.is_err() {
        let _ = fs::remove_file(&staged);
    }
    merged
}

fn store_retry_cost(cache: &Cache, artifact: Artifact, record: Option<CostRecord>) {
    if let Some(record) = record {
        store_cost(cache, artifact, &record);
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

fn manga_text(value: &str, cache: &Path) -> TextEnsemble<TextDetector> {
    let mut detectors = vec![TextDetector::cached(60, value, cache)];
    if !value.split('+').any(|language| language == "jpn") {
        detectors.push(TextDetector::cached(60, "jpn", cache));
    }
    TextEnsemble::new(detectors)
}

fn production_renderer<C, D>(client: C, text: D, border: BorderDetector) -> MangaRenderer<D>
where
    C: ImageSource + 'static,
{
    MangaRenderer::new(client, IMAGE_ATTEMPTS_PER_ARTIFACT, text, border)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io::Cursor;

    use image::{DynamicImage, GrayImage, ImageFormat, Luma};
    use serde_json::Value;
    use tempfile::TempDir;

    use super::*;
    use crate::generation::manga::{Renderer, SceneText};

    #[derive(Clone, Debug)]
    struct CountingImageSource {
        calls: Rc<Cell<usize>>,
        image: Vec<u8>,
    }

    impl CountingImageSource {
        fn new() -> Self {
            let mut image = Cursor::new(Vec::new());
            DynamicImage::ImageLuma8(GrayImage::from_pixel(16, 16, Luma([0])))
                .write_to(&mut image, ImageFormat::Png)
                .expect("test image must encode");
            Self {
                calls: Rc::new(Cell::new(0)),
                image: image.into_inner(),
            }
        }

        fn calls(&self) -> usize {
            self.calls.get()
        }
    }

    impl ImageSource for CountingImageSource {
        fn image(&self, _scene: &Value) -> Result<Vec<u8>> {
            self.calls.set(self.calls.get() + 1);
            Ok(self.image.clone())
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct RejectingText;

    impl SceneText for RejectingText {
        fn detected(&self, _scene: &Value, _image: &GrayImage) -> Result<String> {
            Ok(String::from("detected text"))
        }
    }

    #[test]
    fn production_renderer_spends_one_image_call_per_artifact_attempt() {
        let source = CountingImageSource::new();
        let renderer = production_renderer(
            source.clone(),
            RejectingText,
            BorderDetector::new(2, 6, 240, 2),
        );
        let result = renderer.render(
            &serde_json::json!({"manga_panel": {"panels": []}}),
            &mut NoopProgress,
        );
        assert_eq!(
            (result.is_err(), source.calls()),
            (true, 1),
            "one outer artifact attempt multiplied into multiple image calls"
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
    fn cached_artifacts_do_not_report_historical_cost() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("cards/test", home.path());
        let record = CostRecord::new(
            "gemini-3.5-flash",
            1,
            100,
            20,
            120,
            GenerationCost::from_nanos(330_000),
        );
        store_cost(&cache, Artifact::Sound, &record);
        assert_eq!(
            cost_for(&cache, Artifact::Sound, true, None),
            None,
            "cache hits must not count historical Gemini cost as current spend"
        );
    }

    #[test]
    fn fresh_artifacts_report_current_request_cost() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("cards/test", home.path());
        let record = CostRecord::new(
            "gemini-3.5-flash",
            1,
            100,
            20,
            120,
            GenerationCost::from_nanos(330_000),
        );
        assert_eq!(
            cost_for(&cache, Artifact::Sound, false, Some(record)),
            Some(GenerationCost::from_nanos(330_000)),
            "fresh Gemini requests must report their current spend"
        );
    }

    #[test]
    fn fresh_artifacts_report_accumulated_retry_cost() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("cards/test", home.path());
        let first = CostRecord::new(
            "gemini-3.5-flash",
            1,
            100,
            20,
            120,
            GenerationCost::from_nanos(330_000),
        );
        let second = CostRecord::new(
            "gemini-3.5-flash",
            1,
            40,
            10,
            50,
            GenerationCost::from_nanos(150_000),
        );
        store_cost(&cache, Artifact::Sound, &first);
        assert_eq!(
            cost_for(&cache, Artifact::Sound, false, Some(second)),
            Some(GenerationCost::from_nanos(480_000)),
            "fresh retry success must report all successful Gemini requests for the artifact"
        );
    }

    #[test]
    fn missing_usage_records_do_not_report_zero_costs() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("cards/test", home.path());
        let record = CostRecord::new("gemini-3.5-flash", 0, 0, 0, 0, GenerationCost::zero());
        assert_eq!(
            cost_for(&cache, Artifact::Sound, false, Some(record)),
            None,
            "missing Gemini usage metadata must leave the request cost absent"
        );
    }
}
