//! Adapts Gemini to scene, image, speech, literal-text, and recall ports.

use std::path::PathBuf;

use anyhow::{Result, bail};
use image::GrayImage;

use super::cost_accounting::CostRecorder;
use crate::gemini::{GeminiClient, HttpTransport, Transport};
use crate::generation::manga::{
    FidelityCheck, ImageSource, LiteralZoomCheck, RecallCard, RecallJudge, RecallReview, TextCheck,
    TextDetector, TextJudge, TextReview, TextReviewGate, literal_zoom_crops,
};
use crate::generation::{SceneSource, Speaker};
use crate::languages::{LanguageProfile, TextGate};

#[derive(Clone)]
/// Adapts metered Gemini calls to scene, image, and speech ports.
pub(super) struct MeteredGemini {
    client: GeminiClient<HttpTransport>,
    costs: CostRecorder,
}

impl MeteredGemini {
    /// Bind a Gemini client to one artifact cost recorder.
    pub(super) fn new(client: GeminiClient<HttpTransport>, costs: CostRecorder) -> Self {
        Self { client, costs }
    }
}

impl SceneSource for MeteredGemini {
    fn scene(
        &self,
        language: &str,
        term: &str,
        sentence: &str,
        target: &str,
        attempt: u8,
    ) -> Result<serde_json::Value> {
        self.client
            .scene_observed(language, term, sentence, target, attempt, |cost| {
                self.costs.push(cost)
            })
    }
}

impl ImageSource for MeteredGemini {
    fn image(&self, prompt: &str) -> Result<Vec<u8>> {
        self.client
            .image_observed(prompt, |cost| self.costs.push(cost))
    }
}

impl Speaker for MeteredGemini {
    fn speech(&self, prompt: &str, text: &str) -> Result<Vec<u8>> {
        self.client
            .speech_observed(prompt, text, |cost| self.costs.push(cost))
    }
}

#[derive(Clone)]
/// Uses Gemini to judge whether an image preserves the recall contract.
pub(super) struct GeminiRecall<T = HttpTransport> {
    client: GeminiClient<T>,
    card: RecallCard,
    costs: CostRecorder,
}

impl<T> GeminiRecall<T> {
    /// Bind Gemini, the recall contract, and its picture cost recorder.
    pub(super) fn new(client: GeminiClient<T>, card: RecallCard, costs: CostRecorder) -> Self {
        Self {
            client,
            card,
            costs,
        }
    }
}

impl<T> RecallJudge for GeminiRecall<T>
where
    T: Transport,
{
    fn review(&self, scene: &serde_json::Value, image: &[u8]) -> Result<RecallReview> {
        let review = self.client.review_recall_observed(
            &self.card,
            scene,
            image_mime(image)?,
            image,
            |cost| self.costs.push(cost),
        )?;
        let review = if review.needs_fidelity() {
            let fidelity = self.client.review_fidelity_observed(
                &FidelityCheck::new(scene)?,
                image_mime(image)?,
                image,
                |cost| self.costs.push(cost),
            )?;
            review.merged_fidelity(fidelity)?
        } else {
            review
        };
        if !review.needs_zoom() {
            return Ok(review);
        }
        let crops = literal_zoom_crops(image)?;
        let zoom = self.client.review_literal_zoom_observed(
            &LiteralZoomCheck::new(),
            crops.as_slice(),
            |cost| self.costs.push(cost),
        )?;
        review.merged_zoom(zoom)
    }
}

#[derive(Clone)]
/// Uses Gemini vision to judge literal writing without OCR.
pub(super) struct GeminiText {
    client: GeminiClient<HttpTransport>,
    check: TextCheck,
    costs: CostRecorder,
}

impl GeminiText {
    /// Bind Gemini, one target language, and its picture cost recorder.
    pub(super) fn new(
        client: GeminiClient<HttpTransport>,
        check: TextCheck,
        costs: CostRecorder,
    ) -> Self {
        Self {
            client,
            check,
            costs,
        }
    }
}

impl TextJudge for GeminiText {
    fn gate(&self) -> TextReviewGate {
        TextReviewGate::LlmJudge
    }

    fn review(&self, image: &[u8], _grayscale: &GrayImage) -> Result<TextReview> {
        self.client
            .review_text_observed(&self.check, image_mime(image)?, image, |cost| {
                self.costs.push(cost)
            })
    }
}

/// Production literal-writing gate selected from one language profile.
pub(super) enum GeminiTextGate {
    /// Detect literal writing with the profile's PP-OCRv5 bundle.
    Ocr(TextDetector),
    /// Detect literal writing directly with Gemini vision.
    LlmJudge(GeminiText),
}

