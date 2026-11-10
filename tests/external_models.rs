//! Verify external providers use canonical prompts, validation, and accounting.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use kamishibai::application::{LearningTarget, Understanding};
use kamishibai::models::{GenerationStage, ModelOutput, ModelProvider, ModelReply, ModelRequest};
use kamishibai::session::{CostRecord, RawInputBatch};
use tempfile::TempDir;

struct External {
    stages: Mutex<Vec<GenerationStage>>,
}

impl External {
    fn new() -> Self {
        Self {
            stages: Mutex::new(Vec::new()),
        }
    }
}

impl ModelProvider for External {
    fn generate(&self, request: ModelRequest) -> Result<ModelReply> {
        let ModelRequest::Text { stage, prompt, .. } = request else {
            anyhow::bail!("unexpected non-text request")
        };
        if !prompt.contains("canard") {
            anyhow::bail!("canonical prompt lost the supplied vocabulary")
        }
        self.stages
            .lock()
            .expect("stage lock must remain available")
            .push(stage);
        Ok(ModelReply::new(
            ModelOutput::Text {
                text: String::from(
                    r#"{"target_lang":"FR","items":[{"term":"canard","senses":[{"understanding":"a duck"}],"selected":0,"ok":true}]}"#,
                ),
                truncated: false,
            },
            CostRecord::unreported("external/text"),
        ))
    }
}

#[test]
fn external_provider_cannot_bypass_canonical_understanding_and_cache() {
    let directory = TempDir::new().expect("temporary job root must exist");
    let provider = Arc::new(External::new());
    let workflow = kamishibai::models::workflow(
        provider.clone(),
        kamishibai::models::Parts::default(),
        "external-text-v1",
        directory.path().join("cache"),
        directory.path().join("output"),
        None,
    )
    .expect("external workflow must compose");
    let target = LearningTarget::Explicit(
        kamishibai::languages::catalog()
            .resolve("FR")
            .expect("French must resolve"),
    );
    let raw = RawInputBatch::new("canard");
    workflow
        .understand(&raw, "EN", &target)
        .expect("canonical understanding must decode");
    workflow
        .understand(&raw, "EN", &target)
        .expect("understanding must reuse cache");
    assert_eq!(
        provider
            .stages
            .lock()
            .expect("stage lock must remain available")
            .as_slice(),
        &[GenerationStage::Intake],
        "external workflow bypassed the canonical understanding cache"
    );
}

struct Complete {
    stages: Mutex<Vec<GenerationStage>>,
}

impl Complete {
    fn new() -> Self {
        Self {
            stages: Mutex::new(Vec::new()),
        }
    }
}

