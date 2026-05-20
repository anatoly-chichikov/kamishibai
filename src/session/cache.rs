//! Persistent session caches for reviewed input and card meta.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::generation::artifact_cache::Cache;
use crate::languages::catalog;

use super::{
    CardMeta, LanguagePair, RawInputBatch, ScriptDetection, TargetDetection, TargetGuess,
    Understanding, Understood, WordCandidate,
};

const UNDERSTANDING_CACHE: &str = "understanding-v1";
const UNDERSTANDING_VERSION: &str = "understanding-v1";
const META_VERSION: &str = "meta-v2";

/// Caching decorator for the first-pass understanding contract.
#[derive(Clone, Debug)]
pub struct CachedUnderstanding<T> {
    inner: T,
    root: PathBuf,
}

impl<T> CachedUnderstanding<T> {
    /// Create one understanding cache rooted in the shared application cache.
    pub fn new(inner: T, root: impl Into<PathBuf>) -> Self {
        Self {
            inner,
            root: root.into(),
        }
    }
}

impl<T> Understanding for CachedUnderstanding<T>
where
    T: Understanding,
{
    /// Normalise raw words into reviewed rows, reusing a prior result for the same input.
    fn understand(&self, raw: &RawInputBatch, my: &str) -> Result<Understood> {
        let cache = Cache::new(UNDERSTANDING_CACHE, self.root.clone());
        let target = ScriptDetection.detect(raw.text(), &catalog())?;
        let entries = normalized_entries(raw);
        let mut merged = vec![None; entries.len()];
        let mut misses = Vec::new();
        let mut guess = None;
        for (index, entry) in entries.iter().enumerate() {
            let filename = self.entry_filename(entry, my, target.code());
            if cache.exists(filename.as_str()) {
                let record: EntryRecord = read_json(&cache, filename.as_str())?;
                guess = guess.or_else(|| Some(record.guess()));
                merged[index] = Some(record.candidate());
            } else {
                misses.push(EntryMiss::new(index, entry));
            }
        }
        if !misses.is_empty() {
            let (detected, candidates) =
                self.missing(&cache, my, target.code(), raw, entries.as_slice(), misses)?;
            guess = Some(detected);
            for (index, candidate) in candidates {
                merged[index] = Some(candidate);
            }
        }
        let candidates = merged
            .into_iter()
            .enumerate()
            .map(|(index, candidate)| {
                candidate.ok_or_else(|| anyhow!("understanding cache left entry {index} empty"))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Understood::new(
            guess.unwrap_or_else(|| TargetGuess::new(target.code(), target.confident())),
            candidates,
        ))
    }
}

impl<T> CachedUnderstanding<T> {
    fn entry_filename(&self, entry: &str, my: &str, target: &str) -> String {
        let key = format!("{UNDERSTANDING_VERSION}\0{my}\0{target}\0{entry}");
        format!("{}.json", digest(key.as_str()))
    }

    fn missing(
        &self,
        cache: &Cache,
        my: &str,
        target: &str,
        raw: &RawInputBatch,
        entries: &[String],
        misses: Vec<EntryMiss>,
    ) -> Result<(TargetGuess, Vec<(usize, WordCandidate)>)>
    where
        T: Understanding,
    {
        let missing_raw = RawInputBatch::new(
            misses
                .iter()
                .map(EntryMiss::entry)
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let understood = self.inner.understand(&missing_raw, my)?;
        if understood.candidates().len() != misses.len() {
            let full = self.inner.understand(raw, my)?;
            self.store_entries(cache, my, target, entries, &full)?;
            return Ok(indexed(full));
        }
        for (miss, candidate) in misses.iter().zip(understood.candidates()) {
            let filename = self.entry_filename(miss.entry(), my, target);
            write_json(
                cache,
                filename.as_str(),
                &EntryRecord::from_candidate(understood.guess(), candidate),
            )?;
        }
        Ok(indexed_missing(understood, misses))
    }

    fn store_entries(
        &self,
        cache: &Cache,
        my: &str,
        target: &str,
        entries: &[String],
        understood: &Understood,
    ) -> Result<()> {
        if entries.len() != understood.candidates().len() {
            return Ok(());
        }
        for (entry, candidate) in entries.iter().zip(understood.candidates()) {
            let filename = self.entry_filename(entry, my, target);
            write_json(
                cache,
                filename.as_str(),
                &EntryRecord::from_candidate(understood.guess(), candidate),
            )?;
        }
        Ok(())
    }
}

/// Persistent cache for Pro card-meta payloads.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CardMetaCache {
    root: PathBuf,
}

impl CardMetaCache {
    /// Create one card-meta cache rooted in the shared application cache.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Return a cached card meta for the exact term, understanding, and language pair.
    pub fn load(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
    ) -> Result<Option<CardMeta>> {
        let cache = self.meta_cache(pair);
        let filename = self.filename(term, understanding, pair);
        if !cache.exists(filename.as_str()) {
            return Ok(None);
        }
        let record: MetaRecord = read_json(&cache, filename.as_str())?;
        Ok(Some(record.meta()))
    }

    /// Persist one card meta and return filename, path, and whether it already existed.
    pub fn store(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        meta: &CardMeta,
    ) -> Result<(String, PathBuf, bool)> {
        let cache = self.meta_cache(pair);
        let filename = self.filename(term, understanding, pair);
        let cached = cache.exists(filename.as_str());
        if !cached {
            write_json(
                &cache,
                filename.as_str(),
                &MetaRecord::from_meta(term, understanding, pair, meta),
            )?;
        }
        let path = cache.filepath(filename.as_str())?;
        Ok((filename, path, cached))
    }

    fn meta_cache(&self, pair: &LanguagePair) -> Cache {
        Cache::new(format!("meta-{}", pair.target()), self.root.clone())
    }

    fn filename(&self, term: &str, understanding: &str, pair: &LanguagePair) -> String {
        let key = format!(
            "{META_VERSION}\0{}\0{}\0{term}\0{understanding}",
            pair.target(),
            pair.support()
        );
        format!("{}.json", digest(key.as_str()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EntryMiss {
    index: usize,
    entry: String,
}

impl EntryMiss {
    fn new(index: usize, entry: &str) -> Self {
        Self {
            index,
            entry: entry.to_string(),
        }
    }

    fn entry(&self) -> &str {
        self.entry.as_str()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct EntryRecord {
    target_lang: String,
    confident: bool,
    item: CandidateRecord,
}

impl EntryRecord {
    fn from_candidate(guess: &TargetGuess, candidate: &WordCandidate) -> Self {
        Self {
            target_lang: guess.code().to_string(),
            confident: guess.confident(),
            item: CandidateRecord::from_candidate(candidate),
        }
    }

    fn guess(&self) -> TargetGuess {
        TargetGuess::new(self.target_lang.clone(), self.confident)
    }

    fn candidate(self) -> WordCandidate {
        self.item.candidate()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct CandidateRecord {
    term: String,
    understanding: String,
    ok: bool,
}

impl CandidateRecord {
    fn from_candidate(candidate: &WordCandidate) -> Self {
        Self {
            term: candidate.term().to_string(),
            understanding: candidate.understanding().to_string(),
            ok: candidate.ok(),
        }
    }

    fn candidate(self) -> WordCandidate {
        WordCandidate::new(self.term, self.understanding, self.ok)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct MetaRecord {
    term: String,
    #[serde(default)]
    understanding: String,
    target_lang: String,
    source_lang: String,
    pronunciation: String,
    transcription: String,
    meaning: String,
    importance: u8,
    source_sentence: String,
    source_highlight: String,
    source_hint: String,
    source_context: String,
    target_sentence: String,
}

impl MetaRecord {
    fn from_meta(term: &str, understanding: &str, pair: &LanguagePair, meta: &CardMeta) -> Self {
        Self {
            term: term.to_string(),
            understanding: understanding.to_string(),
            target_lang: pair.target().to_string(),
            source_lang: pair.support().to_string(),
            pronunciation: meta.pronunciation().to_string(),
            transcription: meta.transcription().to_string(),
            meaning: meta.meaning().to_string(),
            importance: meta.importance(),
            source_sentence: meta.source_sentence().to_string(),
            source_highlight: meta.source_highlight().to_string(),
            source_hint: meta.source_hint().to_string(),
            source_context: meta.source_context().to_string(),
            target_sentence: meta.target_sentence().to_string(),
        }
    }

    fn meta(self) -> CardMeta {
        CardMeta::new(
            self.pronunciation,
            self.transcription,
            self.meaning,
            self.importance,
            self.source_sentence,
            self.source_highlight,
            self.source_hint,
            self.source_context,
            self.target_sentence,
        )
    }
}

fn read_json<T>(cache: &Cache, filename: &str) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let path = cache.filepath(filename)?;
    let text = fs::read_to_string(path)
        .with_context(|| format!("cache file '{filename}' is unreadable"))?;
    serde_json::from_str(text.as_str())
        .with_context(|| format!("cache file '{filename}' is invalid"))
}

fn write_json<T>(cache: &Cache, filename: &str, payload: &T) -> Result<()>
where
    T: Serialize,
{
    let staged = cache.stage(".json")?;
    let result = fs::write(&staged, serde_json::to_string_pretty(payload)?)
        .map_err(anyhow::Error::from)
        .and_then(|()| cache.commit(&staged, filename));
    if result.is_err() {
        let _ = fs::remove_file(&staged);
    }
    result
}

fn normalized_entries(raw: &RawInputBatch) -> Vec<String> {
    raw.text()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect()
}

fn indexed(understood: Understood) -> (TargetGuess, Vec<(usize, WordCandidate)>) {
    let guess = understood.guess().clone();
    let candidates = understood
        .candidates()
        .iter()
        .cloned()
        .enumerate()
        .collect();
    (guess, candidates)
}

fn indexed_missing(
    understood: Understood,
    misses: Vec<EntryMiss>,
) -> (TargetGuess, Vec<(usize, WordCandidate)>) {
    let guess = understood.guess().clone();
    let candidates = misses
        .into_iter()
        .zip(understood.candidates().iter().cloned())
        .map(|(miss, candidate)| (miss.index, candidate))
        .collect();
    (guess, candidates)
}

fn digest(value: &str) -> String {
    let full = format!("{:x}", md5::compute(value.as_bytes()));
    full[..12].to_string()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use tempfile::TempDir;

    use super::*;

    #[derive(Clone)]
    struct ChangingUnderstanding {
        calls: Rc<RefCell<usize>>,
    }

    impl ChangingUnderstanding {
        fn new(calls: Rc<RefCell<usize>>) -> Self {
            Self { calls }
        }
    }

    impl Understanding for ChangingUnderstanding {
        fn understand(&self, raw: &RawInputBatch, _my: &str) -> Result<Understood> {
            let current = *self.calls.borrow();
            *self.calls.borrow_mut() = current + 1;
            let candidates = normalized_entries(raw)
                .into_iter()
                .map(|entry| WordCandidate::new(entry, format!("variant {current}"), true))
                .collect();
            Ok(Understood::new(TargetGuess::new("en", true), candidates))
        }
    }

    fn meta(sentence: &str) -> CardMeta {
        CardMeta::new(
            "/lantern/",
            "/the lantern glowed/",
            "фонарь",
            5,
            "Фонарь светился",
            "Фонарь",
            "Подумай о свете",
            "A common concrete noun",
            sentence,
        )
    }

    #[test]
    fn repeated_input_reuses_cached_understanding() {
        let directory = TempDir::new().expect("tempdir must be created");
        let calls = Rc::new(RefCell::new(0));
        let cache =
            CachedUnderstanding::new(ChangingUnderstanding::new(calls.clone()), directory.path());
        let first = cache
            .understand(&RawInputBatch::new(" lantern \n"), "ru")
            .expect("first understanding must succeed");
        let second = cache
            .understand(&RawInputBatch::new("lantern"), "ru")
            .expect("second understanding must succeed");
        assert_eq!(
            (
                *calls.borrow(),
                first.candidates()[0].understanding(),
                second.candidates()[0].understanding(),
            ),
            (1, "variant 0", "variant 0"),
            "understanding cache no longer reuses normalized duplicate input"
        );
    }

    #[test]
    fn support_language_keeps_a_separate_understanding_cache_entry() {
        let directory = TempDir::new().expect("tempdir must be created");
        let calls = Rc::new(RefCell::new(0));
        let cache =
            CachedUnderstanding::new(ChangingUnderstanding::new(calls.clone()), directory.path());
        cache
            .understand(&RawInputBatch::new("lantern"), "ru")
            .expect("first understanding must succeed");
        let second = cache
            .understand(&RawInputBatch::new("lantern"), "el")
            .expect("second understanding must succeed");
        assert_eq!(
            (*calls.borrow(), second.candidates()[0].understanding()),
            (2, "variant 1"),
            "understanding cache no longer separates different support languages"
        );
    }

    #[test]
    fn cached_entry_is_reused_when_a_new_entry_is_added_to_the_batch() {
        let directory = TempDir::new().expect("tempdir must be created");
        let calls = Rc::new(RefCell::new(0));
        let cache =
            CachedUnderstanding::new(ChangingUnderstanding::new(calls.clone()), directory.path());
        cache
            .understand(&RawInputBatch::new("catdog"), "ru")
            .expect("first understanding must succeed");
        let second = cache
            .understand(&RawInputBatch::new("catdog\nflower"), "ru")
            .expect("second understanding must succeed");
        let candidates = second
            .candidates()
            .iter()
            .map(|candidate| {
                (
                    candidate.term().to_string(),
                    candidate.understanding().to_string(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            (*calls.borrow(), candidates),
            (
                2,
                vec![
                    (String::from("catdog"), String::from("variant 0")),
                    (String::from("flower"), String::from("variant 1"))
                ]
            ),
            "understanding cache no longer reuses one cached entry inside a larger batch"
        );
    }

    #[test]
    fn card_meta_cache_reopens_the_same_sentence_for_the_same_understanding() {
        let directory = TempDir::new().expect("tempdir must be created");
        let pair = LanguagePair::new("en", "ru");
        let cache = CardMetaCache::new(directory.path());
        let first = cache
            .store(
                "lantern",
                "a portable lamp",
                &pair,
                &meta("The lantern glowed under rain"),
            )
            .expect("meta must store");
        let loaded = cache
            .load("lantern", "a portable lamp", &pair)
            .expect("meta must load")
            .expect("meta must exist");
        let second = cache
            .store(
                "lantern",
                "a portable lamp",
                &pair,
                &meta("A new sentence should not replace it"),
            )
            .expect("meta must store again");
        assert_eq!(
            (
                first.0 == second.0,
                first.2,
                second.2,
                loaded.target_sentence()
            ),
            (true, false, true, "The lantern glowed under rain"),
            "card meta cache no longer preserves the first generated sentence"
        );
    }

    #[test]
    fn card_meta_cache_key_includes_the_understanding() {
        let directory = TempDir::new().expect("tempdir must be created");
        let pair = LanguagePair::new("en", "ru");
        let cache = CardMetaCache::new(directory.path());
        let noun = cache
            .store(
                "wreck",
                "a ruined ship",
                &pair,
                &meta("The wreck leaned in the fog"),
            )
            .expect("noun meta must store");
        let verb = cache
            .store(
                "wreck",
                "to destroy",
                &pair,
                &meta("I might wreck the old bike"),
            )
            .expect("verb meta must store");
        let loaded = cache
            .load("wreck", "to destroy", &pair)
            .expect("verb meta must load")
            .expect("verb meta must exist");
        assert_eq!(
            (noun.0 == verb.0, loaded.target_sentence()),
            (false, "I might wreck the old bike"),
            "card meta cache no longer separates corrected meanings"
        );
    }

    #[test]
    fn card_meta_cache_cannot_reuse_a_term_after_understanding_changes() {
        let directory = TempDir::new().expect("tempdir must be created");
        let pair = LanguagePair::new("en", "ru");
        let cache = CardMetaCache::new(directory.path());
        cache
            .store(
                "cat",
                "Сущ. «кошка», домашнее животное.",
                &pair,
                &meta("The cat slept on the windowsill"),
            )
            .expect("meta must store");
        let loaded = cache
            .load("cat", "Сущ. «кот», домашнее животное.", &pair)
            .expect("meta lookup must succeed");
        assert_eq!(
            loaded, None,
            "card meta cache must not reuse a stale meta after the user changes understanding"
        );
    }
}
