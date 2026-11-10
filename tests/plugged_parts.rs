//! A stranger's parts: their own model, picture style, and gentle world, plugged without touching the core.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use kamishibai::application::{GenerationRun, LearningTarget, Understanding};
use kamishibai::models::{
    GenerationStage, Look, ModelOutput, ModelProvider, ModelReply, ModelRequest, Parts,
    PictureStyle, Resend, SoftWorld,
};
use kamishibai::session::{CardDraft, CostRecord, RawInputBatch};
use serde_json::Value;
use tempfile::TempDir;

struct Watercolor;

impl PictureStyle for Watercolor {
    fn name(&self) -> &str {
        "watercolor-v1"
    }

    fn prompt(&self, scene: &Value) -> Result<String> {
        let panels = scene
            .pointer("/manga_panel/panels")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        Ok(format!("WATERCOLOR wash of {panels} framed scenes"))
    }
}

struct Cranes {
    admitted: Mutex<Vec<String>>,
}

impl Cranes {
    fn new() -> Self {
        Self {
            admitted: Mutex::new(Vec::new()),
        }
    }
}

impl SoftWorld for Cranes {
    fn name(&self) -> &str {
        "cranes-v1"
    }

    fn brief(&self) -> &str {
        "SOFT WORLD\nOnly folded paper cranes act here."
    }

    fn admit(&self, _scene: &Value, prompt: &str) -> Result<()> {
        self.admitted
            .lock()
            .expect("admission log must remain available")
            .push(String::from(prompt));
        Ok(())
    }
}

struct Studio {
    refusals: Mutex<u8>,
    seen: Mutex<Vec<(GenerationStage, String)>>,
}

