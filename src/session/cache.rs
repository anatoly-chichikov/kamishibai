//! Persistent session caches for reviewed input and card meta.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::application::{LearningLanguageRequired, LearningTarget, Understanding};
use crate::generation::artifact_cache::{Cache, META_FILE};
use crate::languages::catalog;

use super::vault::{CardCell, digest};
use super::{
    CardDraft, CardMeta, IntakeTooLarge, LanguagePair, LearningDetection, LearningGuess,
    MAX_INTAKE_WORDS, RawInputBatch, ScriptDetection, Sense, SentenceLabels, Understood,
    WordCandidate,
};

const UNDERSTANDING_VERSION: &str = "v11";

/// How many vocabulary lines one intake request carries.
///
/// Twenty words of the worst-case polysemous shape stay well under the intake
/// output ceiling and take roughly a quarter of the transport timeout, and each
/// chunk is written to the cache as soon as it decodes — so a batch that fails
/// part-way keeps everything the earlier chunks produced.
const INTAKE_CHUNK_WORDS: usize = 20;
const META_POLICY: &str = "v11-scoped-and-illustrated-usage";

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
    context: Option<&'a RawInputBatch>,
}

impl<'a> UnderstandingScope<'a> {
    fn new(
        known: &'a str,
        identity: &'a str,
        target: &'a LearningTarget,
        context: Option<&'a RawInputBatch>,
    ) -> Self {
        Self {
            known,
            identity,
            target,
            context,
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
        let entries = normalized_entries(raw);
        let context = raw.context().map_or_else(
            || entries.join("\n"),
            |context| normalized_entries(&RawInputBatch::new(context)).join("\n"),
        );
        let detected = match target {
            LearningTarget::Detect => ScriptDetection.detect(&context, &catalog())?,
            LearningTarget::Explicit(code) => LearningGuess::new(code.to_string(), true),
        };
        let known = known.to_uppercase();
        let target_code = detected.code().to_uppercase();
        let target_identity = match target {
            LearningTarget::Detect => format!(
                "detect:{target_code}:{}",
                digest(&[&context, &raw.learning_history().join("\n")])
            ),
            LearningTarget::Explicit(code) => format!("explicit:{code}"),
        };
        let context =
            RawInputBatch::new(context).with_learning_history(raw.learning_history().to_vec());
        let scope = UnderstandingScope::new(
            known.as_str(),
            target_identity.as_str(),
            target,
            matches!(target, LearningTarget::Detect).then_some(&context),
        );
        let cache = Cache::new(
            format!("understanding/{known}-{target_code}"),
            self.root.clone(),
        );
        let mut merged = vec![None; entries.len()];
        let mut misses = Vec::new();
        let mut guess = match target {
            LearningTarget::Detect => None,
            LearningTarget::Explicit(code) => Some(LearningGuess::new(code.to_string(), true)),
        };
        for (index, entry) in entries.iter().enumerate() {
            let filename = self.entry_filename(entry, scope.known, scope.identity);
            if cache.exists(filename.as_str()) {
                let record: EntryRecord = read_json(&cache, filename.as_str())?;
                reconcile(&mut guess, record.guess(), target)?;
                merged[index] = Some(record.candidate());
            } else {
                misses.push(EntryMiss::new(index, entry));
            }
        }
        if !misses.is_empty() {
            let (returned, candidates) = self.missing(&cache, scope, misses, guess)?;
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
            LearningTarget::Detect => {
                guess.unwrap_or_else(|| LearningGuess::new(target_code, detected.confident()))
            }
            LearningTarget::Explicit(code) => LearningGuess::new(code.to_string(), true),
        };
        if matches!(target, LearningTarget::Detect) {
            validate_detection(&guess, &candidates, &known, raw.learning_history())?;
        }
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
        mut guess: Option<LearningGuess>,
    ) -> Result<(LearningGuess, Vec<(usize, WordCandidate)>)>
    where
        T: Understanding,
    {
        let unique = deduplicated(&misses);
        let mut resolved: Vec<(usize, WordCandidate)> = Vec::new();
        for chunk in unique.chunks(INTAKE_CHUNK_WORDS) {
            let understood = self.chunk_understood(scope, chunk)?;
            reconcile(&mut guess, understood.guess().clone(), scope.target)?;
            for ((entry, rows), candidate) in chunk.iter().zip(understood.candidates()) {
                let filename = self.entry_filename(entry, scope.known, scope.identity);
                write_json(
                    cache,
                    filename.as_str(),
                    &EntryRecord::from_candidate(
                        guess
                            .as_ref()
                            .expect("invariant: a validated chunk has a learning language"),
                        candidate,
                    ),
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
        let raw = match scope.context {
            Some(context) => raw
                .with_context(context.text())
                .with_learning_history(context.learning_history().to_vec()),
            None => raw,
        };
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

fn reconcile(
    current: &mut Option<LearningGuess>,
    returned: LearningGuess,
    target: &LearningTarget,
) -> Result<()> {
    let code = catalog()
        .resolve(returned.code())
        .context("understanding returned an unsupported target language")?;
    if let Some(previous) = current {
        if previous.requires_confirmation() != returned.requires_confirmation() {
            return Err(LearningLanguageRequired.into());
        }
        if previous.code() != code.as_ref() {
            if matches!(target, LearningTarget::Detect) {
                return Err(LearningLanguageRequired.into());
            }
            return Err(anyhow!(
                "understanding target '{}' conflicts with batch target '{}'",
                code,
                previous.code()
            ));
        }
    } else {
        *current = Some(
            LearningGuess::new(code.to_string(), returned.confident())
                .with_alternates(returned.alternates().to_vec())
                .with_confirmation(returned.requires_confirmation()),
        );
    }
    Ok(())
}

fn validate_detection(
    guess: &LearningGuess,
    candidates: &[WordCandidate],
    known: &str,
    history: &[String],
) -> Result<()> {
    let translated = candidates
        .iter()
        .flat_map(WordCandidate::senses)
        .any(|sense| sense.translation().is_some());
    let foreign = candidates.iter().any(|candidate| {
        candidate.ok()
            && candidate
                .senses()
                .iter()
                .all(|sense| sense.translation().is_none())
    });
    if guess.requires_confirmation() {
        let recent = history
            .iter()
            .filter_map(|language| catalog().resolve(language).ok())
            .find(|language| !language.as_ref().eq_ignore_ascii_case(known));
        let valid_translation = candidates.iter().any(|candidate| {
            candidate.ok()
                && candidate
                    .senses()
                    .iter()
                    .any(|sense| sense.translation().is_some())
        });
        let untranslated = candidates.iter().any(|candidate| {
            candidate.ok()
                && candidate
                    .senses()
                    .iter()
                    .any(|sense| sense.translation().is_none())
        });
        if !valid_translation
            || untranslated
            || !recent.is_some_and(|language| language.as_ref().eq_ignore_ascii_case(guess.code()))
        {
            return Err(LearningLanguageRequired.into());
        }
    } else if translated && !foreign {
        return Err(LearningLanguageRequired.into());
    }
    Ok(())
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
        let cell = CardCell::new(self.root.clone(), pair, term, understanding);
        self.load_at(&cell)
    }

    /// Return cached card metadata from one already-resolved card identity.
    pub(crate) fn load_at(&self, cell: &CardCell) -> Result<Option<CardMeta>> {
        Ok(self.record_at(cell)?.map(MetaRecord::meta))
    }

    /// Return cached metadata for one draft's complete reviewed-sense identity.
    pub(crate) fn load_for_draft(&self, draft: &CardDraft) -> Result<Option<CardMeta>> {
        self.load_at(&CardCell::for_draft(self.root.clone(), draft))
    }

    /// Return metadata from one card identity only under the current policy.
    pub(crate) fn load_current_at(&self, cell: &CardCell) -> Result<Option<CardMeta>> {
        Ok(self
            .record_at(cell)?
            .filter(MetaRecord::current)
            .map(MetaRecord::meta))
    }

    /// Check whether the current persisted document contains exactly the supplied metadata.
    pub(crate) fn matches(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        meta: &CardMeta,
    ) -> Result<bool> {
        let cell = CardCell::new(self.root.clone(), pair, term, understanding);
        Ok(self.record_at(&cell)?.as_ref()
            == Some(&MetaRecord::from_meta(term, understanding, pair, meta)))
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

    /// Atomically replace metadata inside one already-resolved card identity.
    pub(crate) fn replace_at(
        &self,
        cell: &CardCell,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        meta: &CardMeta,
    ) -> Result<(String, PathBuf)> {
        let cache = cell.cache();
        replace_json(
            &cache,
            META_FILE,
            &MetaRecord::from_meta(term, understanding, pair, meta),
        )?;
        Ok((META_FILE.to_string(), cache.filepath(META_FILE)?))
    }

    fn record_at(&self, cell: &CardCell) -> Result<Option<MetaRecord>> {
        let cache = cell.cache();
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
    #[serde(default)]
    requires_confirmation: bool,
    item: CandidateRecord,
}

impl EntryRecord {
    fn from_candidate(guess: &LearningGuess, candidate: &WordCandidate) -> Self {
        Self {
            target_lang: guess.code().to_string(),
            confident: guess.confident(),
            alternates: guess.alternates().to_vec(),
            requires_confirmation: guess.requires_confirmation(),
            item: CandidateRecord::from_candidate(candidate),
        }
    }

    fn guess(&self) -> LearningGuess {
        LearningGuess::new(self.target_lang.clone(), self.confident)
            .with_alternates(self.alternates.clone())
            .with_confirmation(self.requires_confirmation)
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    translation: Option<String>,
}

impl SenseRecord {
    fn from_sense(sense: &Sense) -> Self {
        Self {
            understanding: sense.understanding().to_string(),
            tag: sense.tag().map(String::from),
            translation: sense.translation().map(String::from),
        }
    }

    fn sense(self) -> Sense {
        match self.translation {
            Some(term) => Sense::translated(term, self.understanding, self.tag),
            None => Sense::new(self.understanding, self.tag),
        }
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
            crate::markdown::compact_card_context(&self.source_context),
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

    #[test]
    fn an_understanding_entry_cannot_lose_translations_when_saved() {
        let directory = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new(String::from("understanding/RU-EN"), directory.path());
        let candidate = WordCandidate::with_senses(
            "лук",
            vec![
                Sense::translated("onion", "Овощ со слоями", None),
                Sense::translated(
                    "bow",
                    "Оружие для стрельбы стрелами",
                    Some(String::from("оружие")),
                ),
            ],
            1,
            true,
        );
        write_json(
            &cache,
            "translated.json",
            &EntryRecord::from_candidate(&LearningGuess::new("EN", true), &candidate),
        )
        .expect("translated entry must save");
        let record: EntryRecord = read_json(&cache, "translated.json").expect("entry must reopen");
        assert_eq!(
            record.candidate(),
            candidate,
            "a cached translation lost its input, learning terms, selected sense, or tags"
        );
    }

    #[test]
    fn a_legacy_candidate_without_translations_cannot_change_its_learning_term() {
        let record: CandidateRecord = serde_json::from_str(
            r#"{"term":"stumble","senses":[{"understanding":"Споткнуться","tag":null}],"selected_senses":[0],"ok":true}"#,
        )
        .expect("legacy candidate must decode");
        let candidate = record.candidate();
        assert_eq!(
            (
                candidate.sense().term(candidate.term()),
                candidate.sense().translation()
            ),
            ("stumble", None),
            "a legacy untranslated candidate stopped using its original learning term"
        );
    }

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

    enum ContextReply {
        Complete,
        ShortFirst,
        FailSecond,
        SwitchTarget,
    }

    struct ContextUnderstanding {
        seen: Rc<RefCell<Vec<RawInputBatch>>>,
        reply: ContextReply,
    }

    impl ContextUnderstanding {
        fn new(seen: Rc<RefCell<Vec<RawInputBatch>>>, reply: ContextReply) -> Self {
            Self { seen, reply }
        }
    }

    impl Understanding for ContextUnderstanding {
        fn understand(
            &self,
            raw: &RawInputBatch,
            _known: &str,
            _target: &LearningTarget,
        ) -> Result<Understood> {
            self.seen.borrow_mut().push(raw.clone());
            let call = self.seen.borrow().len();
            if matches!(self.reply, ContextReply::FailSecond) && call == 2 {
                return Err(anyhow!("the second chunk failed"));
            }
            let context = raw.context().unwrap_or(raw.text());
            let language = if context.lines().any(|word| word == "cat") {
                "EN"
            } else {
                "FR"
            };
            let target = if matches!(self.reply, ContextReply::SwitchTarget) && call == 2 {
                "EL"
            } else {
                language
            };
            let kept = if matches!(self.reply, ContextReply::ShortFirst) && call == 1 {
                raw.word_count().saturating_sub(1)
            } else {
                raw.word_count()
            };
            let candidates = raw
                .lines()
                .take(kept)
                .map(|entry| {
                    if entry
                        .chars()
                        .any(|character| ('\u{0400}'..='\u{04ff}').contains(&character))
                    {
                        WordCandidate::with_senses(
                            entry,
                            vec![Sense::translated(format!("{target}:{entry}"), target, None)],
                            0,
                            true,
                        )
                    } else {
                        WordCandidate::new(entry, target, true)
                    }
                })
                .collect();
            Ok(Understood::new(
                LearningGuess::new(target, true),
                candidates,
            ))
        }
    }

    fn mixed_blob() -> String {
        (0..INTAKE_CHUNK_WORDS)
            .map(|index| format!("слово-{index}"))
            .chain(std::iter::once(String::from("cat")))
            .collect::<Vec<_>>()
            .join("\n")
    }

    enum RecentReply {
        Complete,
        ShortFirst,
        ForceConfirmation,
        SwitchConfirmation,
    }

    struct RecentUnderstanding {
        seen: Rc<RefCell<Vec<RawInputBatch>>>,
        reply: RecentReply,
    }

    impl RecentUnderstanding {
        fn new(seen: Rc<RefCell<Vec<RawInputBatch>>>, reply: RecentReply) -> Self {
            Self { seen, reply }
        }
    }

    impl Understanding for RecentUnderstanding {
        fn understand(
            &self,
            raw: &RawInputBatch,
            _known: &str,
            target: &LearningTarget,
        ) -> Result<Understood> {
            self.seen.borrow_mut().push(raw.clone());
            let call = self.seen.borrow().len();
            let foreign = raw
                .context()
                .unwrap_or(raw.text())
                .lines()
                .any(|line| line == "cat");
            let code = match target {
                LearningTarget::Explicit(code) => code.as_ref(),
                LearningTarget::Detect if foreign => "EN",
                LearningTarget::Detect => raw.learning_history()[0].as_str(),
            };
            let confirmation = matches!(target, LearningTarget::Detect)
                && (!foreign || matches!(self.reply, RecentReply::ForceConfirmation))
                && !(matches!(self.reply, RecentReply::SwitchConfirmation) && call == 2);
            let kept = if matches!(self.reply, RecentReply::ShortFirst) && call == 1 {
                raw.word_count().saturating_sub(1)
            } else {
                raw.word_count()
            };
            let candidates = raw
                .lines()
                .take(kept)
                .map(|entry| {
                    if entry == "cat" {
                        WordCandidate::new(entry, "Кошка", true)
                    } else {
                        WordCandidate::with_senses(
                            entry,
                            vec![Sense::translated(
                                format!("{code}:{entry}"),
                                "Перевод",
                                None,
                            )],
                            0,
                            true,
                        )
                    }
                })
                .collect();
            Ok(Understood::new(
                LearningGuess::new(code, true).with_confirmation(confirmation),
                candidates,
            ))
        }
    }

    #[test]
    fn cached_recent_translations_cannot_skip_confirmation_or_call_the_provider_again() {
        let directory = TempDir::new().expect("tempdir must be created");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let cache = CachedUnderstanding::new(
            RecentUnderstanding::new(seen.clone(), RecentReply::Complete),
            directory.path(),
        );
        let input = RawInputBatch::new("сумка\nдорога")
            .with_learning_history(vec![String::from("FR"), String::from("EN")]);
        cache
            .understand(&input, "RU", &LearningTarget::Detect)
            .expect("recent translations must be proposed");
        let replayed = cache
            .understand(&input, "RU", &LearningTarget::Detect)
            .expect("recent translations must replay");
        assert_eq!(
            (
                replayed.guess().code(),
                replayed.guess().requires_confirmation(),
                seen.borrow().len()
            ),
            ("FR", true, 1),
            "cached recent translations bypassed confirmation or repeated a provider call"
        );
    }

    #[test]
    fn automatic_recent_translations_cannot_reuse_a_different_ordered_history() {
        let directory = TempDir::new().expect("tempdir must be created");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let cache = CachedUnderstanding::new(
            RecentUnderstanding::new(seen.clone(), RecentReply::Complete),
            directory.path(),
        );
        let mut languages = Vec::new();
        for history in [["FR", "EN"], ["EN", "FR"], ["EN", "EL"]] {
            let input = RawInputBatch::new("сумка")
                .with_learning_history(history.map(String::from).to_vec());
            languages.push(
                cache
                    .understand(&input, "RU", &LearningTarget::Detect)
                    .expect("recent translations must be proposed")
                    .guess()
                    .code()
                    .to_string(),
            );
        }
        assert_eq!(
            (languages, seen.borrow().len()),
            (
                vec![String::from("FR"), String::from("EN"), String::from("EN")],
                3
            ),
            "automatic cache ignored a changed current or previous learning language"
        );
    }

    #[test]
    fn explicit_translations_cannot_change_cache_identity_with_recent_history() {
        let directory = TempDir::new().expect("tempdir must be created");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let cache = CachedUnderstanding::new(
            RecentUnderstanding::new(seen.clone(), RecentReply::Complete),
            directory.path(),
        );
        let target = LearningTarget::Explicit(catalog().resolve("EL").expect("Greek must resolve"));
        for recent in ["FR", "EN"] {
            let input =
                RawInputBatch::new("сумка").with_learning_history(vec![String::from(recent)]);
            cache
                .understand(&input, "RU", &target)
                .expect("explicit translations must succeed");
        }
        assert_eq!(
            seen.borrow().len(),
            1,
            "recent history defeated explicit per-entry cache reuse"
        );
    }

    #[test]
    fn recent_language_context_cannot_disappear_from_chunks_or_retries() {
        let directory = TempDir::new().expect("tempdir must be created");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let cache = CachedUnderstanding::new(
            RecentUnderstanding::new(seen.clone(), RecentReply::ShortFirst),
            directory.path(),
        );
        let input = RawInputBatch::new(
            (0..=INTAKE_CHUNK_WORDS)
                .map(|index| format!("слово-{index}"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .with_learning_history(vec![String::from("FR"), String::from("EN")]);
        let understood = cache
            .understand(&input, "RU", &LearningTarget::Detect)
            .expect("recent translation chunks must succeed");
        assert_eq!(
            (
                seen.borrow()
                    .iter()
                    .map(RawInputBatch::word_count)
                    .collect::<Vec<_>>(),
                seen.borrow()
                    .iter()
                    .all(|chunk| chunk.learning_history() == input.learning_history()
                        && chunk.context() == Some(input.text())),
                understood.guess().requires_confirmation()
            ),
            (vec![INTAKE_CHUNK_WORDS, INTAKE_CHUNK_WORDS, 1], true, true),
            "chunking or retry lost recent languages, batch context, or confirmation"
        );
    }

    #[test]
    fn mixed_input_cannot_be_presented_as_a_recent_language_proposal() {
        let directory = TempDir::new().expect("tempdir must be created");
        let cache = CachedUnderstanding::new(
            RecentUnderstanding::new(
                Rc::new(RefCell::new(Vec::new())),
                RecentReply::ForceConfirmation,
            ),
            directory.path(),
        );
        let input =
            RawInputBatch::new("сумка\ncat").with_learning_history(vec![String::from("EN")]);
        assert!(
            cache
                .understand(&input, "RU", &LearningTarget::Detect)
                .is_err(),
            "mixed input was incorrectly offered as a recent-language fallback"
        );
    }

    #[test]
    fn recent_history_cannot_override_foreign_words_in_the_complete_batch() {
        let directory = TempDir::new().expect("tempdir must be created");
        let cache = CachedUnderstanding::new(
            RecentUnderstanding::new(Rc::new(RefCell::new(Vec::new())), RecentReply::Complete),
            directory.path(),
        );
        let input =
            RawInputBatch::new(mixed_blob()).with_learning_history(vec![String::from("FR")]);
        let understood = cache
            .understand(&input, "RU", &LearningTarget::Detect)
            .expect("foreign words must determine the learning language");
        assert_eq!(
            (
                understood.guess().code(),
                understood.guess().requires_confirmation()
            ),
            ("EN", false),
            "recent language displaced foreign-word evidence or added confirmation to mixed input"
        );
    }

    #[test]
    fn chunks_cannot_disagree_about_recent_language_confirmation() {
        let directory = TempDir::new().expect("tempdir must be created");
        let cache = CachedUnderstanding::new(
            RecentUnderstanding::new(
                Rc::new(RefCell::new(Vec::new())),
                RecentReply::SwitchConfirmation,
            ),
            directory.path(),
        );
        let input = RawInputBatch::new(
            (0..=INTAKE_CHUNK_WORDS)
                .map(|index| format!("слово-{index}"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .with_learning_history(vec![String::from("FR")]);
        assert!(
            cache
                .understand(&input, "RU", &LearningTarget::Detect)
                .is_err(),
            "conflicting chunk confirmation flags were silently combined"
        );
    }

    #[test]
    fn recent_proposals_cannot_name_an_older_or_missing_learning_language() {
        let candidates = vec![WordCandidate::with_senses(
            "сумка",
            vec![Sense::translated("sac", "Вещь для переноски", None)],
            0,
            true,
        )];
        let guess = LearningGuess::new("FR", true).with_confirmation(true);
        let refused = [Vec::new(), vec![String::from("EN"), String::from("FR")]]
            .iter()
            .all(|history| validate_detection(&guess, &candidates, "RU", history).is_err());
        assert!(
            refused,
            "a recent proposal invented a destination or skipped the latest eligible language"
        );
    }

    #[test]
    fn recent_proposals_cannot_be_blocked_by_ineligible_history_entries() {
        let candidates = vec![WordCandidate::with_senses(
            "сумка",
            vec![Sense::translated("sac", "Вещь для переноски", None)],
            0,
            true,
        )];
        let guess = LearningGuess::new("FR", true).with_confirmation(true);
        let history = ["ru", "unknown", "fr", "EN"].map(String::from);
        assert!(
            validate_detection(&guess, &candidates, "RU", &history).is_ok(),
            "known or unsupported history entries displaced the latest eligible language"
        );
    }

    #[test]
    fn automatic_native_only_batch_cannot_invent_a_learning_language() {
        let directory = TempDir::new().expect("tempdir must be created");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let cache = CachedUnderstanding::new(
            ContextUnderstanding::new(seen.clone(), ContextReply::Complete),
            directory.path(),
        );
        let input = RawInputBatch::new("сумка\nдорога");
        let first = cache.understand(&input, "RU", &LearningTarget::Detect);
        let second = cache.understand(&input, "RU", &LearningTarget::Detect);
        assert_eq!(
            (
                first.is_err_and(|error| error.is::<LearningLanguageRequired>()),
                second.is_err_and(|error| error.is::<LearningLanguageRequired>()),
                seen.borrow().len(),
            ),
            (true, true, 1),
            "native-only input invented a destination through the cache wrapper or on cache replay"
        );
    }

    #[test]
    fn a_support_only_chunk_cannot_lose_the_learning_language_evidence() {
        let directory = TempDir::new().expect("tempdir must be created");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let cache = CachedUnderstanding::new(
            ContextUnderstanding::new(seen.clone(), ContextReply::Complete),
            directory.path(),
        );
        let input = mixed_blob();
        let understood = cache
            .understand(&RawInputBatch::new(&input), "RU", &LearningTarget::Detect)
            .expect("support-only chunks must use the complete batch context");
        assert_eq!(
            (
                understood.guess().code(),
                seen.borrow()
                    .iter()
                    .map(RawInputBatch::word_count)
                    .collect::<Vec<_>>(),
                seen.borrow()
                    .iter()
                    .all(|raw| raw.context() == Some(input.as_str())),
            ),
            ("EN", vec![INTAKE_CHUNK_WORDS, 1], true),
            "chunking lost target-language evidence or increased the provider row count"
        );
    }

    #[test]
    fn a_short_reply_retry_cannot_drop_the_complete_language_context() {
        let directory = TempDir::new().expect("tempdir must be created");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let cache = CachedUnderstanding::new(
            ContextUnderstanding::new(seen.clone(), ContextReply::ShortFirst),
            directory.path(),
        );
        let input = mixed_blob();
        cache
            .understand(&RawInputBatch::new(&input), "RU", &LearningTarget::Detect)
            .expect("a short chunk must be retried");
        assert_eq!(
            (
                seen.borrow()
                    .iter()
                    .map(RawInputBatch::word_count)
                    .collect::<Vec<_>>(),
                seen.borrow()
                    .iter()
                    .all(|raw| raw.context() == Some(input.as_str())),
            ),
            (vec![INTAKE_CHUNK_WORDS, INTAKE_CHUNK_WORDS, 1], true),
            "retry discarded the full batch context or re-requested unrelated rows"
        );
    }

    #[test]
    fn partially_cached_input_cannot_hide_the_learning_language_evidence() {
        let directory = TempDir::new().expect("tempdir must be created");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let cache = CachedUnderstanding::new(
            ContextUnderstanding::new(seen.clone(), ContextReply::FailSecond),
            directory.path(),
        );
        let input = format!("cat\n{}\nпоследнее", blob(INTAKE_CHUNK_WORDS - 1));
        let first = cache.understand(&RawInputBatch::new(&input), "RU", &LearningTarget::Detect);
        let retried = cache
            .understand(&RawInputBatch::new(&input), "RU", &LearningTarget::Detect)
            .expect("partial cache must resume the missing chunk");
        assert_eq!(
            (
                first.is_err(),
                retried.guess().code(),
                seen.borrow()
                    .iter()
                    .map(RawInputBatch::word_count)
                    .collect::<Vec<_>>(),
                seen.borrow().last().and_then(RawInputBatch::context) == Some(input.as_str()),
            ),
            (true, "EN", vec![INTAKE_CHUNK_WORDS, 1, 1], true),
            "partial cache hid the target-language evidence or re-requested completed rows"
        );
    }

    #[test]
    fn automatic_chunks_cannot_mix_different_learning_languages() {
        let directory = TempDir::new().expect("tempdir must be created");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let cache = CachedUnderstanding::new(
            ContextUnderstanding::new(seen, ContextReply::SwitchTarget),
            directory.path(),
        );
        assert!(
            cache
                .understand(
                    &RawInputBatch::new(mixed_blob()),
                    "RU",
                    &LearningTarget::Detect
                )
                .expect_err("conflicting chunk languages must require a choice")
                .is::<LearningLanguageRequired>(),
            "understanding merged candidates from conflicting target languages"
        );
    }

    #[test]
    fn cached_candidates_cannot_mix_different_learning_languages() {
        let directory = TempDir::new().expect("tempdir must be created");
        let cache = CachedUnderstanding::new(
            ContextUnderstanding::new(Rc::new(RefCell::new(Vec::new())), ContextReply::Complete),
            directory.path(),
        );
        let input = "cat\nсумка";
        cache
            .understand(&RawInputBatch::new(input), "RU", &LearningTarget::Detect)
            .expect("mixed input must be understood");
        let namespace = Cache::new("understanding/RU-RU", directory.path());
        let identity = format!("detect:RU:{}", digest(&[input, ""]));
        let filename = cache.entry_filename("сумка", "RU", &identity);
        write_json(
            &namespace,
            &filename,
            &EntryRecord::from_candidate(
                &LearningGuess::new("FR", true),
                &WordCandidate::new("сумка", "FR", true),
            ),
        )
        .expect("a conflicting cache record must be seeded");
        assert!(
            cache
                .understand(&RawInputBatch::new(input), "RU", &LearningTarget::Detect)
                .expect_err("conflicting cached languages must require a choice")
                .is::<LearningLanguageRequired>(),
            "understanding silently merged cache entries for different target languages"
        );
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
    fn recent_proposals_cannot_reuse_the_previous_intake_policy() {
        let cache = CachedUnderstanding::new(
            ChangingUnderstanding::new(Rc::new(RefCell::new(0))),
            "/tmp/kamishibai-understanding-version-test",
        );
        let current = cache.entry_filename("lantern", "RU", "detect:EN");
        let previous = format!("{}.json", digest(&["v10", "RU", "detect:EN", "lantern"]));
        assert_eq!(
            (UNDERSTANDING_VERSION, current != previous),
            ("v11", true),
            "recent proposals reused an intake policy without history or confirmation"
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
            (
                record.guess().alternates(),
                record.guess().requires_confirmation()
            ),
            (Vec::<String>::new().as_slice(), false),
            "a legacy entry invented alternates or a recent-language confirmation"
        );
    }

    #[test]
    fn automatic_batches_cannot_reuse_an_entry_from_a_different_language_context() {
        let directory = TempDir::new().expect("tempdir must be created");
        let cache = CachedUnderstanding::new(
            ContextUnderstanding::new(Rc::new(RefCell::new(Vec::new())), ContextReply::Complete),
            directory.path(),
        );
        cache
            .understand(
                &RawInputBatch::new("cat\nсумка"),
                "RU",
                &LearningTarget::Detect,
            )
            .expect("English context must be understood");
        let second = cache
            .understand(
                &RawInputBatch::new("chat\nсумка"),
                "RU",
                &LearningTarget::Detect,
            )
            .expect("French context must be understood independently");
        assert_eq!(
            (
                second.guess().code(),
                second.candidates()[1].understanding()
            ),
            ("FR", "FR"),
            "automatic understanding reused a translation from another batch context"
        );
    }

    #[test]
    fn cached_entry_is_reused_when_a_new_entry_is_added_to_the_batch() {
        let directory = TempDir::new().expect("tempdir must be created");
        let calls = Rc::new(RefCell::new(0));
        let cache =
            CachedUnderstanding::new(ChangingUnderstanding::new(calls.clone()), directory.path());
        let target =
            LearningTarget::Explicit(catalog().resolve("EN").expect("English must resolve"));
        cache
            .understand(&RawInputBatch::new("catdog"), "ru", &target)
            .expect("first understanding must succeed");
        let second = cache
            .understand(&RawInputBatch::new("catdog\nflower"), "ru", &target)
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
    fn cached_card_context_is_compacted_without_rewriting_the_cache_or_other_fields() {
        let directory = TempDir::new().expect("tempdir must be created");
        let pair = LanguagePair::new("en", "ru");
        let cache = CardMetaCache::new(directory.path());
        let context = "**Meaning.**\n- **approval**\n- a benefit\nrecap: one is approval and the other is a benefit.\n\n**Usage.**\nCongratulate a friend.\n\n**Limits.**\nA dry reply can sound dismissive.\n\n**Nuance.**\nWarm intonation helps convey sincerity.";
        let expected = "**Meaning.**\n- **approval**\n- a benefit\n\n**Usage.**\nCongratulate a friend.\n\n**Limits.**\nA dry reply can sound dismissive.\n\n**Nuance.**\nWarm intonation helps convey sincerity.";
        let original = meta("Good for you!").with_source_context(context);
        let (_, path, _) = cache
            .store("good for you", "approval", &pair, &original)
            .expect("meta must store");
        let bytes = fs::read(&path).expect("cached bytes must be readable");
        let loaded = cache
            .load("good for you", "approval", &pair)
            .expect("meta must load");
        assert_eq!(
            (
                loaded,
                fs::read(&path).expect("cached bytes must remain readable")
            ),
            (Some(original.with_source_context(expected)), bytes),
            "context compaction changed unrelated metadata or wrote to the saved cache"
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
            .insert(
                String::from("policy"),
                serde_json::json!("v10-explained-and-illustrated-usage"),
            );
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
            .load_current_at(&CardCell::new(
                directory.path(),
                &pair,
                "lantern",
                "a portable lamp",
            ))
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