impl ModelProvider for Complete {
    fn generate(&self, request: ModelRequest) -> Result<ModelReply> {
        let (stage, output) = match request {
            ModelRequest::Text {
                stage,
                images,
                options,
                ..
            } => {
                let (format, _, _, _) = options.into_parts();
                if stage == GenerationStage::Features
                    && !matches!(format, kamishibai::models::TextFormat::Schema(_))
                {
                    anyhow::bail!("canonical scene schema was lost")
                }
                if matches!(
                    stage,
                    GenerationStage::Text
                        | GenerationStage::Recall
                        | GenerationStage::Fidelity
                        | GenerationStage::Zoom
                ) && images.is_empty()
                {
                    anyhow::bail!("canonical image inspection lost its visual evidence")
                }
                let value = match stage {
                    GenerationStage::Intake => {
                        serde_json::json!({"target_lang":"VI","items":[{"term":"mèo","senses":[{"understanding":"a cat"}],"selected":0,"ok":true}]})
                    }
                    GenerationStage::Metadata => serde_json::json!({
                        "pronunciation":"mɛw", "transcription":"kɔn mɛw ŋu",
                        "meaning":"a cat", "importance":5,
                        "source_sentence":"The cat sleeps on the windowsill", "source_highlight":"cat",
                        "source_hint":"A furry household animal", "source_context":"A concrete animal noun",
                        "target_sentence":"Con mèo ngủ trên bậu cửa sổ",
                        "labels":{"register":"neutral","level":"a1","type":"statement","approx":[]}
                    }),
                    GenerationStage::Phonetics => {
                        serde_json::json!({"pronunciation":"mɛw", "transcription":"kɔn mɛw ŋu"})
                    }
                    GenerationStage::Features => super::gemini::scene_features(),
                    GenerationStage::Scene => super::gemini::dynamic_scene(),
                    GenerationStage::Text => {
                        serde_json::json!({"decision":"ALLOW","evidence":[],"reason":"No writing is visible"})
                    }
                    GenerationStage::Recall => {
                        serde_json::json!({"decision":"ALLOW","evidence":[],"literal_writing_present":false,"literal_evidence":[],"page_frame":"FRAMED","reason":"No answer-bearing writing is visible"})
                    }
                    GenerationStage::Fidelity => {
                        serde_json::json!({"scene_fidelity_decision":"ALLOW","scene_fidelity_evidence":[],"reason":"The scene remains visible"})
                    }
                    GenerationStage::Zoom => {
                        serde_json::json!({"literal_writing_present":false,"literal_evidence":[],"reason":"No writing is visible at any scale"})
                    }
                    other => anyhow::bail!("unexpected text stage {other:?}"),
                };
                (
                    stage,
                    ModelOutput::Text {
                        text: value.to_string(),
                        truncated: false,
                    },
                )
            }
            ModelRequest::Image {
                prompt,
                aspect_ratio,
            } => {
                if prompt.is_empty() || aspect_ratio != "1:1" {
                    anyhow::bail!("canonical illustration contract was lost")
                }
                (
                    GenerationStage::Picture,
                    ModelOutput::Image(framed_image()?),
                )
            }
            ModelRequest::Speech { prompt, text } => {
                if !prompt.contains("Vietnamese") || text != "Con mèo ngủ trên bậu cửa sổ"
                {
                    anyhow::bail!("canonical pronunciation contract was lost")
                }
                (
                    GenerationStage::Speech,
                    ModelOutput::Speech(vec![17, 0, 31, 0, 29, 0, 0, 0]),
                )
            }
        };
        self.stages
            .lock()
            .expect("stage lock must remain available")
            .push(stage);
        Ok(ModelReply::new(
            output,
            CostRecord::unreported(format!("external/{stage:?}")),
        ))
    }
}

fn framed_image() -> Result<Vec<u8>> {
    let mut image = image::GrayImage::from_pixel(1024, 1024, image::Luma([255]));
    for y in 20..1004 {
        for x in 20..1004 {
            if !(28..996).contains(&x)
                || !(28..996).contains(&y)
                || ((390..620).contains(&x) && (440..615).contains(&y))
            {
                image.put_pixel(x, y, image::Luma([0]));
            }
        }
    }
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(image).write_to(&mut bytes, image::ImageFormat::Png)?;
    Ok(bytes.into_inner())
}

struct Quiet;

impl kamishibai::application::PublishProgress for Quiet {
    fn advance(&self, _phase: kamishibai::application::PublishPhase) {}
}