impl Studio {
    fn refusing(pictures: u8) -> Self {
        Self {
            refusals: Mutex::new(pictures),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn prompts(&self, stage: GenerationStage) -> Vec<String> {
        self.seen
            .lock()
            .expect("request log must remain available")
            .iter()
            .filter(|(seen, _)| *seen == stage)
            .map(|(_, prompt)| prompt.clone())
            .collect()
    }
}

impl ModelProvider for Studio {
    fn generate(&self, request: ModelRequest) -> Result<ModelReply> {
        let (stage, prompt, output) = match request {
            ModelRequest::Text { stage, prompt, .. } => {
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
                    prompt,
                    ModelOutput::Text {
                        text: value.to_string(),
                        truncated: false,
                    },
                )
            }
            ModelRequest::Image { prompt, .. } => {
                let mut refusals = self
                    .refusals
                    .lock()
                    .expect("refusal counter must remain available");
                let output = if *refusals > 0 {
                    *refusals -= 1;
                    ModelOutput::Refused
                } else {
                    ModelOutput::Image(framed()?)
                };
                (GenerationStage::Picture, prompt, output)
            }
            ModelRequest::Speech { prompt, .. } => (
                GenerationStage::Speech,
                prompt,
                ModelOutput::Speech(vec![17, 0, 31, 0, 29, 0, 0, 0]),
            ),
        };
        self.seen
            .lock()
            .expect("request log must remain available")
            .push((stage, prompt));
        Ok(ModelReply::new(
            output,
            CostRecord::unreported(format!("studio/{stage:?}")),
        ))
    }
}

fn framed() -> Result<Vec<u8>> {
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

fn produced(studio: Arc<Studio>, look: Look, directory: &TempDir) -> Vec<CardDraft> {
    let workflow = kamishibai::models::workflow(
        studio,
        Parts::new(look, Arc::new(kamishibai::gemini::EmbeddedPrompts)),
        "studio-v1",
        directory.path().join("cache"),
        directory.path().join("output"),
        None,
    )
    .expect("a stranger's parts must compose");
    let target = LearningTarget::Explicit(
        kamishibai::languages::catalog()
            .resolve("VI")
            .expect("Vietnamese must resolve"),
    );
    let understood = workflow
        .understand(&RawInputBatch::new("mèo"), "EN", &target)
        .expect("understanding must decode");
    let drafts = kamishibai::application::drafts_for(
        understood.candidates(),
        &kamishibai::session::LanguagePair::new("VI", "EN"),
    );
    let mut run = GenerationRun::new(drafts).expect("the run must start");
    while run
        .advance(&workflow)
        .expect("each artifact must settle")
        .is_some()
    {}
    run.into_drafts()
}

fn soft_files(directory: &TempDir) -> usize {
    fn count(path: &std::path::Path) -> usize {
        std::fs::read_dir(path).map_or(0, |entries| {
            entries
                .filter_map(std::result::Result::ok)
                .map(|entry| {
                    let path = entry.path();
                    if path.is_dir() {
                        count(&path)
                    } else {
                        usize::from(
                            path.file_name().and_then(|name| name.to_str())
                                == Some(kamishibai::generation::artifact_cache::SOFT_FILE),
                        )
                    }
                })
                .sum()
        })
    }
    count(directory.path())
}

#[test]
fn a_plugged_style_cannot_be_replaced_by_the_built_in_one() {
    let directory = TempDir::new().expect("temporary job root must exist");
    let studio = Arc::new(Studio::refusing(0));
    let drafts = produced(
        studio.clone(),
        Look::new(Arc::new(Watercolor), None, Resend::Automatic),
        &directory,
    );
    assert_eq!(
        (
            drafts.iter().all(|draft| draft.artifacts().all_ready()),
            studio.prompts(GenerationStage::Picture),
        ),
        (
            true,
            vec![String::from("WATERCOLOR wash of 1 framed scenes")]
        ),
        "the drawing model was asked in a style the application did not plug"
    );
}

#[test]
fn a_refused_picture_cannot_skip_the_plugged_gentle_world() {
    let directory = TempDir::new().expect("temporary job root must exist");
    let studio = Arc::new(Studio::refusing(1));
    let cranes = Arc::new(Cranes::new());
    let drafts = produced(
        studio.clone(),
        Look::new(
            Arc::new(Watercolor),
            Some(cranes.clone()),
            Resend::Automatic,
        ),
        &directory,
    );
    let retold = |stage| {
        studio
            .prompts(stage)
            .iter()
            .filter(|prompt| prompt.ends_with("\n\nSOFT WORLD\nOnly folded paper cranes act here."))
            .count()
    };
    let admitted = cranes
        .admitted
        .lock()
        .expect("admission log must remain available")
        .clone();
    assert_eq!(
        (
            drafts.iter().all(|draft| draft.artifacts().all_ready()),
            studio.prompts(GenerationStage::Picture).len(),
            retold(GenerationStage::Features),
            retold(GenerationStage::Scene),
            admitted
                .iter()
                .all(|prompt| prompt.starts_with("WATERCOLOR")),
            admitted.is_empty(),
        ),
        (true, 2, 1, 1, true, false),
        "a refused picture was not retold once in the plugged world and checked by it"
    );
}

#[test]
fn an_empty_world_socket_cannot_invent_a_gentle_retelling() {
    let directory = TempDir::new().expect("temporary job root must exist");
    let studio = Arc::new(Studio::refusing(1));
    let drafts = produced(
        studio.clone(),
        Look::new(Arc::new(Watercolor), None, Resend::Automatic),
        &directory,
    );
    let worded = [GenerationStage::Features, GenerationStage::Scene]
        .into_iter()
        .flat_map(|stage| studio.prompts(stage))
        .filter(|prompt| prompt.contains("SOFT WORLD"))
        .count();
    assert_eq!(
        (
            drafts.iter().all(|draft| draft.artifacts().all_ready()),
            studio.prompts(GenerationStage::Picture).len(),
            worded,
            soft_files(&directory),
        ),
        (true, 2, 0, 0),
        "a refusal without a plugged world asked for a retelling or left gentle state behind"
    );
}
