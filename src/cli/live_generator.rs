//! Live implementation of the UI-neutral card workflow.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use time::OffsetDateTime;
use time::format_description::parse as parse_time;

use super::card_workflow::{
    CardGeneration, DeckPublishing, KeyValidation, PublishPhase, PublishProgress,
};
use crate::anki::{CardModel, StableId, VocabularyDeck, VocabularyNote};
use crate::config::default_store;
use crate::gemini::{GeminiClient, HttpTransport};
use crate::generation::artifact_cache::{ILLUSTRATION_FILE, VOICE_FILE};
use crate::generation::manga::{
    BorderDetector, Illustration, MangaRenderer, Progress as SceneProgress, TextDetector,
};
use crate::generation::speech::Audio;
use crate::generation::{SceneComposer, render_audio_prompt};
use crate::languages::{LanguageCatalog, catalog, naming};
use crate::report::{CardSheet, Thumbnail};
use crate::runtime::locations::{LocationArgs, Locations, SystemContext};
use crate::session::{
    ArtifactFile, BulkCorrection, CachedUnderstanding, CardCell, CardCorrection, CardDraft,
    CardMeta, CardMetaCache, CardMetaGeneration, CardRevision, LanguagePair, RawInputBatch,
    Understanding, Understood, WordCandidate, to_entry,
};
use crate::vocabulary::VocabularyEntry;

const IMAGE_STYLE: &str = "max-width: 100%; height: auto; border-radius: 10px";

type LiveIllustration =
    Illustration<SceneComposer<GeminiClient<HttpTransport>>, MangaRenderer<TextDetector>>;

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

    fn audio(&self, draft: &CardDraft) -> Result<Audio<GeminiClient<HttpTransport>>> {
        let item = self.catalog.item(draft.pair().learning())?;
        Ok(Audio::new(
            self.cell(draft).cache(),
            render_audio_prompt(item.prompt.as_str()),
            self.client()?,
        ))
    }

    fn illustration(&self, draft: &CardDraft) -> Result<LiveIllustration> {
        let item = self.catalog.item(draft.pair().learning())?;
        let client = self.client()?;
        Ok(Illustration::new(
            self.cell(draft).cache(),
            SceneComposer::new(client.clone(), item.prompt.as_str()),
            MangaRenderer::new(
                client,
                3,
                TextDetector::cached(60, item.ocr.as_str(), self.cache.clone()),
                BorderDetector::new(6, 240, 10),
            ),
        ))
    }

    fn generate_visual<F>(
        &self,
        draft: &CardDraft,
        artifact: &str,
        render: F,
    ) -> Result<ArtifactFile>
    where
        F: FnOnce(&LiveIllustration, &str, &str, &mut NoopProgress) -> Result<(String, bool)>,
    {
        let meta = draft
            .meta()
            .ok_or_else(|| anyhow!("meta must be ready before {artifact}"))?;
        let learning = draft.pair().learning();
        let illustration = self.illustration(draft)?;
        let mut progress = NoopProgress;
        let (filename, cached) = render(
            &illustration,
            meta.target_sentence(),
            learning,
            &mut progress,
        )?;
        let path = illustration.filepath(filename.as_str())?;
        Ok(artifact_file(filename, path, cached))
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
        self.client()?.generate_card_meta(term, understanding, pair)
    }
}

impl CardCorrection for LiveCardGenerator {
    fn correct_card(
        &self,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<CardRevision> {
        self.client()?.correct_card(draft, comment, pair)
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
            "scene",
            |illustration, sentence, target, progress| {
                illustration.scene_only(sentence, target, progress)
            },
        )
    }

    fn generate_picture(&self, draft: &CardDraft) -> Result<ArtifactFile> {
        self.generate_visual(
            draft,
            "picture",
            |illustration, sentence, target, progress| {
                illustration.picture_only(sentence, target, progress)
            },
        )
    }

    fn generate_sound(&self, draft: &CardDraft) -> Result<ArtifactFile> {
        let meta = draft
            .meta()
            .ok_or_else(|| anyhow!("meta must be ready before sound"))?;
        let audio = self.audio(draft)?;
        let (filename, cached) = audio.generate(meta.target_sentence())?;
        let path = audio.filepath(filename.as_str())?;
        Ok(artifact_file(filename, path, cached))
    }

    fn store_card_meta(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        meta: &CardMeta,
    ) -> Result<ArtifactFile> {
        let (filename, path, cached) = self.meta_cache().store(term, understanding, pair, meta)?;
        Ok(artifact_file(filename, path, cached))
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
        let entries: Vec<VocabularyEntry> = drafts
            .iter()
            .filter(|draft| draft.artifacts().all_ready())
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
        for draft in drafts.iter().filter(|draft| draft.artifacts().all_ready()) {
            let entry = to_entry(draft)?;
            let cell = self.cell(draft);
            let cache = cell.cache();
            let voice = cell.media_name("wav");
            let image = cell.media_name("jpg");
            let voice_path = cache.filepath(VOICE_FILE)?;
            let image_path = cache.filepath(ILLUSTRATION_FILE)?;
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

struct NoopProgress;

impl SceneProgress for NoopProgress {
    fn step(&mut self, _name: &str) {}

    fn done(&mut self, _name: &str, _label: &str, _path: Option<&Path>) {}
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

fn artifact_file(filename: String, path: PathBuf, cached: bool) -> ArtifactFile {
    let size = fs::metadata(&path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    ArtifactFile::new(filename, path, format_size(size), cached)
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
