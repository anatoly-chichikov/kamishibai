//! Gemini metadata generation, correction, and stable cache persistence.

use std::path::PathBuf;

use anyhow::{Result, anyhow};

use super::artifact_file;
use super::cost_accounting::CostAccounting;
use super::invalidate_draft;
use super::invalidation::{DependentGuards, clear_for_meta_refresh};
use crate::gemini::GeminiAccess;
use crate::generation::artifact_cache::{Cache, META_FILE, ROOT_STAGE_LOCK_TIMEOUT, RootStage};
use crate::generation::visual_revision;
use crate::session::{
    Artifact, ArtifactAttempt, ArtifactFile, AxisSet, CardCell, CardDraft, CardMeta, CardMetaCache,
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
        let draft = CardDraft::new(term, understanding, pair.clone());
        self.generate_draft(&draft, request, slot)
    }

    /// Generate or load one metadata document under its reviewed-sense identity.
    pub(super) fn generate_draft(
        &self,
        draft: &CardDraft,
        request: Option<&SentenceLabelSelection>,
        slot: Option<usize>,
    ) -> ArtifactAttempt<(CardMeta, Option<ArtifactFile>)> {
        let cell = CardCell::for_draft(self.cache.clone(), draft);
        let cache = cell.cache();
        let visual = match cache.visual(visual_revision()) {
            Ok(visual) => visual,
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        };
        let _meta = match cache.hold_root_stage(RootStage::Meta, ROOT_STAGE_LOCK_TIMEOUT) {
            Ok(meta) => meta,
            Err(error) => return ArtifactAttempt::unmetered(Err(error)),
        };
        match self.meta_cache().load_current_at(&cell) {
            Ok(Some(meta)) if request.is_none_or(|request| request.pinned().is_empty()) => {
                let result = match meta.sentence_labels().cloned() {
                    Some(labels) if !labels.pinned().is_empty() || !labels.approx().is_empty() => {
                        let labels = labels.with_axis_state(AxisSet::default(), AxisSet::default());
                        let meta = meta.with_sentence_labels(labels);
                        self.replace_cached_at(&cell, draft, &meta)
                            .map(|file| (meta, Some(file)))
                    }
                    _ => self.cached_file_at(&cell).map(|file| (meta, Some(file))),
                };
                return ArtifactAttempt::unmetered(result);
            }
            Ok(Some(meta)) => {
                if let Some(meta) = requested_cached(meta, request) {
                    let result = self
                        .replace_cached_at(&cell, draft, &meta)
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
            .generate_draft_meta_observed(draft, request, |record| costs.push(record))
            .and_then(|meta| {
                self.replace_generated_at(&cell, &cache, &visual, draft, &meta)
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
        let cache = CardCell::for_draft(self.cache.clone(), draft).cache();
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
        let cache = CardCell::for_draft(self.cache.clone(), draft).cache();
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
        if self.meta_cache().matches(term, understanding, pair, meta)? {
            return self.cached_file(term, understanding, pair);
        }
        let _dependents = DependentGuards::hold(&cache, &visual)?;
        self.replace_generated(&cache, &visual, term, understanding, pair, meta)
    }

    fn replace(&self, draft: &CardDraft, revision: &CardRevision) -> Result<ArtifactFile> {
        invalidate_draft(self.cache.as_path(), draft, false, true)?;
        let revised = draft.clone().with_revision(revision.clone(), None);
        let cell = CardCell::for_draft(self.cache.clone(), &revised);
        let cache = cell.cache();
        let visual = cache.visual(visual_revision())?;
        let _meta = cache.hold_root_stage(RootStage::Meta, ROOT_STAGE_LOCK_TIMEOUT)?;
        let _dependents = DependentGuards::hold(&cache, &visual)?;
        self.replace_generated_at(&cell, &cache, &visual, &revised, revision.meta())
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
        let cell = CardCell::new(self.cache.clone(), pair, term, understanding);
        clear_for_meta_refresh(cache, visual)?;
        self.replace_meta_at(&cell, term, understanding, pair, meta, false)
    }

    fn replace_generated_at(
        &self,
        cell: &CardCell,
        cache: &Cache,
        visual: &Cache,
        draft: &CardDraft,
        meta: &CardMeta,
    ) -> Result<ArtifactFile> {
        clear_for_meta_refresh(cache, visual)?;
        self.replace_meta_at(
            cell,
            draft.term(),
            draft.understanding(),
            draft.pair(),
            meta,
            false,
        )
    }

    fn replace_cached_at(
        &self,
        cell: &CardCell,
        draft: &CardDraft,
        meta: &CardMeta,
    ) -> Result<ArtifactFile> {
        self.replace_meta_at(
            cell,
            draft.term(),
            draft.understanding(),
            draft.pair(),
            meta,
            true,
        )
    }

    fn replace_meta_at(
        &self,
        cell: &CardCell,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        meta: &CardMeta,
        cached: bool,
    ) -> Result<ArtifactFile> {
        let (filename, path) =
            self.meta_cache()
                .replace_at(cell, term, understanding, pair, meta)?;
        Ok(artifact_file(filename, path, cached, None))
    }

    fn cached_file(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
    ) -> Result<ArtifactFile> {
        let cell = CardCell::new(self.cache.clone(), pair, term, understanding);
        self.cached_file_at(&cell)
    }

    fn cached_file_at(&self, cell: &CardCell) -> Result<ArtifactFile> {
        let cache = cell.cache();
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
    let approx = request
        .pinned()
        .iter()
        .try_fold(AxisSet::default(), |approx, axis| {
            let token = request.token(axis)?;
            if labels.approx().contains(axis) {
                match labels.recorded_request_token(axis) {
                    Some(recorded) if recorded == token => {
                        return Some(approx.including(axis));
                    }
                    None => return None,
                    Some(_) => {}
                }
            }
            (labels.token(axis) == Some(token)).then_some(approx)
        })?;
    let labels = labels.with_axis_state(request.pinned().clone(), approx);
    Some(meta.with_sentence_labels(request.reconciled(labels)))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::TempDir;

    use super::*;
    use crate::generation::artifact_cache::{
        ILLUSTRATION_FILE, META_COST_FILE, SCENE_FILE, VOICE_FILE,
    };

    fn production(root: &Path) -> MetadataProduction {
        MetadataProduction::new(
            root.to_path_buf(),
            GeminiAccess::unavailable(),
            CostAccounting::new(None),
        )
    }

    fn meta(sentence: &str) -> CardMeta {
        CardMeta::new(
            "ka.naʁ",
            "sample",
            "duck",
            7,
            "The duck swims",
            "duck",
            "a water bird",
            "A bird near the pond",
            sentence,
        )
    }

    #[test]
    fn supplied_metadata_cannot_be_replaced_by_an_older_cached_sentence() {
        let root = TempDir::new().expect("temporary cache must exist");
        let production = production(root.path());
        let pair = LanguagePair::new("FR", "EN");
        let revised = meta("Le canard traverse le jardin");
        production
            .store("canard", "a duck", &pair, &meta("Le canard nage"))
            .expect("original metadata must store");
        production
            .store("canard", "a duck", &pair, &revised)
            .expect("revised metadata must store");
        assert_eq!(
            production
                .meta_cache()
                .load("canard", "a duck", &pair)
                .unwrap(),
            Some(revised),
            "supplied metadata was silently replaced by an older cached sentence"
        );
    }

    #[test]
    fn replaced_metadata_cannot_reuse_media_from_the_previous_sentence() {
        let root = TempDir::new().expect("temporary cache must exist");
        let production = production(root.path());
        let pair = LanguagePair::new("FR", "EN");
        production
            .store("canard", "a duck", &pair, &meta("Le canard nage"))
            .expect("original metadata must store");
        let cache = CardCell::new(root.path(), &pair, "canard", "a duck").cache();
        let visual = cache.visual(visual_revision()).unwrap();
        let paths = [
            cache.filepath(VOICE_FILE).unwrap(),
            visual.filepath(SCENE_FILE).unwrap(),
            visual.filepath(ILLUSTRATION_FILE).unwrap(),
        ];
        for path in &paths {
            fs::write(path, b"old sentence artifact").unwrap();
        }
        let cost = cache.filepath(META_COST_FILE).unwrap();
        fs::write(&cost, b"previous spend").unwrap();
        production
            .store(
                "canard",
                "a duck",
                &pair,
                &meta("Le canard traverse le jardin"),
            )
            .expect("revised metadata must store");
        assert_eq!(
            (paths.map(|path| path.exists()), fs::read(cost).unwrap()),
            ([false; 3], b"previous spend".to_vec()),
            "metadata replacement retained outdated media or discarded previous spending"
        );
    }

    #[test]
    fn identical_supplied_metadata_cannot_discard_existing_media() {
        let root = TempDir::new().expect("temporary cache must exist");
        let production = production(root.path());
        let pair = LanguagePair::new("FR", "EN");
        let meta = meta("Le canard nage").with_source_context(
            "**Meaning.**\n- **a duck**\n- a newspaper hoax\nrecap: one is a bird and the other is a hoax.\n\n**Usage.**\nA bird near the pond.",
        );
        production
            .store("canard", "a duck", &pair, &meta)
            .expect("original metadata must store");
        let cache = CardCell::new(root.path(), &pair, "canard", "a duck").cache();
        let audio = cache.filepath(VOICE_FILE).unwrap();
        fs::write(&audio, b"current sentence audio").unwrap();
        production
            .store("canard", "a duck", &pair, &meta)
            .expect("identical metadata must remain cached");
        assert_eq!(
            fs::read(audio).unwrap(),
            b"current sentence audio",
            "storing identical metadata discarded valid audio"
        );
    }
}
