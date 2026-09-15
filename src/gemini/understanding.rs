//! Gemini implementation of the word-understanding use case.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;

use super::{GeminiAccess, GeminiClient, Transport};
use crate::application::{
    BulkCorrection, GenerationCostLedger, GenerationScope, LearningTarget, Understanding,
};
use crate::generation::artifact_cache::{Cache, ROOT_STAGE_LOCK_TIMEOUT, RootStage};
use crate::session::{
    CachedUnderstanding, CostRecord, LanguagePair, RawInputBatch, SenseCorrection, Understood,
    WordCandidate,
};

/// Understands words through Gemini while reusing the understanding cache.
#[derive(Clone)]
pub(crate) struct GeminiUnderstanding {
    access: GeminiAccess,
    cache: PathBuf,
    ledger: Option<Arc<dyn GenerationCostLedger>>,
}

impl GeminiUnderstanding {
    /// Bind Gemini access to the shared cache root.
    #[must_use]
    pub(crate) fn new(
        access: GeminiAccess,
        cache: PathBuf,
        ledger: Option<Arc<dyn GenerationCostLedger>>,
    ) -> Self {
        Self {
            access,
            cache,
            ledger,
        }
    }
}

struct ObservedUnderstanding<T> {
    client: GeminiClient<T>,
    usage: InputUsage,
}

impl<T> ObservedUnderstanding<T> {
    fn new(client: GeminiClient<T>, usage: InputUsage) -> Self {
        Self { client, usage }
    }
}

impl<T: Transport> Understanding for ObservedUnderstanding<T> {
    fn understand(
        &self,
        raw: &RawInputBatch,
        known: &str,
        target: &LearningTarget,
    ) -> Result<Understood> {
        self.client
            .understand_observed(raw, known, target, |record| {
                self.usage.record(GenerationScope::Intake, &record)
            })
    }
}

struct InputUsage {
    cache: PathBuf,
    ledger: Option<Arc<dyn GenerationCostLedger>>,
}

impl InputUsage {
    fn new(cache: PathBuf, ledger: Option<Arc<dyn GenerationCostLedger>>) -> Self {
        Self { cache, ledger }
    }

    fn record(&self, scope: GenerationScope, record: &CostRecord) -> Result<()> {
        if let Some(ledger) = &self.ledger {
            ledger.record(scope, record)?;
        }
        let filename = match scope {
            GenerationScope::Intake => "intake.json",
            GenerationScope::Senses => "senses.json",
            GenerationScope::Card { .. } => {
                anyhow::bail!("input usage cannot contain card artifact scope")
            }
        };
        let cache = Cache::new("usage", self.cache.clone());
        let _guard = cache.hold_root_stage(RootStage::Meta, ROOT_STAGE_LOCK_TIMEOUT)?;
        let path = cache.filepath(filename)?;
        let merged = if path.exists() {
            serde_json::from_slice::<CostRecord>(&fs::read(path)?)?.merged(record)
        } else {
            record.clone()
        };
        let staged = cache.stage(".usage.json")?;
        let result = serde_json::to_vec_pretty(&merged)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| fs::write(&staged, bytes).map_err(anyhow::Error::from))
            .and_then(|()| cache.commit(&staged, filename));
        if result.is_err() {
            let _ = fs::remove_file(staged);
        }
        result
    }
}

impl Understanding for GeminiUnderstanding {
    fn understand(
        &self,
        raw: &RawInputBatch,
        known: &str,
        target: &LearningTarget,
    ) -> Result<Understood> {
        let usage = InputUsage::new(self.cache.clone(), self.ledger.clone());
        CachedUnderstanding::new(
            ObservedUnderstanding::new(self.access.client()?, usage),
            self.cache.clone(),
        )
        .understand(raw, known, target)
    }
}

impl BulkCorrection for GeminiUnderstanding {
    fn correct_bulk(
        &self,
        candidate: &WordCandidate,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<SenseCorrection> {
        let usage = InputUsage::new(self.cache.clone(), self.ledger.clone());
        self.access
            .client()?
            .correct_bulk_observed(candidate, comment, pair, |record| {
                usage.record(GenerationScope::Senses, &record)
            })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use serde_json::{Value, json};
    use tempfile::TempDir;

    use super::*;
    use crate::gemini::{GeminiProfile, GenerationStage, StageModels, TransportResponse};
    use crate::session::GenerationCost;

    #[derive(Clone)]
    struct Scripted {
        replies: Arc<Mutex<VecDeque<String>>>,
        calls: Arc<AtomicUsize>,
    }

    impl Scripted {
        fn new(replies: Vec<String>, calls: Arc<AtomicUsize>) -> Self {
            Self {
                replies: Arc::new(Mutex::new(replies.into())),
                calls,
            }
        }
    }

    impl Transport for Scripted {
        fn post(&self, _url: &str, _key: &str, _body: &str) -> Result<TransportResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let text = self
                .replies
                .lock()
                .map_err(|_| anyhow::anyhow!("reply queue poisoned"))?
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("no scripted reply"))?;
            Ok(TransportResponse { status: 200, body: json!({
                "candidates": [{"content": {"parts": [{"text": text}]}}],
                "usageMetadata": {"promptTokenCount": 137, "candidatesTokenCount": 59, "thoughtsTokenCount": 11, "totalTokenCount":207}
            }).to_string() })
        }
    }

