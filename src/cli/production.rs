//! Production implementation of the CLI pass contracts.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use time::OffsetDateTime;
use time::format_description::parse as parse_time;

use super::passes::{MediaPasses, PublishProgress};
use crate::anki::{CardModel, StableId, VocabularyDeck, VocabularyNote};
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
    ArtifactFile, BulkCorrection, CachedUnderstanding, CardBody, CardBodyCache, CardBodyGeneration,
    CardCorrection, CardDraft, CardRevision, LanguagePair, RawInputBatch, Understanding,
    Understood, WordCandidate, to_entry,
};
use crate::tui::BusyKind;
use crate::vocabulary::VocabularyEntry;

const IMAGE_STYLE: &str = "max-width: 100%; height: auto; border-radius: 10px";

/// Production media and lifecycle passes backed by Gemini and the on-disk cache.
#[derive(Clone)]
pub(super) struct ProductionPasses {
    client: GeminiClient<HttpTransport>,
    cache: PathBuf,
    output: PathBuf,
    catalog: LanguageCatalog,
}

impl ProductionPasses {
    /// Build production passes from a live Gemini client and on-disk locations.
    pub(super) fn new(
        client: GeminiClient<HttpTransport>,
        cache: PathBuf,
        output: PathBuf,
    ) -> Self {
        Self {
            client,
            cache,
            output,
            catalog: catalog(),
        }
    }

    fn body_cache(&self) -> CardBodyCache {
        CardBodyCache::new(self.cache.clone())
    }

    fn audio_for(&self, target_lang: &str) -> Result<Audio<GeminiClient<HttpTransport>>> {
        let item = self.catalog.item(target_lang)?;
        Ok(Audio::new(
            Cache::new(item.audio_cache.as_str(), self.cache.clone()),
            render_audio_prompt(item.prompt.as_str()),
            self.client.clone(),
        ))
    }

    fn illustration_for(
        &self,
        target_lang: &str,
    ) -> Result<Illustration<SceneComposer<GeminiClient<HttpTransport>>, MangaRenderer<TextDetector>>>
    {
        let item = self.catalog.item(target_lang)?;
        let client = self.client.clone();
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
}

impl Understanding for ProductionPasses {
    fn understand(&self, raw: &RawInputBatch, my: &str) -> Result<Understood> {
        CachedUnderstanding::new(self.client.clone(), self.cache.clone()).understand(raw, my)
    }
}

impl BulkCorrection for ProductionPasses {
    fn correct_bulk(
        &self,
        candidates: &[WordCandidate],
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<Vec<WordCandidate>> {
        self.client.correct_bulk(candidates, comment, pair)
    }
}

impl CardBodyGeneration for ProductionPasses {
    fn generate_card_body(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
    ) -> Result<CardBody> {
        if let Some(body) = self.body_cache().load(term, understanding, pair)? {
            return Ok(body);
        }
        self.client.generate_card_body(term, understanding, pair)
    }
}

impl CardCorrection for ProductionPasses {
    fn correct_card(
        &self,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<CardRevision> {
        self.client.correct_card(draft, comment, pair)
    }
}

impl MediaPasses for ProductionPasses {
    fn produce_scene(&self, draft: &CardDraft) -> Result<ArtifactFile> {
        let body = draft
            .body()
            .ok_or_else(|| anyhow!("body must be ready before scene"))?;
        let illustration = self.illustration_for(draft.pair().target())?;
        let mut progress = NoopProgress;
        let (filename, cached) = illustration.scene_only(
            body.target_sentence(),
            draft.pair().target(),
            &mut progress,
        )?;
        let path = illustration.filepath(filename.as_str())?;
        let size = fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        Ok(ArtifactFile::new(filename, path, format_size(size), cached))
    }

    fn produce_picture(&self, draft: &CardDraft) -> Result<ArtifactFile> {
        let body = draft
            .body()
            .ok_or_else(|| anyhow!("body must be ready before picture"))?;
        let illustration = self.illustration_for(draft.pair().target())?;
        let mut progress = NoopProgress;
        let (filename, cached) = illustration.picture_only(
            body.target_sentence(),
            draft.pair().target(),
            &mut progress,
        )?;
        let path = illustration.filepath(filename.as_str())?;
        let size = fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        Ok(ArtifactFile::new(filename, path, format_size(size), cached))
    }

    fn produce_sound(&self, draft: &CardDraft) -> Result<ArtifactFile> {
        let body = draft
            .body()
            .ok_or_else(|| anyhow!("body must be ready before sound"))?;
        let audio = self.audio_for(draft.pair().target())?;
        let (filename, cached) = audio.generate(body.target_sentence())?;
        let path = audio.filepath(filename.as_str())?;
        let size = fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        Ok(ArtifactFile::new(filename, path, format_size(size), cached))
    }

    fn persist_body(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        body: &CardBody,
    ) -> Result<ArtifactFile> {
        let (filename, path, cached) = self.body_cache().store(term, understanding, pair, body)?;
        let size = fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        Ok(ArtifactFile::new(filename, path, format_size(size), cached))
    }

    fn publish(
        &self,
        drafts: &[CardDraft],
        progress: &PublishProgress,
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

fn format_unit(bytes: u64, unit: u64, suffix: &str) -> String {
    let whole = bytes / unit;
    let tenth = bytes % unit * 10 / unit;
    format!("{whole}.{tenth} {suffix}")
}

fn release_stamp() -> Result<String> {
    Ok(OffsetDateTime::now_utc()
        .format(parse_time("[year]-[month]-[day]_[hour][minute][second]")?.as_slice())?)
}
