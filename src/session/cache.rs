//! Persistent session caches for reviewed input and card meta.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::application::{LearningTarget, Understanding};
use crate::generation::artifact_cache::{Cache, META_FILE};
use crate::languages::catalog;

use super::vault::{CardCell, digest};
use super::{
    CardMeta, IntakeTooLarge, LanguagePair, LearningDetection, LearningGuess, MAX_INTAKE_WORDS,
    RawInputBatch, ScriptDetection, Sense, SentenceLabels, Understood, WordCandidate,
};

const UNDERSTANDING_VERSION: &str = "v6";

/// How many vocabulary lines one intake request carries.
///
/// Twenty words of the worst-case polysemous shape stay well under the intake
/// output ceiling and take roughly a quarter of the transport timeout, and each
/// chunk is written to the cache as soon as it decodes — so a batch that fails
/// part-way keeps everything the earlier chunks produced.
const INTAKE_CHUNK_WORDS: usize = 20;
const META_POLICY: &str = "v3-initial-sentence-preferences";

/// Caching decorator for the first-pass understanding contract.
#[derive(Clone, Debug)]
pub struct CachedUnderstanding<T> {
    inner: T,
    root: PathBuf,
}

#[derive(Clone, Copy, Debug)]
struct UnderstandingScope<'a> {
    known: &'a str,
    identity: &'a str,
    target: &'a LearningTarget,
}

impl<'a> UnderstandingScope<'a> {
    fn new(known: &'a str, identity: &'a str, target: &'a LearningTarget) -> Self {
        Self {
            known,
            identity,
            target,
        }
    }
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
    fn understand(
        &self,
        raw: &RawInputBatch,
        known: &str,
        target: &LearningTarget,
    ) -> Result<Understood> {
        let words = raw.word_count();
        if words > MAX_INTAKE_WORDS {
            return Err(IntakeTooLarge::new(words).into());
        }
        let detected = match target {
            LearningTarget::Detect => ScriptDetection.detect(raw.text(), &catalog())?,
            LearningTarget::Explicit(code) => LearningGuess::new(code.to_string(), true),
        };
        let known = known.to_uppercase();
        let target_code = detected.code().to_uppercase();
        let target_identity = match target {
            LearningTarget::Detect => format!("detect:{target_code}"),
            LearningTarget::Explicit(code) => format!("explicit:{code}"),
        };
        let scope = UnderstandingScope::new(known.as_str(), target_identity.as_str(), target);
        let cache = Cache::new(
            format!("understanding/{known}-{target_code}"),
            self.root.clone(),
        );
        let entries = normalized_entries(raw);
        let mut merged = vec![None; entries.len()];
        let mut misses = Vec::new();
        let mut guess = None;
        for (index, entry) in entries.iter().enumerate() {
            let filename = self.entry_filename(entry, scope.known, scope.identity);
            if cache.exists(filename.as_str()) {
                let record: EntryRecord = read_json(&cache, filename.as_str())?;
                guess = guess.or_else(|| Some(record.guess()));
                merged[index] = Some(record.candidate());
            } else {
                misses.push(EntryMiss::new(index, entry));
            }
        }
        if !misses.is_empty() {
            let (returned, candidates) = self.missing(&cache, scope, misses)?;
            guess = Some(returned);
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
        let guess = match target {
            LearningTarget::Detect => guess.unwrap_or(detected),
            LearningTarget::Explicit(code) => LearningGuess::new(code.to_string(), true),
        };
        Ok(Understood::new(guess, candidates))
    }
}

impl<T> CachedUnderstanding<T> {
    fn entry_filename(&self, entry: &str, my: &str, target: &str) -> String {
        format!(
            "{}.json",
            digest(&[UNDERSTANDING_VERSION, my, target, entry])
        )
    }