#[test]
fn external_provider_cannot_skip_canonical_artifacts_inspections_or_publication() {
    use kamishibai::application::{GenerationRun, StudyPublishing};
    let directory = TempDir::new().expect("temporary job root must exist");
    let provider = Arc::new(Complete::new());
    let workflow = kamishibai::models::workflow(
        provider.clone(),
        kamishibai::models::Parts::default(),
        "complete-v1",
        directory.path().join("cache"),
        directory.path().join("output"),
        None,
    )
    .expect("external workflow must compose");
    let target = LearningTarget::Explicit(
        kamishibai::languages::catalog()
            .resolve("VI")
            .expect("Vietnamese must resolve"),
    );
    let understood = workflow
        .understand(&RawInputBatch::new("mèo"), "EN", &target)
        .expect("canonical understanding must decode");
    let drafts = kamishibai::application::drafts_for(
        understood.candidates(),
        &kamishibai::session::LanguagePair::new("VI", "EN"),
    );
    let mut run = GenerationRun::new(drafts).expect("canonical run must start");
    while let Some(step) = run
        .advance(&workflow)
        .expect("canonical artifact must settle")
    {
        let (_, artifact, _, error) = step.into_parts();
        if let Some(error) = error {
            panic!("external canonical {artifact:?} failed: {error}")
        }
    }
    let package = workflow
        .publish(run.drafts(), &Quiet)
        .expect("completed external artifacts must publish");
    let (deck, report, _) = package.into_paths();
    let stages = provider
        .stages
        .lock()
        .expect("stage lock must remain available");
    assert!(
        [
            GenerationStage::Intake,
            GenerationStage::Metadata,
            GenerationStage::Phonetics,
            GenerationStage::Speech,
            GenerationStage::Features,
            GenerationStage::Scene,
            GenerationStage::Picture,
            GenerationStage::Text,
            GenerationStage::Recall,
            GenerationStage::Fidelity,
            GenerationStage::Zoom
        ]
        .iter()
        .all(|stage| stages.contains(stage))
            && run
                .drafts()
                .iter()
                .all(|draft| draft.artifacts().all_ready())
            && run.drafts().iter().all(|draft| draft
                .artifacts()
                .cost()
                .is_some_and(|cost| cost.total().is_none()))
            && std::path::Path::new(&deck).is_file()
            && std::path::Path::new(&report).is_file(),
        "external generation skipped a canonical stage, lost unpriced usage, or failed to publish both study artifacts"
    );
}

struct Invalid;

impl ModelProvider for Invalid {
    fn generate(&self, _request: ModelRequest) -> Result<ModelReply> {
        Ok(ModelReply::new(
            ModelOutput::Text {
                text: String::from("not valid metadata"),
                truncated: false,
            },
            CostRecord::new(
                "external/unknown",
                1,
                73,
                19,
                92,
                kamishibai::session::GenerationCost::unknown(),
            ),
        ))
    }
}

struct Ledger {
    records: Mutex<Vec<CostRecord>>,
}

impl Ledger {
    fn new() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
        }
    }
}

impl kamishibai::application::GenerationCostLedger for Ledger {
    fn record(
        &self,
        _scope: kamishibai::application::GenerationScope,
        usage: &CostRecord,
    ) -> Result<()> {
        self.records
            .lock()
            .expect("ledger must remain available")
            .push(usage.clone());
        Ok(())
    }
}

#[test]
fn rejected_external_output_cannot_erase_measured_usage_or_invent_a_price() {
    use kamishibai::application::CardProduction;
    let directory = TempDir::new().expect("temporary job root must exist");
    let ledger = Arc::new(Ledger::new());
    let workflow = kamishibai::models::workflow(
        Arc::new(Invalid),
        kamishibai::models::Parts::default(),
        "invalid-v1",
        directory.path().join("cache"),
        directory.path().join("output"),
        Some(ledger.clone()),
    )
    .expect("external workflow must compose");
    let draft = kamishibai::session::CardDraft::new(
        "canard",
        "a duck",
        kamishibai::session::LanguagePair::new("FR", "EN"),
    );
    let attempt = workflow.generate_draft_meta_in(3, &draft);
    let records = ledger.records.lock().expect("ledger must remain available");
    assert!(
        attempt.into_result().is_err()
            && records.len() == 1
            && records[0].requests() == 1
            && records[0].cost().total().is_none(),
        "rejected external metadata lost its paid request or invented a complete price"
    );
}

#[test]
fn changed_external_policy_cannot_reuse_another_policy_cache() {
    let directory = TempDir::new().expect("temporary job root must exist");
    let first = kamishibai::models::cache_root(directory.path(), "text-v3;picture-v4")
        .expect("first identity must validate");
    let second = kamishibai::models::cache_root(directory.path(), "text-v5;picture-v4")
        .expect("second identity must validate");
    assert!(
        first != second && first.parent() == second.parent(),
        "a changed external model reused another model's mutable artifacts"
    );
}