impl GeminiTextGate {
    /// Compose the language-declared text gate without provider or model initialization.
    pub(super) fn new(
        language: &LanguageProfile,
        cache: PathBuf,
        client: GeminiClient<HttpTransport>,
        costs: CostRecorder,
    ) -> Self {
        match language.text_gate {
            TextGate::Ocr(model) => Self::Ocr(TextDetector::cached(60, model, cache)),
            TextGate::LlmJudge => Self::LlmJudge(GeminiText::new(
                client,
                TextCheck::new(language.prompt.clone()),
                costs,
            )),
        }
    }

    #[cfg(test)]
    /// Return the configured OCR bundle, or absence for the direct LLM route.
    pub(super) fn ocr_model(&self) -> Option<crate::languages::OcrModel> {
        match self {
            Self::Ocr(detector) => Some(detector.model()),
            Self::LlmJudge(_) => None,
        }
    }
}

impl TextJudge for GeminiTextGate {
    fn gate(&self) -> TextReviewGate {
        match self {
            Self::Ocr(judge) => judge.gate(),
            Self::LlmJudge(judge) => judge.gate(),
        }
    }

    fn review(&self, image: &[u8], grayscale: &GrayImage) -> Result<TextReview> {
        match self {
            Self::Ocr(judge) => judge.review(image, grayscale),
            Self::LlmJudge(judge) => judge.review(image, grayscale),
        }
    }
}