    fn missing(
        &self,
        cache: &Cache,
        scope: UnderstandingScope<'_>,
        misses: Vec<EntryMiss>,
    ) -> Result<(LearningGuess, Vec<(usize, WordCandidate)>)>
    where
        T: Understanding,
    {
        let unique = deduplicated(&misses);
        let mut guess: Option<LearningGuess> = None;
        let mut resolved: Vec<(usize, WordCandidate)> = Vec::new();
        for chunk in unique.chunks(INTAKE_CHUNK_WORDS) {
            let understood = self.chunk_understood(scope, chunk)?;
            guess = guess.or_else(|| Some(understood.guess().clone()));
            for ((entry, rows), candidate) in chunk.iter().zip(understood.candidates()) {
                let filename = self.entry_filename(entry, scope.known, scope.identity);
                write_json(
                    cache,
                    filename.as_str(),
                    &EntryRecord::from_candidate(understood.guess(), candidate),
                )?;
                for row in rows {
                    resolved.push((*row, candidate.clone()));
                }
            }
        }
        let guess = guess
            .ok_or_else(|| anyhow!("understanding pass returned no language for the batch"))?;
        Ok((guess, resolved))
    }

    /// Ask one bounded chunk, retrying that chunk alone when the reply carries
    /// the wrong number of rows. A short reply is the signature of a truncated
    /// response, so the retry stays chunk-scoped and never grows the request.
    fn chunk_understood(
        &self,
        scope: UnderstandingScope<'_>,
        chunk: &[(String, Vec<usize>)],
    ) -> Result<Understood>
    where
        T: Understanding,
    {
        let raw = RawInputBatch::new(
            chunk
                .iter()
                .map(|(entry, _)| entry.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let understood = self.inner.understand(&raw, scope.known, scope.target)?;
        enforce_target(&understood, scope.target)?;
        if understood.candidates().len() == chunk.len() {
            return Ok(understood);
        }
        let retried = self.inner.understand(&raw, scope.known, scope.target)?;
        enforce_target(&retried, scope.target)?;
        if retried.candidates().len() != chunk.len() {
            return Err(anyhow!(
                "understanding pass returned {} rows for {} words",
                retried.candidates().len(),
                chunk.len()
            ));
        }
        Ok(retried)
    }
}

/// Group repeated vocabulary lines so one line is asked about exactly once.
///
/// Entries stay 1:1 with the input rows everywhere else, so the returned rows
/// carry every position that shares a line and the single answer fans back out
/// to all of them.
fn deduplicated(misses: &[EntryMiss]) -> Vec<(String, Vec<usize>)> {
    let mut unique: Vec<(String, Vec<usize>)> = Vec::new();
    for miss in misses {
        if let Some(slot) = unique.iter_mut().find(|(entry, _)| entry == miss.entry()) {
            slot.1.push(miss.index());
        } else {
            unique.push((miss.entry().to_string(), vec![miss.index()]));
        }
    }
    unique
}

fn enforce_target(understood: &Understood, target: &LearningTarget) -> Result<()> {
    if let LearningTarget::Explicit(expected) = target
        && !understood
            .guess()
            .code()
            .eq_ignore_ascii_case(expected.as_ref())
    {
        return Err(anyhow!(
            "understanding target '{}' violates required target '{}'",
            understood.guess().code(),
            expected
        ));
    }
    Ok(())
}

/// Persistent cache for Gemini card-meta payloads.
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
        Ok(self
            .record(term, understanding, pair)?
            .map(MetaRecord::meta))
    }

    /// Return cached card meta only when it uses the current generation policy.
    pub(crate) fn load_current(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
    ) -> Result<Option<CardMeta>> {
        Ok(self
            .record(term, understanding, pair)?
            .filter(MetaRecord::current)
            .map(MetaRecord::meta))
    }

    /// Persist one card meta and return filename, path, and whether it already existed.
    pub fn store(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        meta: &CardMeta,
    ) -> Result<(String, PathBuf, bool)> {
        let cache = CardCell::new(self.root.clone(), pair, term, understanding).cache();
        let existed = cache.exists(META_FILE);
        let cached = existed && read_json::<MetaRecord>(&cache, META_FILE)?.current();
        if !cached {
            replace_json(
                &cache,
                META_FILE,
                &MetaRecord::from_meta(term, understanding, pair, meta),
            )?;
        }
        let path = cache.filepath(META_FILE)?;
        Ok((META_FILE.to_string(), path, cached))
    }

    /// Atomically replace one metadata record even when its policy is current.
    pub(crate) fn replace(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        meta: &CardMeta,
    ) -> Result<(String, PathBuf)> {
        let cache = CardCell::new(self.root.clone(), pair, term, understanding).cache();
        replace_json(
            &cache,
            META_FILE,
            &MetaRecord::from_meta(term, understanding, pair, meta),
        )?;
        Ok((META_FILE.to_string(), cache.filepath(META_FILE)?))
    }

