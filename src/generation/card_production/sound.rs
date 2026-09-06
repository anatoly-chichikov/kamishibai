//! Native-speaker audio production for one completed card.

use std::path::PathBuf;

use anyhow::{Result, anyhow};

use super::artifact_file;
use super::cost_accounting::{CostAccounting, CostRecorder};
use super::gemini_media::MeteredGemini;
use crate::gemini::GeminiAccess;
use crate::generation::artifact_cache::{ROOT_STAGE_LOCK_TIMEOUT, RootStage, VOICE_FILE};
use crate::generation::render_audio_prompt;
use crate::generation::speech::Audio;
use crate::languages::LanguageCatalog;
use crate::session::{Artifact, ArtifactAttempt, ArtifactFile, CardCell, CardDraft};

/// Produces and caches the pronunciation audio for a card.
#[derive(Clone)]
pub(super) struct SoundProduction {
    cache: PathBuf,
    catalog: LanguageCatalog,
    access: GeminiAccess,
    costs: CostAccounting,
}

impl SoundProduction {
    /// Bind sound production to languages, Gemini, cache, and accounting.
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
            costs,
        }
    }

    /// Generate or load sound attributed to one stable card slot.
    pub(super) fn generate(&self, slot: usize, draft: &CardDraft) -> ArtifactAttempt<ArtifactFile> {
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
        let costs = self
            .costs
            .recorder(cache.clone(), Artifact::Sound, Some(slot));
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

    fn cell(&self, draft: &CardDraft) -> CardCell {
        CardCell::for_draft(self.cache.clone(), draft)
    }

    fn audio(&self, draft: &CardDraft, costs: CostRecorder) -> Result<Audio<MeteredGemini>> {
        let item = self.catalog.item(draft.pair().learning())?;
        Ok(Audio::new(
            self.cell(draft).cache(),
            render_audio_prompt(item.prompt.as_str()),
            MeteredGemini::new(self.access.client()?, costs),
        ))
    }
}
