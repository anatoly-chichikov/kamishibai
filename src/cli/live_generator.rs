//! Live implementation of the UI-shaped card workflow.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use time::OffsetDateTime;
use time::format_description::parse as parse_time;

use super::card_workflow::{CardGeneration, DeckPublishProgress, DeckPublishing, KeyValidation};
use crate::anki::{CardModel, StableId, VocabularyDeck, VocabularyNote};
use crate::config::default_store;
use crate::gemini::{GeminiClient, HttpTransport};
use crate::generation::artifact_cache::Cache;
use crate::generation::manga::{
    BorderDetector, Illustration, MangaRenderer, Progress as SceneProgress, TextDetector,
};
use crate::generation::speech::Audio;
use crate::generation::{SceneComposer, render_audio_prompt};
use crate::languages::{LanguageCatalog, catalog, naming};
use crate::report::{CardSheet, Thumbnail};
use crate::runtime::locations::{LocationArgs, Locations, SystemContext};
use crate::session::{
    ArtifactFile, BulkCorrection, CachedUnderstanding, CardCorrection, CardDraft, CardMeta,
    CardMetaCache, CardMetaGeneration, CardRevision, LanguagePair, RawInputBatch, Understanding,
    Understood, WordCandidate, to_entry,
};
use crate::tui::BusyKind;
use crate::vocabulary::VocabularyEntry;

const IMAGE_STYLE: &str = "max-width: 100%; height: auto; border-radius: 10px";

type LiveIllustration =
    Illustration<SceneComposer<GeminiClient<HttpTransport>>, MangaRenderer<TextDetector>>;

/// Live card generator backed by Gemini and the on-disk cache.
#[derive(Clone)]
pub(super) struct LiveCardGenerator {
    cache: PathBuf,
    output: PathBuf,
    catalog: LanguageCatalog,
}

impl LiveCardGenerator {
    /// Build a live card generator from on-disk locations.
    pub(super) fn new(cache: PathBuf, output: PathBuf) -> Self {
        Self {
            cache,
            output,
            catalog: catalog(),
        }
    }

    fn client(&self) -> Result<GeminiClient<HttpTransport>> {
        let saved_key = default_store(&SystemContext)
            .ok()
            .and_then(|store| store.read().ok())
            .and_then(|prefs| prefs.api_key);
        GeminiClient::from_saved(saved_key.as_deref())
    }

    fn meta_cache(&self) -> CardMetaCache {
        CardMetaCache::new(self.cache.clone())
    }

    fn audio_for(&self, target_lang: &str) -> Result<Audio<GeminiClient<HttpTransport>>> {
        let item = self.catalog.item(target_lang)?;
        Ok(Audio::new(
            Cache::new(item.audio_cache.as_str(), self.cache.clone()),
            render_audio_prompt(item.prompt.as_str()),
            self.client()?,
        ))
    }

    fn illustration_for(&self, target_lang: &str) -> Result<LiveIllustration> {
        let item = self.catalog.item(target_lang)?;
        let client = self.client()?;
        Ok(Illustration::new(
            Cache::new(item.image_cache.as_str(), self.cache.clone()),
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
        let target = draft.pair().target();
        let illustration = self.illustration_for(target)?;
        let mut progress = NoopProgress;
        let (filename, cached) =
            render(&illustration, meta.target_sentence(), target, &mut progress)?;
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
        candidates: &[WordCandidate],
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<Vec<WordCandidate>> {
        self.client()?.correct_bulk(candidates, comment, pair)
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
        let audio = self.audio_for(draft.pair().target())?;
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
        progress: &DeckPublishProgress,
    ) -> Result<(String, String, String)> {
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
            Vec::<PathBuf>::new(),
        );
        let mut report = CardSheet::new();
        for draft in drafts.iter().filter(|draft| draft.artifacts().all_ready()) {
            let entry = to_entry(draft)?;
            let audio_file = draft
                .artifacts()
                .sound()
                .file()
                .ok_or_else(|| anyhow!("sound artifact missing for {}", draft.term()))?;
            let picture_file = draft
                .artifacts()
                .picture()
                .file()
                .ok_or_else(|| anyhow!("picture artifact missing for {}", draft.term()))?;
            let audio_path = self
                .audio_for(draft.pair().target())?
                .filepath(audio_file.name())?;
            let picture_path = self
                .illustration_for(draft.pair().target())?
                .filepath(picture_file.name())?;
            container.attach(audio_path);
            container.attach(picture_path.clone());
            container.add(
                &entry,
                format!("[sound:{}]", audio_file.name()).as_str(),
                format!("<img src='{}' style='{IMAGE_STYLE}'>", picture_file.name()).as_str(),
            );
            report.append(&entry, Some(picture_path));
        }
        let stamp = release_stamp()?;
        let apkg = self
            .output
            .join(format!("{}_{}.apkg", decknaming.prefix, stamp));
        container.save(&apkg)?;
        progress.report_phase(BusyKind::PublishingReport);
        let pdf = self
            .output
            .join(format!("{}_{}.pdf", decknaming.prefix, stamp));
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