    fn record(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
    ) -> Result<Option<MetaRecord>> {
        let cache = CardCell::new(self.root.clone(), pair, term, understanding).cache();
        if !cache.exists(META_FILE) {
            return Ok(None);
        }
        Ok(Some(read_json(&cache, META_FILE)?))
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

    fn index(&self) -> usize {
        self.index
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct EntryRecord {
    target_lang: String,
    confident: bool,
    /// The batch's equally plausible languages, copied onto every entry exactly
    /// like `target_lang`. Defaulted so entries written before the pass reported
    /// alternates keep loading — they simply offer none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    alternates: Vec<String>,
    item: CandidateRecord,
}

impl EntryRecord {
    fn from_candidate(guess: &LearningGuess, candidate: &WordCandidate) -> Self {
        Self {
            target_lang: guess.code().to_string(),
            confident: guess.confident(),
            alternates: guess.alternates().to_vec(),
            item: CandidateRecord::from_candidate(candidate),
        }
    }

    fn guess(&self) -> LearningGuess {
        LearningGuess::new(self.target_lang.clone(), self.confident)
            .with_alternates(self.alternates.clone())
    }

    fn candidate(self) -> WordCandidate {
        self.item.candidate()
    }
}

/// Serde shape for one reviewed candidate, shared by the understanding cache and
/// the persistent session record so both round-trip the same curation state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct CandidateRecord {
    term: String,
    senses: Vec<SenseRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selected: Option<usize>,
    #[serde(default)]
    selected_senses: Vec<usize>,
    ok: bool,
}

impl CandidateRecord {
    /// Project one reviewed candidate into its serializable record.
    pub(crate) fn from_candidate(candidate: &WordCandidate) -> Self {
        Self {
            term: candidate.term().to_string(),
            senses: candidate
                .senses()
                .iter()
                .map(SenseRecord::from_sense)
                .collect(),
            selected: None,
            selected_senses: candidate.selected_senses().to_vec(),
            ok: candidate.ok(),
        }
    }

    /// Rebuild the reviewed candidate from its record.
    pub(crate) fn candidate(self) -> WordCandidate {
        let selected = if self.selected_senses.is_empty() {
            self.selected
                .map(|index| vec![index])
                .unwrap_or_else(|| vec![0])
        } else {
            self.selected_senses
        };
        WordCandidate::with_selected_senses(
            self.term,
            self.senses.into_iter().map(SenseRecord::sense).collect(),
            selected,
            self.ok,
        )
    }

    /// Return the term this record carries.
    pub(crate) fn term(&self) -> &str {
        self.term.as_str()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct SenseRecord {
    understanding: String,
    tag: Option<String>,
}

impl SenseRecord {
    fn from_sense(sense: &Sense) -> Self {
        Self {
            understanding: sense.understanding().to_string(),
            tag: sense.tag().map(String::from),
        }
    }

    fn sense(self) -> Sense {
        Sense::new(self.understanding, self.tag)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct MetaRecord {
    #[serde(default)]
    policy: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    labels: Option<SentenceLabels>,
}

impl MetaRecord {
    fn from_meta(term: &str, understanding: &str, pair: &LanguagePair, meta: &CardMeta) -> Self {
        Self {
            policy: String::from(META_POLICY),
            term: term.to_string(),
            understanding: understanding.to_string(),
            target_lang: pair.learning().to_string(),
            source_lang: pair.known().to_string(),
            pronunciation: meta.pronunciation().to_string(),
            transcription: meta.transcription().to_string(),
            meaning: meta.meaning().to_string(),
            importance: meta.importance(),
            source_sentence: meta.source_sentence().to_string(),
            source_highlight: meta.source_highlight().to_string(),
            source_hint: meta.source_hint().to_string(),
            source_context: meta.source_context().to_string(),
            target_sentence: meta.target_sentence().to_string(),
            labels: meta.sentence_labels().cloned(),
        }
    }

    fn current(&self) -> bool {
        self.policy == META_POLICY
    }

    fn meta(self) -> CardMeta {
        let meta = CardMeta::new(
            self.pronunciation,
            self.transcription,
            self.meaning,
            self.importance,
            self.source_sentence,
            self.source_highlight,
            self.source_hint,
            self.source_context,
            self.target_sentence,
        );
        match self.labels {
            Some(labels) => meta.with_sentence_labels(labels),
            None => meta,
        }
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

fn replace_json<T>(cache: &Cache, filename: &str, payload: &T) -> Result<()>
where
    T: Serialize,
{
    let staged = cache.stage(".json")?;
    let result = fs::write(&staged, serde_json::to_string_pretty(payload)?)
        .map_err(anyhow::Error::from)
        .and_then(|()| commit_replacement(cache, &staged, filename));
    if result.is_err() {
        let _ = fs::remove_file(&staged);
    }
    result
}

#[cfg(not(windows))]
fn commit_replacement(cache: &Cache, staged: &std::path::Path, filename: &str) -> Result<()> {
    cache.commit(staged, filename)
}

#[cfg(windows)]
fn commit_replacement(cache: &Cache, staged: &std::path::Path, filename: &str) -> Result<()> {
    let current = cache.filepath(filename)?;
    if !current.exists() {
        return cache.commit(staged, filename);
    }
    let backup = cache.stage(".backup")?;
    fs::remove_file(&backup)?;
    fs::rename(&current, &backup)?;
    match cache.commit(staged, filename) {
        Ok(()) => {
            fs::remove_file(backup)?;
            Ok(())
        }
        Err(error) => {
            if current.exists() {
                fs::remove_file(&current)?;
            }
            fs::rename(backup, current).with_context(|| {
                format!("failed to restore cache after replacement error: {error:#}")
            })?;
            Err(error)
        }
    }
}

fn normalized_entries(raw: &RawInputBatch) -> Vec<String> {
    raw.lines().map(String::from).collect()
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

    /// A pass that always reports two equally plausible languages.
    struct AmbiguousUnderstanding {
        calls: Rc<RefCell<usize>>,
    }

    impl AmbiguousUnderstanding {
        fn new(calls: Rc<RefCell<usize>>) -> Self {
            Self { calls }
        }
    }

    impl Understanding for AmbiguousUnderstanding {
        fn understand(
            &self,
            raw: &RawInputBatch,
            _known: &str,
            _target: &LearningTarget,
        ) -> Result<Understood> {
            *self.calls.borrow_mut() += 1;
            let candidates = normalized_entries(raw)
                .into_iter()
                .map(|entry| WordCandidate::new(entry, "a cat", true))
                .collect();
            let guess = LearningGuess::new("EN", true)
                .with_alternates(vec![String::from("DE"), String::from("NL")]);
            Ok(Understood::new(guess, candidates))
        }
    }

    impl Understanding for ChangingUnderstanding {
        fn understand(
            &self,
            raw: &RawInputBatch,
            _known: &str,
            target: &LearningTarget,
        ) -> Result<Understood> {
            let current = *self.calls.borrow();
            *self.calls.borrow_mut() = current + 1;
            let candidates = normalized_entries(raw)
                .into_iter()
                .map(|entry| WordCandidate::new(entry, format!("variant {current}"), true))
                .collect();
            let guess = match target {
                LearningTarget::Detect => LearningGuess::new("en", true),
                LearningTarget::Explicit(code) => LearningGuess::new(code.to_string(), true),
            };
            Ok(Understood::new(guess, candidates))
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

    /// A pass that records every batch it is handed and can fail on demand.
    struct RecordingUnderstanding {
        seen: Rc<RefCell<Vec<Vec<String>>>>,
        fail_from: usize,
    }

    impl RecordingUnderstanding {
        fn new(seen: Rc<RefCell<Vec<Vec<String>>>>, fail_from: usize) -> Self {
            Self { seen, fail_from }
        }
    }

    impl Understanding for RecordingUnderstanding {
        fn understand(
            &self,
            raw: &RawInputBatch,
            _known: &str,
            _target: &LearningTarget,
        ) -> Result<Understood> {
            let entries = normalized_entries(raw);
            self.seen.borrow_mut().push(entries.clone());
            if self.seen.borrow().len() > self.fail_from {
                return Err(anyhow!("understanding pass refused this chunk"));
            }
            let candidates = entries
                .into_iter()
                .map(|entry| WordCandidate::new(entry, "a meaning", true))
                .collect();
            Ok(Understood::new(LearningGuess::new("EN", true), candidates))
        }
    }

    /// A pass whose first reply is one row short, then answers in full.
    struct ShortOnceUnderstanding {
        seen: Rc<RefCell<Vec<Vec<String>>>>,
    }

    impl ShortOnceUnderstanding {
        fn new(seen: Rc<RefCell<Vec<Vec<String>>>>) -> Self {
            Self { seen }
        }
    }

    impl Understanding for ShortOnceUnderstanding {
        fn understand(
            &self,
            raw: &RawInputBatch,
            _known: &str,
            _target: &LearningTarget,
        ) -> Result<Understood> {
            let entries = normalized_entries(raw);
            self.seen.borrow_mut().push(entries.clone());
            let first = self.seen.borrow().len() == 1;
            let kept = if first {
                entries.len().saturating_sub(1)
            } else {
                entries.len()
            };
            let candidates = entries
                .into_iter()
                .take(kept)
                .map(|entry| WordCandidate::new(entry, "a meaning", true))
                .collect();
            Ok(Understood::new(LearningGuess::new("EN", true), candidates))
        }
    }

    fn blob(count: usize) -> String {
        (0..count)
            .map(|index| format!("word-{index:03}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn an_oversized_batch_is_refused_before_the_understanding_pass_runs() {
        let directory = TempDir::new().expect("tempdir must be created");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let cache = CachedUnderstanding::new(
            RecordingUnderstanding::new(seen.clone(), usize::MAX),
            directory.path(),
        );
        let refused = cache.understand(
            &RawInputBatch::new(blob(MAX_INTAKE_WORDS + 1)),
            "ru",
            &LearningTarget::Detect,
        );
        let files = std::fs::read_dir(directory.path())
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(
            (refused.is_err(), seen.borrow().len(), files),
            (true, 0, 0),
            "an oversized batch still reached the provider or touched the cache"
        );
    }

    #[test]
    fn a_batch_larger_than_one_chunk_is_asked_in_bounded_requests() {
        let directory = TempDir::new().expect("tempdir must be created");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let cache = CachedUnderstanding::new(
            RecordingUnderstanding::new(seen.clone(), usize::MAX),
            directory.path(),
        );
        cache
            .understand(&RawInputBatch::new(blob(45)), "ru", &LearningTarget::Detect)
            .expect("a full batch must be understood");
        let requests = seen.borrow();
        assert_eq!(
            (
                requests.len(),
                requests.iter().map(Vec::len).max(),
                requests.iter().map(Vec::len).sum::<usize>()
            ),
            (3, Some(INTAKE_CHUNK_WORDS), 45),
            "the batch was not split into bounded intake requests"
        );
    }

    #[test]
    fn a_failing_chunk_keeps_the_entries_earlier_chunks_produced() {
        let directory = TempDir::new().expect("tempdir must be created");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let cache = CachedUnderstanding::new(
            RecordingUnderstanding::new(seen.clone(), 1),
            directory.path(),
        );
        let refused =
            cache.understand(&RawInputBatch::new(blob(40)), "ru", &LearningTarget::Detect);
        let stored = std::fs::read_dir(directory.path().join("understanding/RU-EN"))
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(
            (refused.is_err(), stored),
            (true, INTAKE_CHUNK_WORDS),
            "a batch that failed part-way threw away the words it had already understood"
        );
    }

    #[test]
    fn duplicate_lines_are_asked_once_and_fan_out_to_every_row() {
        let directory = TempDir::new().expect("tempdir must be created");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let cache = CachedUnderstanding::new(
            RecordingUnderstanding::new(seen.clone(), usize::MAX),
            directory.path(),
        );
        let understood = cache
            .understand(
                &RawInputBatch::new("alpha\nbeta\nalpha"),
                "ru",
                &LearningTarget::Detect,
            )
            .expect("a batch with a repeated line must be understood");
        assert_eq!(
            (
                seen.borrow().len(),
                seen.borrow().first().map(Vec::len),
                understood.candidates().len()
            ),
            (1, Some(2), 3),
            "a repeated line was asked about twice or lost its answer"
        );
    }

    #[test]
    fn a_short_chunk_reply_re_asks_only_that_chunk() {
        let directory = TempDir::new().expect("tempdir must be created");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let cache =
            CachedUnderstanding::new(ShortOnceUnderstanding::new(seen.clone()), directory.path());
        cache
            .understand(&RawInputBatch::new(blob(25)), "ru", &LearningTarget::Detect)
            .expect("a short first reply must be retried and then succeed");
        let requests = seen.borrow();
        assert_eq!(
            (requests.len(), requests.iter().map(Vec::len).max()),
            (3, Some(INTAKE_CHUNK_WORDS)),
            "a short reply re-asked more than the chunk that came back short"
        );
    }

    #[test]
    fn repeated_input_reuses_cached_understanding() {
        let directory = TempDir::new().expect("tempdir must be created");
        let calls = Rc::new(RefCell::new(0));
        let cache =
            CachedUnderstanding::new(ChangingUnderstanding::new(calls.clone()), directory.path());
        let first = cache
            .understand(
                &RawInputBatch::new(" lantern \n"),
                "ru",
                &LearningTarget::Detect,
            )
            .expect("first understanding must succeed");
        let second = cache
            .understand(
                &RawInputBatch::new("lantern"),
                "ru",
                &LearningTarget::Detect,
            )
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
    fn combined_intake_contract_uses_the_version_six_understanding_identity() {
        let cache = CachedUnderstanding::new(
            ChangingUnderstanding::new(Rc::new(RefCell::new(0))),
            "/tmp/kamishibai-understanding-version-test",
        );
        assert_eq!(
            cache.entry_filename("lantern", "RU", "detect:EN"),
            "594e83a91b56.json",
            "language-local examples and explicit-target intake reused an earlier understanding contract"
        );
    }

    #[test]
    fn support_language_keeps_a_separate_understanding_cache_entry() {
        let directory = TempDir::new().expect("tempdir must be created");
        let calls = Rc::new(RefCell::new(0));
        let cache =
            CachedUnderstanding::new(ChangingUnderstanding::new(calls.clone()), directory.path());
        cache
            .understand(
                &RawInputBatch::new("lantern"),
                "ru",
                &LearningTarget::Detect,
            )
            .expect("first understanding must succeed");
        let second = cache
            .understand(
                &RawInputBatch::new("lantern"),
                "el",
                &LearningTarget::Detect,
            )
            .expect("second understanding must succeed");
        assert_eq!(
            (*calls.borrow(), second.candidates()[0].understanding()),
            (2, "variant 1"),
            "understanding cache no longer separates different support languages"
        );
    }

    #[test]
    fn explicit_languages_and_autodetection_have_distinct_cache_identities() {
        let directory = TempDir::new().expect("tempdir must be created");
        let calls = Rc::new(RefCell::new(0));
        let cache =
            CachedUnderstanding::new(ChangingUnderstanding::new(calls.clone()), directory.path());
        let french =
            LearningTarget::Explicit(catalog().resolve("fr").expect("French must resolve"));
        let english =
            LearningTarget::Explicit(catalog().resolve("EN").expect("English must resolve"));
        let first = cache
            .understand(&RawInputBatch::new("chat"), "RU", &french)
            .expect("explicit French understanding must succeed");
        let second = cache
            .understand(&RawInputBatch::new("chat"), "RU", &english)
            .expect("explicit English understanding must succeed");
        let third = cache
            .understand(&RawInputBatch::new("chat"), "RU", &LearningTarget::Detect)
            .expect("autodetected understanding must succeed");
        let repeated = cache
            .understand(&RawInputBatch::new("chat"), "RU", &french)
            .expect("explicit French cache lookup must succeed");
        assert_eq!(
            (
                *calls.borrow(),
                first.candidates()[0].understanding(),
                second.candidates()[0].understanding(),
                third.candidates()[0].understanding(),
                repeated.candidates()[0].understanding(),
            ),
            (3, "variant 0", "variant 1", "variant 2", "variant 0"),
            "explicit EN, explicit FR, and autodetection reused one understanding identity"
        );
    }

    #[test]
    fn a_cached_entry_replays_the_alternates_the_pass_reported() {
        let directory = TempDir::new().expect("tempdir must be created");
        let calls = Rc::new(RefCell::new(0));
        let cache =
            CachedUnderstanding::new(AmbiguousUnderstanding::new(calls.clone()), directory.path());
        cache
            .understand(&RawInputBatch::new("chat"), "RU", &LearningTarget::Detect)
            .expect("first understanding must succeed");
        let replayed = cache
            .understand(&RawInputBatch::new("chat"), "RU", &LearningTarget::Detect)
            .expect("cached understanding must succeed");
        assert_eq!(
            (*calls.borrow(), replayed.guess().alternates()),
            (1, ["DE".to_string(), "NL".to_string()].as_slice()),
            "a cache hit lost the languages the pass judged equally plausible"
        );
    }

    #[test]
    fn an_entry_written_before_alternates_existed_still_loads() {
        let directory = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new(String::from("understanding/RU-EN"), directory.path());
        fs::create_dir_all(cache.path()).expect("cache directory must be created");
        fs::write(
            cache.path().join("legacy.json"),
            br#"{"target_lang":"EN","confident":true,"item":{"term":"chat","senses":[{"understanding":"a cat","tag":null}],"selected_senses":[0],"ok":true}}"#,
        )
        .expect("legacy entry must be written");
        let record: EntryRecord = read_json(&cache, "legacy.json").expect("legacy entry must load");
        assert_eq!(
            record.guess().alternates(),
            Vec::<String>::new().as_slice(),
            "an entry predating alternates must load offering none"
        );
    }

    #[test]
    fn cached_entry_is_reused_when_a_new_entry_is_added_to_the_batch() {
        let directory = TempDir::new().expect("tempdir must be created");
        let calls = Rc::new(RefCell::new(0));
        let cache =
            CachedUnderstanding::new(ChangingUnderstanding::new(calls.clone()), directory.path());
        cache
            .understand(&RawInputBatch::new("catdog"), "ru", &LearningTarget::Detect)
            .expect("first understanding must succeed");
        let second = cache
            .understand(
                &RawInputBatch::new("catdog\nflower"),
                "ru",
                &LearningTarget::Detect,
            )
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
    fn failed_meta_replacement_preserves_the_previous_document() {
        let directory = TempDir::new().expect("tempdir must be created");
        let cache = Cache::failing("card", directory.path(), 0);
        fs::write(
            cache.filepath(META_FILE).expect("meta path must resolve"),
            r#"{"value":"old"}"#,
        )
        .expect("old meta must store");
        let result = replace_json(&cache, META_FILE, &serde_json::json!({"value": "new"}));
        let preserved = fs::read_to_string(
            cache
                .filepath(META_FILE)
                .expect("preserved meta path must resolve"),
        )
        .expect("old meta must remain readable");
        assert_eq!(
            (result.is_err(), preserved.as_str()),
            (true, r#"{"value":"old"}"#),
            "failed meta replacement deleted the previous readable document"
        );
    }

    #[test]
    fn card_meta_cache_replaces_an_older_prompt_policy_in_place() {
        let directory = TempDir::new().expect("tempdir must be created");
        let pair = LanguagePair::new("en", "ru");
        let cache = CardMetaCache::new(directory.path());
        let cell = CardCell::new(directory.path(), &pair, "lantern", "a portable lamp").cache();
        let mut legacy = serde_json::to_value(MetaRecord::from_meta(
            "lantern",
            "a portable lamp",
            &pair,
            &meta("The old prompt wrote this sentence"),
        ))
        .expect("legacy meta must serialize");
        legacy
            .as_object_mut()
            .expect("legacy meta must be an object")
            .remove("policy");
        fs::write(
            cell.filepath(META_FILE)
                .expect("legacy meta path must resolve"),
            serde_json::to_vec_pretty(&legacy).expect("legacy meta must encode"),
        )
        .expect("legacy meta must store");
        let legacy_read = cache
            .load("lantern", "a portable lamp", &pair)
            .expect("legacy meta lookup must succeed")
            .expect("legacy meta must remain readable");
        let generation_read = cache
            .load_current("lantern", "a portable lamp", &pair)
            .expect("generation meta lookup must succeed");
        let stored = cache
            .store(
                "lantern",
                "a portable lamp",
                &pair,
                &meta("The localized prompt wrote this sentence"),
            )
            .expect("localized meta must replace the stale record");
        let refreshed = cache
            .load("lantern", "a portable lamp", &pair)
            .expect("localized meta must load")
            .expect("localized meta must exist");
        assert_eq!(
            (
                legacy_read.target_sentence(),
                generation_read,
                stored.2,
                refreshed.target_sentence()
            ),
            (
                "The old prompt wrote this sentence",
                None,
                false,
                "The localized prompt wrote this sentence"
            ),
            "card meta cache lost legacy reads or reused an older generation policy"
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
            (noun.1 == verb.1, loaded.target_sentence()),
            (false, "I might wreck the old bike"),
            "card meta cache no longer separates corrected meanings into distinct folders"
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