    #[derive(Default)]
    struct UsageLedger {
        entries: Mutex<Vec<(GenerationScope, CostRecord)>>,
    }

    impl GenerationCostLedger for UsageLedger {
        fn record(&self, scope: GenerationScope, record: &CostRecord) -> Result<()> {
            self.entries
                .lock()
                .map_err(|_| anyhow::anyhow!("usage ledger poisoned"))?
                .push((scope, record.clone()));
            Ok(())
        }
    }

    fn client(replies: Vec<String>, calls: Arc<AtomicUsize>) -> GeminiClient<Scripted> {
        let models = StageModels::default()
            .with_model(GenerationStage::Intake, "unpriced-intake")
            .and_then(|models| models.with_model(GenerationStage::Senses, "unpriced-senses"))
            .expect("test model selection must validate");
        let profile = GeminiProfile::from_models("https://unused.example/models", models)
            .expect("test profile must validate");
        GeminiClient::from_profile("unused-test-key", Scripted::new(replies, calls), profile)
    }

    #[test]
    fn cached_intake_cannot_erase_or_repeat_its_original_unpriced_usage() {
        let directory = TempDir::new().expect("cache must exist");
        let calls = Arc::new(AtomicUsize::new(0));
        let ledger = Arc::new(UsageLedger::default());
        let reply = json!({"target_lang":"FR", "items":[{"term":"canard", "senses":[{"understanding":"a duck"}], "selected":0, "ok":true}]}).to_string();
        let inner = ObservedUnderstanding::new(
            client(vec![reply], calls.clone()),
            InputUsage::new(directory.path().to_path_buf(), Some(ledger.clone())),
        );
        let cache = CachedUnderstanding::new(inner, directory.path());
        let target = LearningTarget::Explicit(
            crate::languages::catalog()
                .resolve("FR")
                .expect("French must resolve"),
        );
        let raw = RawInputBatch::new("canard");
        cache
            .understand(&raw, "EN", &target)
            .expect("fresh intake must decode");
        cache
            .understand(&raw, "EN", &target)
            .expect("cached intake must decode");
        let stored: Value = serde_json::from_slice(
            &fs::read(directory.path().join("usage/intake.json")).expect("usage must persist"),
        )
        .expect("usage must decode");
        assert_eq!(
            (
                calls.load(Ordering::SeqCst),
                ledger.entries.lock().expect("ledger must lock").clone(),
                stored["total_tokens"].as_u64()
            ),
            (
                1,
                vec![(
                    GenerationScope::Intake,
                    CostRecord::new(
                        "unpriced-intake",
                        1,
                        137,
                        70,
                        207,
                        GenerationCost::unknown()
                    )
                )],
                Some(207)
            ),
            "cached intake erased the actual request usage or charged it again"
        );
    }

    #[test]
    fn invalid_sense_payload_keeps_its_request_in_the_ledger_and_cache() {
        let directory = TempDir::new().expect("cache must exist");
        let calls = Arc::new(AtomicUsize::new(0));
        let ledger = Arc::new(UsageLedger::default());
        let usage = InputUsage::new(directory.path().to_path_buf(), Some(ledger.clone()));
        let result = client(vec![String::from("{broken")], calls.clone()).correct_bulk_observed(
            &WordCandidate::new("canard", "a duck", true),
            "Add another sense",
            &LanguagePair::new("FR", "EN"),
            |record| usage.record(GenerationScope::Senses, &record),
        );
        let stored: CostRecord = serde_json::from_slice(
            &fs::read(directory.path().join("usage/senses.json"))
                .expect("failed correction usage must persist"),
        )
        .expect("usage must decode");
        assert_eq!(
            (
                result.is_err(),
                calls.load(Ordering::SeqCst),
                ledger.entries.lock().expect("ledger must lock").clone(),
                stored.requests()
            ),
            (
                true,
                1,
                vec![(
                    GenerationScope::Senses,
                    CostRecord::new(
                        "unpriced-senses",
                        1,
                        137,
                        70,
                        207,
                        GenerationCost::unknown()
                    )
                )],
                1
            ),
            "payload decoding discarded the sense-correction request and its usage"
        );
    }

    struct FailingLedger;

    impl GenerationCostLedger for FailingLedger {
        fn record(&self, _scope: GenerationScope, _record: &CostRecord) -> Result<()> {
            anyhow::bail!("ledger unavailable")
        }
    }

    #[test]
    fn intake_cannot_continue_to_another_chunk_after_ledger_failure() {
        let directory = TempDir::new().expect("cache must exist");
        let calls = Arc::new(AtomicUsize::new(0));
        let inner = ObservedUnderstanding::new(
            client(vec![String::from("{}"), String::from("{}")], calls.clone()),
            InputUsage::new(
                directory.path().to_path_buf(),
                Some(Arc::new(FailingLedger)),
            ),
        );
        let cache = CachedUnderstanding::new(inner, directory.path());
        let raw = RawInputBatch::new(
            (0..21)
                .map(|index| format!("terme-{index}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let target = LearningTarget::Explicit(
            crate::languages::catalog()
                .resolve("FR")
                .expect("French must resolve"),
        );
        let result = cache.understand(&raw, "EN", &target);
        assert!(
            result.is_err() && calls.load(Ordering::SeqCst) == 1,
            "intake spent another request after accounting failed"
        );
    }
}