fn image_mime(image: &[u8]) -> Result<&'static str> {
    match image::guess_format(image)? {
        image::ImageFormat::Jpeg => Ok("image/jpeg"),
        image::ImageFormat::Png => Ok("image/png"),
        image::ImageFormat::WebP => Ok("image/webp"),
        image::ImageFormat::Gif => Ok("image/gif"),
        format => bail!("unsupported vision-review image format {format:?}"),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::io::Cursor;
    use std::rc::Rc;

    use image::{DynamicImage, GrayImage, ImageFormat, Luma};
    use serde_json::{Value, json};
    use tempfile::tempdir;

    use super::*;
    use crate::gemini::{Transport, TransportResponse};
    use crate::generation::artifact_cache::Cache;
    use crate::generation::card_production::cost_accounting::load_cost_record;
    use crate::generation::manga::{HiddenRecall, ShownRecall};
    use crate::languages::{OcrModel, catalog};
    use crate::session::Artifact;

    #[derive(Clone, Debug)]
    struct ScriptedTransport {
        requests: Rc<RefCell<Vec<Value>>>,
        responses: Rc<RefCell<VecDeque<TransportResponse>>>,
    }

    impl ScriptedTransport {
        fn new(responses: Vec<Value>) -> Self {
            Self {
                requests: Rc::new(RefCell::new(Vec::new())),
                responses: Rc::new(RefCell::new(
                    responses.into_iter().map(response).collect::<VecDeque<_>>(),
                )),
            }
        }
    }

    impl Transport for ScriptedTransport {
        fn post(&self, _url: &str, _key: &str, body: &str) -> Result<TransportResponse> {
            self.requests.borrow_mut().push(serde_json::from_str(body)?);
            self.responses
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("scripted recall response queue is empty"))
        }
    }

    fn response(value: Value) -> TransportResponse {
        TransportResponse {
            status: 200,
            body: json!({
                "candidates": [{"content": {"parts": [{"text": value.to_string()}]}}],
                "usageMetadata": {
                    "promptTokenCount": 100,
                    "candidatesTokenCount": 20,
                    "totalTokenCount": 120
                }
            })
            .to_string(),
        }
    }

    fn recall_card() -> RecallCard {
        RecallCard::new(
            ShownRecall::new("Hindi", "यह केवल दिखाया गया वाक्य है।", "दिखाया", "एक दृश्य संकेत"),
            HiddenRecall::new(
                "English",
                "misunderstanding",
                "They had a misunderstanding.",
            ),
        )
    }

    fn recall_scene() -> Value {
        json!({
            "manga_panel": {
                "semantic_spine": {
                    "literal_event": "two people misunderstand one another",
                    "semantic_focus": "a failed exchange",
                    "visual_relation": "opposition",
                    "metaphor": {"literal_anchor": "two confused people"}
                },
                "panels": [{
                    "id": "p1",
                    "semantic_job": "show both people reacting to the failed exchange",
                    "shot_contract": {"visible_anchor": "two confused people facing one another"},
                    "scene": {"subjects": [{
                        "id": "first_person",
                        "figure": "one person",
                        "pose": "facing the other person",
                        "expression": "confused"
                    }, {
                        "id": "second_person",
                        "figure": "another person",
                        "pose": "facing the first person",
                        "expression": "confused"
                    }]}
                }]
            }
        })
    }

    fn source_png() -> Vec<u8> {
        let mut image = GrayImage::from_pixel(1024, 1024, Luma([255]));
        for y in [674, 679, 684, 689] {
            for x in 430..462 {
                image.put_pixel(x, y, Luma([0]));
            }
        }
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageLuma8(image)
            .write_to(&mut encoded, ImageFormat::Png)
            .expect("synthetic recall image must encode");
        encoded.into_inner()
    }

    #[test]
    fn language_profiles_compose_direct_llm_or_exact_ocr_text_gates() {
        let directory = tempdir().expect("text-gate composition cache must be created");
        let cache = Cache::new("text-gate", directory.path());
        let costs = CostRecorder::new(cache, Artifact::Picture);
        let client = GeminiClient::new("test-key", HttpTransport::new());
        let routes = ["he", "vi", "ko", "ar", "hi", "th", "uk"].map(|code| {
            let language = catalog()
                .item(code)
                .expect("supported language must have one profile");
            let gate = GeminiTextGate::new(
                &language,
                directory.path().to_path_buf(),
                client.clone(),
                costs.clone(),
            );
            (gate.gate(), gate.ocr_model())
        });
        assert_eq!(
            routes,
            [
                (TextReviewGate::LlmJudge, None),
                (TextReviewGate::LlmJudge, None),
                (TextReviewGate::Ocr, Some(OcrModel::Korean)),
                (TextReviewGate::Ocr, Some(OcrModel::Arabic)),
                (TextReviewGate::Ocr, Some(OcrModel::Devanagari)),
                (TextReviewGate::Ocr, Some(OcrModel::Th)),
                (TextReviewGate::Ocr, Some(OcrModel::Cyrillic)),
            ],
            "language profiles constructed an OCR detector for HE/VI or selected a wrong OCR bundle"
        );
    }

    #[test]
    fn scale_aware_recall_merges_one_zoom_request_after_a_clean_full_review() {
        let transport = ScriptedTransport::new(vec![
            json!({
                "decision": "ALLOW",
                "evidence": [],
                "literal_writing_present": false,
                "literal_evidence": [],
                "reason": "No answer-bearing writing is visible"
            }),
            json!({
                "scene_fidelity_decision": "ALLOW",
                "scene_fidelity_evidence": [],
                "reason": "Every required subject and relation is visible"
            }),
            json!({
                "literal_writing_present": false,
                "literal_evidence": [{
                    "description": "four organized pseudo-writing rows on a distant board",
                    "location": "crop 8 lower-center; original lower-center",
                    "kind": "PSEUDO_WRITING"
                }],
                "reason": "The enlarged board contains writing-like rows"
            }),
        ]);
        let requests = transport.requests.clone();
        let directory = tempdir().expect("scale-aware recall cache must be created");
        let cache = Cache::new("scale-aware-recall", directory.path());
        let costs = CostRecorder::new(cache.clone(), Artifact::Picture);
        let recall = GeminiRecall::new(GeminiClient::new("key", transport), recall_card(), costs);
        let review = recall
            .review(&recall_scene(), source_png().as_slice())
            .expect("scale-aware recall must return a merged verdict");
        let archived = serde_json::to_value(&review).expect("merged recall must serialize");
        let cost = load_cost_record(&cache, Artifact::Picture)
            .expect("scale-aware recall cost must load")
            .expect("scale-aware recall cost must exist");
        let sent = requests.borrow();
        assert_eq!(
            (
                sent.len(),
                sent[1]["contents"][0]["parts"].as_array().map(Vec::len),
                sent[2]["contents"][0]["parts"].as_array().map(Vec::len),
                review.allows(),
                review.literal_rejection().is_some(),
                archived["fidelity_inspected"].as_bool(),
                archived["zoom_inspected"].as_bool(),
                cost.model(),
                cost.requests(),
            ),
            (
                3,
                Some(2),
                Some(10),
                true,
                true,
                Some(true),
                Some(true),
                "gemini-3.5-flash-lite,gemini-3.6-flash,gemini-3.6-flash",
                3,
            ),
            "clean full recall did not run dedicated fidelity before exactly one zoom scan"
        );
    }

    #[test]
    fn dedicated_fidelity_rejection_skips_zoom_and_preserves_semantic_allow() {
        let transport = ScriptedTransport::new(vec![
            json!({
                "decision": "ALLOW",
                "evidence": [],
                "literal_writing_present": false,
                "literal_evidence": [],
                "reason": "No answer-bearing writing is visible"
            }),
            json!({
                "scene_fidelity_decision": "ALLOW",
                "scene_fidelity_evidence": [{
                    "requirement": "touchy_man must remain the same person in p1 and p2",
                    "observed": "p1 shows an older heavy square-faced man while p2 shows a younger slim soft-faced man",
                    "location": "touchy_man in left p1 and listener in right p2",
                    "kind": "BROKEN_SUBJECT_CONTINUITY"
                }],
                "reason": "The repeated subject is substituted"
            }),
        ]);
        let requests = transport.requests.clone();
        let directory = tempdir().expect("dedicated fidelity cache must be created");
        let cache = Cache::new("dedicated-fidelity-recall", directory.path());
        let costs = CostRecorder::new(cache.clone(), Artifact::Picture);
        let recall = GeminiRecall::new(GeminiClient::new("key", transport), recall_card(), costs);
        let review = recall
            .review(&recall_scene(), source_png().as_slice())
            .expect("dedicated fidelity rejection must return a merged verdict");
        let archived = serde_json::to_value(&review).expect("fidelity review must serialize");
        let cost = load_cost_record(&cache, Artifact::Picture)
            .expect("dedicated fidelity cost must load")
            .expect("dedicated fidelity cost must exist");
        assert_eq!(
            (
                requests.borrow().len(),
                review.allows(),
                review.scene_fidelity_rejection().is_some(),
                review.literal_rejection(),
                archived["scene_fidelity_decision"].as_str(),
                archived["scene_fidelity_evidence"][0]["kind"].as_str(),
                archived["fidelity_inspected"].as_bool(),
                archived["zoom_inspected"].as_bool(),
                cost.model(),
                cost.requests(),
            ),
            (
                2,
                true,
                true,
                None,
                Some("REJECT"),
                Some("BROKEN_SUBJECT_CONTINUITY"),
                Some(true),
                Some(false),
                "gemini-3.5-flash-lite,gemini-3.6-flash",
                2,
            ),
            "dedicated fidelity rejection changed semantics, ran zoom, or lost typed cost and archive proof"
        );
    }

    #[test]
    fn scale_aware_recall_skips_zoom_after_any_full_review_rejection() {
        let full_reviews = [
            json!({
                "decision": "REJECT",
                "evidence": [{
                    "reading": "misunderstanding",
                    "location": "upper sign",
                    "kind": "FOCUS"
                }],
                "literal_writing_present": true,
                "literal_evidence": [{
                    "description": "the word misunderstanding",
                    "location": "upper sign",
                    "kind": "WRITING"
                }],
                "reason": "The hidden answer is visible"
            }),
            json!({
                "decision": "ALLOW",
                "evidence": [],
                "literal_writing_present": false,
                "literal_evidence": [{
                    "description": "organized pseudo-writing rows",
                    "location": "lower board",
                    "kind": "PSEUDO_WRITING"
                }],
                "reason": "No answer-bearing writing is visible"
            }),
            json!({
                "decision": "ALLOW",
                "evidence": [],
                "scene_fidelity_decision": "ALLOW",
                "scene_fidelity_evidence": [{
                    "requirement": "panel p1 requires both confused people",
                    "observed": "only one person is visible",
                    "location": "full image",
                    "kind": "MISSING_REQUIRED_SUBJECT"
                }],
                "literal_writing_present": false,
                "literal_evidence": [],
                "reason": "No answer-bearing writing is visible"
            }),
        ];
        let outcomes = full_reviews.map(|full| {
            let transport = ScriptedTransport::new(vec![full]);
            let requests = transport.requests.clone();
            let directory = tempdir().expect("skip-path recall cache must be created");
            let costs = CostRecorder::new(
                Cache::new("skip-scale-aware-recall", directory.path()),
                Artifact::Picture,
            );
            let recall =
                GeminiRecall::new(GeminiClient::new("key", transport), recall_card(), costs);
            let review = recall
                .review(&recall_scene(), source_png().as_slice())
                .expect("full rejection must return without a zoom request");
            (
                requests.borrow().len(),
                serde_json::to_value(review)
                    .expect("full rejection must serialize")["zoom_inspected"]
                    .as_bool(),
            )
        });
        assert_eq!(
            outcomes,
            [(1, Some(false)), (1, Some(false)), (1, Some(false))],
            "scale-aware recall ran after the full review had already rejected the image"
        );
    }
}
