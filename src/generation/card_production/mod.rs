//! Gemini-backed production of cached card artifacts.

mod attempt_archive;
mod cost_accounting;
mod gemini_media;
mod invalidation;
mod metadata;
mod picture_recovery;
mod picture_requests;
mod scene_attempt;
mod sound;
mod visual;

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;

use crate::application::{
    CardCorrection, CardMetaGeneration, CardProduction, GenerationCostLedger,
};
use crate::gemini::GeminiAccess;
use crate::languages::LanguageCatalog;
use crate::session::{
    ArtifactAttempt, ArtifactFile, CardDraft, CardMeta, CardRevision, GenerationCost, LanguagePair,
    SentenceLabelSelection,
};

use cost_accounting::CostAccounting;
use metadata::MetadataProduction;
use sound::SoundProduction;
use visual::VisualProduction;
#[cfg(test)]
use visual::production_renderer;

#[cfg(test)]
pub(crate) use invalidation::invalidate_card;
pub(crate) use invalidation::invalidate_draft;

/// Produces card metadata and media through focused Gemini adapters.
#[derive(Clone)]
pub(crate) struct GeminiCardProduction {
    metadata: MetadataProduction,
    visual: VisualProduction,
    sound: SoundProduction,
}

impl GeminiCardProduction {
    fn new(metadata: MetadataProduction, visual: VisualProduction, sound: SoundProduction) -> Self {
        Self {
            metadata,
            visual,
            sound,
        }
    }

    /// Compose metadata, visual, and sound production around one Gemini policy.
    #[must_use]
    pub(crate) fn from_gemini(
        cache: PathBuf,
        catalog: LanguageCatalog,
        access: GeminiAccess,
        ledger: Option<Arc<dyn GenerationCostLedger>>,
    ) -> Self {
        let costs = CostAccounting::new(ledger);
        Self::new(
            MetadataProduction::new(cache.clone(), access, costs.clone()),
            VisualProduction::new(cache.clone(), catalog, access, costs.clone()),
            SoundProduction::new(cache, catalog, access, costs),
        )
    }
}

impl CardMetaGeneration for GeminiCardProduction {
    fn generate_card_meta(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        request: Option<&SentenceLabelSelection>,
    ) -> Result<CardMeta> {
        self.metadata
            .generate(term, understanding, pair, request, None)
            .into_result()
            .map(|(meta, _file)| meta)
    }
}

impl CardCorrection for GeminiCardProduction {
    fn correct_card(
        &self,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<CardRevision> {
        self.metadata
            .correct(draft, comment, pair, None)
            .into_result()
    }

    fn correct_card_accounted(
        &self,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
    ) -> ArtifactAttempt<CardRevision> {
        self.metadata.correct(draft, comment, pair, None)
    }
}

impl CardProduction for GeminiCardProduction {
    fn generate_meta_in(
        &self,
        slot: usize,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        request: Option<&SentenceLabelSelection>,
    ) -> ArtifactAttempt<(CardMeta, Option<ArtifactFile>)> {
        self.metadata
            .generate(term, understanding, pair, request, Some(slot))
    }

    fn generate_draft_meta_in(
        &self,
        slot: usize,
        draft: &CardDraft,
    ) -> ArtifactAttempt<(CardRevision, Option<ArtifactFile>)> {
        if draft.rewrite().is_some() {
            return self.metadata.rewrite(draft, slot);
        }
        let term = draft.term().to_string();
        let understanding = draft.understanding().to_string();
        self.metadata
            .generate_draft(draft, draft.meta_request(), Some(slot))
            .map(|(meta, file)| (CardRevision::new(term, understanding, meta), file))
    }

    fn generate_scene_in(&self, slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
        self.visual.scene(slot, draft)
    }

    fn generate_picture_in(&self, slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
        self.visual.picture(slot, draft)
    }

    fn generate_sound_in(&self, slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
        self.sound.generate(slot, draft)
    }

    fn store_card_meta(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        meta: &CardMeta,
    ) -> Result<ArtifactFile> {
        self.metadata.store(term, understanding, pair, meta)
    }
}

#[cfg(test)]
pub(crate) use picture_requests::reserve_picture_request;
pub(crate) use picture_requests::restart_picture_request_series;

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

#[cfg(test)]
mod tests;
