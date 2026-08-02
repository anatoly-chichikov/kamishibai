//! Gemini metadata generation, correction, and stable cache persistence.

use std::fs;
use std::path::PathBuf;

use anyhow::{Result, anyhow};

use super::artifact_file;
use super::cost_accounting::CostAccounting;
use super::invalidate_card;
use crate::gemini::GeminiAccess;
use crate::generation::artifact_cache::{
    Cache, ROOT_STAGE_LOCK_TIMEOUT, RootStage, VOICE_COST_FILE, VOICE_FILE,
};
use crate::session::{
    Artifact, ArtifactAttempt, ArtifactFile, CardCell, CardDraft, CardMeta, CardMetaCache,
    CardRevision, LanguagePair,
};

/// Produces and stores the metadata that identifies one card.
#[derive(Clone)]
pub(super) struct MetadataProduction {
    cache: PathBuf,
    access: GeminiAccess,
    costs: CostAccounting,
}

impl MetadataProduction {
    /// Bind metadata production to Gemini, cache, and workflow accounting.
    #[must_use]
    pub(super) fn new(cache: PathBuf, access: GeminiAccess, costs: CostAccounting) -> Self {
        Self {
            cache,
            access,
            costs,
        }
    }

    /// Generate or load one metadata document with optional slot attribution.
    pub(super) fn generate(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        slot: Option<usize>,
    ) -> ArtifactAttempt<(CardMeta, Option<ArtifactFile>)> {
        let cache = CardCell::new(self.cache.clone(), pair, term, understanding).cache();
        let _guard = match cache.hold_root_stage(RootStage::Meta, ROOT_STAGE_LOCK_TIMEOUT) {
            Ok(guard) => guard,
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        };
        match self.meta_cache().load_current(term, understanding, pair) {
            Ok(Some(meta)) => {
                let result = self
                    .store_unlocked(term, understanding, pair, &meta)
                    .map(|file| (meta, Some(file)));
                return ArtifactAttempt::unmetered(result);
            }
            Ok(None) => {}
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        }
        let client = match self.access.client() {
            Ok(client) => client,
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        };
        let costs = self.costs.recorder(cache, Artifact::Meta, slot);
        let result = client
            .generate_card_meta_observed(term, understanding, pair, |record| costs.push(record))
            .and_then(|meta| {
                self.store_unlocked(term, understanding, pair, &meta)
                    .map(|file| (meta, Some(file)))
            });
        match costs.cumulative(false) {
            Ok(cost) => ArtifactAttempt::new(result, cost),
            Err(error) => ArtifactAttempt::unmetered(Err(error)),
        }
    }

    /// Correct one card and return the exact request spend.
    pub(super) fn correct(
        &self,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
        slot: Option<usize>,
    ) -> ArtifactAttempt<CardRevision> {
        let client = match self.access.client() {
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
        let costs = self.costs.recorder(cache, Artifact::Meta, slot);
        let result =
            client.correct_card_observed(draft, comment, pair, |cost| costs.push_correction(cost));
        match costs.current(false) {
            Ok(delta) => ArtifactAttempt::new(result, delta),
            Err(error) => ArtifactAttempt::unmetered(Err(error)),
        }
    }

    /// Rewrite and replace one draft through the metadata artifact retry boundary.
    pub(super) fn rewrite(
        &self,
        draft: &CardDraft,
        slot: usize,
    ) -> ArtifactAttempt<(CardRevision, Option<ArtifactFile>)> {
        let Some(rewrite) = draft.rewrite() else {
            return ArtifactAttempt::unmetered(Err(anyhow!(
                "metadata rewrite requires a queued card rewrite"
            )));
        };
        let client = match self.access.client() {
            Ok(client) => client,
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        };
        let cache = CardCell::new(
            self.cache.clone(),
            draft.pair(),
            draft.term(),
            draft.understanding(),
        )
        .cache();
        let costs = self.costs.recorder(cache, Artifact::Meta, Some(slot));
        let result = client
            .correct_card_observed(draft, rewrite.note(), draft.pair(), |cost| {
                costs.push_correction(cost)
            })
            .and_then(|revision| {
                self.replace(draft, &revision)
                    .map(|file| (revision, Some(file)))
            });
        match costs.current(false) {
            Ok(delta) => ArtifactAttempt::new(result, delta),
            Err(error) => ArtifactAttempt::unmetered(Err(error)),
        }
    }

    /// Persist supplied metadata under the stable card identity.
    pub(super) fn store(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        meta: &CardMeta,
    ) -> Result<ArtifactFile> {
        let cache = CardCell::new(self.cache.clone(), pair, term, understanding).cache();
        let _guard = cache.hold_root_stage(RootStage::Meta, ROOT_STAGE_LOCK_TIMEOUT)?;
        self.store_unlocked(term, understanding, pair, meta)
    }

    fn store_unlocked(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        meta: &CardMeta,
    ) -> Result<ArtifactFile> {
        let cache = CardCell::new(self.cache.clone(), pair, term, understanding).cache();
        let refresh = self
            .meta_cache()
            .load_current(term, understanding, pair)?
            .is_none();
        let _voice = refresh
            .then(|| cache.hold_root_stage(RootStage::Voice, ROOT_STAGE_LOCK_TIMEOUT))
            .transpose()?;
        if refresh {
            remove_cached(&cache, VOICE_FILE)?;
            remove_cached(&cache, VOICE_COST_FILE)?;
        }
        let (filename, path, cached) = self.meta_cache().store(term, understanding, pair, meta)?;
        Ok(artifact_file(filename, path, cached, None))
    }

    fn replace(&self, draft: &CardDraft, revision: &CardRevision) -> Result<ArtifactFile> {
        invalidate_card(
            self.cache.as_path(),
            draft.pair(),
            draft.term(),
            draft.understanding(),
            false,
            true,
        )?;
        self.store(
            revision.term(),
            revision.understanding(),
            draft.pair(),
            revision.meta(),
        )
    }

    fn meta_cache(&self) -> CardMetaCache {
        CardMetaCache::new(self.cache.clone())
    }
}

fn remove_cached(cache: &Cache, filename: &str) -> Result<()> {
    let path = cache.path().join(filename);
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
