//! Gemini metadata generation, correction, and stable cache persistence.

use std::path::PathBuf;

use anyhow::{Result, anyhow};

use super::artifact_file;
use super::cost_accounting::CostAccounting;
use super::invalidate_card;
use super::invalidation::{DependentGuards, clear_for_meta_refresh};
use crate::gemini::GeminiAccess;
use crate::generation::artifact_cache::{Cache, META_FILE, ROOT_STAGE_LOCK_TIMEOUT, RootStage};
use crate::generation::visual_revision;
use crate::session::{
    Artifact, ArtifactAttempt, ArtifactFile, CardCell, CardDraft, CardMeta, CardMetaCache,
    CardRevision, LanguagePair, SentenceLabelSelection,
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
        request: Option<&SentenceLabelSelection>,
        slot: Option<usize>,
    ) -> ArtifactAttempt<(CardMeta, Option<ArtifactFile>)> {
        let cache = CardCell::new(self.cache.clone(), pair, term, understanding).cache();
        let visual = match cache.visual(visual_revision()) {
            Ok(visual) => visual,
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        };
        let _meta = match cache.hold_root_stage(RootStage::Meta, ROOT_STAGE_LOCK_TIMEOUT) {
            Ok(meta) => meta,
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        };
        match self.meta_cache().load_current(term, understanding, pair) {
            Ok(Some(meta)) if request.is_none_or(|request| request.pinned().is_empty()) => {
                let result = self
                    .cached_file(term, understanding, pair)
                    .map(|file| (meta, Some(file)));
                return ArtifactAttempt::unmetered(result);
            }
            Ok(Some(meta)) => {
                if let Some(meta) = requested_cached(meta, request) {
                    let result = self
                        .replace_cached(term, understanding, pair, &meta)
                        .map(|file| (meta, Some(file)));
                    return ArtifactAttempt::unmetered(result);
                }
            }
            Ok(None) => {}
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        }
        let _dependents = match DependentGuards::hold(&cache, &visual) {
            Ok(dependents) => dependents,
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        };
        let client = match self.access.client() {
            Ok(client) => client,
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        };
        let costs = self.costs.recorder(cache.clone(), Artifact::Meta, slot);
        let result = client
            .generate_card_meta_observed(term, understanding, pair, request, |record| {
                costs.push(record)
            })
            .and_then(|meta| {
                self.replace_generated(&cache, &visual, term, understanding, pair, &meta)
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
        let visual = cache.visual(visual_revision())?;
        let _meta = cache.hold_root_stage(RootStage::Meta, ROOT_STAGE_LOCK_TIMEOUT)?;
        if self
            .meta_cache()
            .load_current(term, understanding, pair)?
            .is_some()
        {
            return self.cached_file(term, understanding, pair);
        }
        let _dependents = DependentGuards::hold(&cache, &visual)?;
        self.replace_generated(&cache, &visual, term, understanding, pair, meta)
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

    pub(super) fn replace_generated(
        &self,
        cache: &Cache,
        visual: &Cache,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        meta: &CardMeta,
    ) -> Result<ArtifactFile> {
        clear_for_meta_refresh(cache, visual)?;
        self.replace_meta(term, understanding, pair, meta, false)
    }

    fn replace_cached(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        meta: &CardMeta,
    ) -> Result<ArtifactFile> {
        self.replace_meta(term, understanding, pair, meta, true)
    }

    fn replace_meta(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        meta: &CardMeta,
        cached: bool,
    ) -> Result<ArtifactFile> {
        let (filename, path) = self.meta_cache().replace(term, understanding, pair, meta)?;
        Ok(artifact_file(filename, path, cached, None))
    }

    fn cached_file(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
    ) -> Result<ArtifactFile> {
        let cache = CardCell::new(self.cache.clone(), pair, term, understanding).cache();
        Ok(artifact_file(
            String::from(META_FILE),
            cache.filepath(META_FILE)?,
            true,
            None,
        ))
    }

    fn meta_cache(&self) -> CardMetaCache {
        CardMetaCache::new(self.cache.clone())
    }
}

fn requested_cached(meta: CardMeta, request: Option<&SentenceLabelSelection>) -> Option<CardMeta> {
    let request = request?;
    let labels = meta.sentence_labels()?.clone();
    let matches = request.pinned().iter().all(|axis| {
        request
            .token(axis)
            .is_some_and(|token| labels.token(axis) == Some(token))
    });
    matches.then(|| meta.with_sentence_labels(request.reconciled(labels)))
}
