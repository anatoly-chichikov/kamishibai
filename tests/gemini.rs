//! Tests for the direct Gemini REST client.

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;
use kamishibai::gemini::{GeminiClient, Transport, TransportResponse};
use kamishibai::session::{CandidateKind, CardDraft, CardPayload, LanguagePair, RawInputBatch};
use serde_json::{Value, json};

/// Fake transport that records requests and replays fixed responses.
#[derive(Clone, Debug)]
struct FakeTransport {
    requests: Rc<RefCell<Vec<(String, String)>>>,
    responses: Rc<RefCell<Vec<Result<TransportResponse>>>>,
}

impl FakeTransport {
    /// Create one fake transport.
    fn new(responses: Vec<Result<TransportResponse>>) -> Self {
        Self {
            requests: Rc::new(RefCell::new(Vec::new())),
            responses: Rc::new(RefCell::new(responses)),
        }
    }
}

impl Transport for FakeTransport {
    /// Record one request and return the next queued response.
    fn post(&self, url: &str, _key: &str, body: &str) -> Result<TransportResponse> {
        self.requests
            .borrow_mut()
            .push((String::from(url), String::from(body)));
        self.responses.borrow_mut().remove(0)
    }
}

/// Return a successful JSON response body.
fn body(value: Value) -> Result<TransportResponse> {
    Ok(TransportResponse {
        status: 200,
        body: serde_json::to_string(&value)?,
    })
}

/// Return the first text prompt recorded by fake transport.
fn recorded_prompt(requests: &Rc<RefCell<Vec<(String, String)>>>) -> Result<String> {
    let body = requests.borrow()[0].1.clone();
    let value = serde_json::from_str::<Value>(body.as_str())?;
    Ok(String::from(
        value["contents"][0]["parts"][0]["text"]
            .as_str()
            .expect("request text must exist"),
    ))
}

/// Return the current language labels exactly as the registry exposes them.
fn registry_language_labels() -> Result<String> {
    let catalog = kamishibai::languages::catalog();
    let mut labels = Vec::new();
    for code in catalog.codes() {
        let item = catalog.item(code)?;
        labels.push(format!("{} ({})", item.code, item.prompt));
    }
    Ok(labels.join(", "))
}

/// Understanding uses Flash and returns form comments plus skipped off-language rows.
#[test]
fn understanding_uses_flash_and_returns_form_comments() -> Result<()> {
    let transport = FakeTransport::new(vec![Ok(body(json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "text": "{\"target_lang\":\"en\",\"items\":[{\"term\":\"wrecked\",\"language\":\"en\",\"kind\":\"word\",\"preview\":\"разрушил / потерпел крушение\",\"note\":\"verb; past tense or past participle of wreck; generate this surface form\",\"include\":true},{\"term\":\"окно\",\"language\":\"ru\",\"kind\":\"skip\",\"preview\":\"not generated\",\"note\":\"outside the EN batch\",\"include\":false}]}"
                }]
            }
        }]
    }))?)]);
    let requests = transport.requests.clone();
    let client = GeminiClient::new("key", transport);
    let understood = client.understand(&RawInputBatch::new("wrecked\nокно"), "ru")?;
    let prompt = recorded_prompt(&requests)?;
    assert_eq!(
        (
            requests.borrow()[0].0.as_str(),
            prompt.contains(registry_language_labels()?.as_str()),
            prompt.contains("Use ISO 639-1 codes: en, ru, es, de, el, zh"),
            understood.guess().code(),
            understood.candidates()[0].term(),
            understood.candidates()[0].kind().label(),
            understood.candidates()[0].note(),
            understood.candidates()[1].kind().label(),
            understood.candidates()[1].included(),
        ),
        (
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3-flash-preview:generateContent",
            true,
            false,
            "en",
            "wrecked",
            "word",
            "en · verb; past tense or past participle of wreck; generate this surface form",
            "skip",
            false,
        ),
        "understanding must use Flash, surface form comments, and skip off-language rows"
    );
    Ok(())
}

/// Understanding rejects grammar labels instead of accepting category drift.
#[test]
fn understanding_rejects_grammar_candidate_kind() {
    let transport = FakeTransport::new(vec![Ok(body(json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "text": "{\"target_lang\":\"en\",\"items\":[{\"term\":\"wrecked\",\"language\":\"en\",\"kind\":\"verb\",\"preview\":\"разрушил\",\"note\":\"past tense of wreck\",\"include\":true}]}"
                }]
            }
        }]
    }))
    .expect("response body must serialize"))]);
    let client = GeminiClient::new("key", transport);
    assert!(
        client
            .understand(&RawInputBatch::new("wrecked"), "ru")
            .is_err(),
        "understanding must not accept grammar labels as candidate kinds"
    );
}

/// Bulk correction uses Flash and can change the candidate form comment.
#[test]
fn bulk_correction_uses_flash_and_updates_form_comments() -> Result<()> {
    let transport = FakeTransport::new(vec![Ok(body(json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "text": "{\"target_lang\":\"en\",\"items\":[{\"term\":\"wound\",\"language\":\"en\",\"kind\":\"word\",\"preview\":\"рана\",\"note\":\"noun, not past tense of wind\",\"include\":true}]}"
                }]
            }
        }]
    }))?)]);
    let client = GeminiClient::new("key", transport);
    let updated = client.correct_bulk(
        &[kamishibai::session::WordCandidate::new(
            "wound",
            CandidateKind::Word,
            "намотал",
            "ambiguous form",
        )],
        "treat it as a noun",
        &LanguagePair::new("en", "ru"),
    )?;
    assert_eq!(
        (
            updated[0].kind().label(),
            updated[0].preview(),
            updated[0].note()
        ),
        ("word", "рана", "en · noun, not past tense of wind"),
        "bulk correction must use Flash output for the refined form comment"
    );
    Ok(())
}

/// Per-card correction uses Flash to recompose the draft fields.
#[test]
fn card_correction_uses_flash_to_recompose_card_fields() -> Result<()> {
    let transport = FakeTransport::new(vec![Ok(body(json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "text": "{\"front\":\"I wound the clock.\",\"back\":\"Я завел часы.\",\"hint\":\"wound = past of wind\",\"highlight\":\"wound\"}"
                }]
            }
        }]
    }))?)]);
    let client = GeminiClient::new("key", transport);
    let draft = CardDraft::new(
        "wound",
        LanguagePair::new("en", "ru"),
        CardPayload::new("front", "back", "hint", "wound"),
    );
    let updated = client.correct_card(&draft, "make it verb", &LanguagePair::new("en", "ru"))?;
    assert_eq!(
        (
            updated.payload().front(),
            updated.payload().back(),
            updated.payload().hint(),
            updated.payload().highlight(),
        ),
        (
            "I wound the clock.",
            "Я завел часы.",
            "wound = past of wind",
            "wound",
        ),
        "card correction must recompose draft fields from Flash JSON"
    );
    Ok(())
}

/// Missing API keys keep the frozen startup error wording.
#[test]
fn missing_api_keys_keep_the_frozen_startup_error_wording() {
    unsafe {
        std::env::remove_var("GEMINI_API_KEY");
    }
    assert_eq!(
        GeminiClient::from_env().unwrap_err().to_string(),
        "GEMINI_API_KEY environment variable is not set; export it before running",
        "missing api keys no longer keep the frozen startup error wording"
    );
}

/// Scene generation keeps the merged scene contract.
#[test]
fn scene_generation_keeps_the_merged_scene_contract() -> Result<()> {
    let transport = FakeTransport::new(vec![Ok(body(
        json!({"candidates":[{"content":{"parts":[{"text":"```json\n[{\"bounds\":{\"x\":0,\"y\":1,\"width\":2000,\"height\":2000},\"scene\":{\"description\":\"A cat\"},\"narrative_weight\":\"primary\",\"bleed\":true}]\n```"}]}}]}),
    )?)]);
    let client = GeminiClient::new("key", transport);
    let scene = client.scene("English", "The cat is sleeping on the windowsill", "en")?;
    assert_eq!(
        (
            scene["manga_panel"]["meta"]["title"].as_str(),
            scene["manga_panel"]["meta"]["target_lang"].as_str(),
            scene["manga_panel"]["panels"][0]["bounds"]["x"].as_i64(),
            scene["manga_panel"]["panels"][0]["bounds"]["width"].as_i64(),
            scene["manga_panel"]["panels"][0]["scene"]["text_in_frame"].as_str()
        ),
        (
            Some("The cat is sleeping on the windowsill"),
            Some("en"),
            Some(16),
            Some(992),
            Some("none")
        ),
        "scene generation no longer keeps the merged scene contract"
    );
    Ok(())
}

/// Scene generation rejects non-array responses.
#[test]
fn scene_generation_rejects_non_array_responses() {
    let transport = FakeTransport::new(vec![Ok(body(
        json!({"candidates":[{"content":{"parts":[{"text":"{\"panels\":[]}"}]}}]}),
    )
    .expect("response body must serialize"))]);
    let client = GeminiClient::new("key", transport);
    assert_eq!(
        client
            .scene("English", "demo", "en")
            .unwrap_err()
            .to_string(),
        "Expected a JSON array of panels",
        "scene generation no longer rejects non-array responses with the frozen error wording"
    );
}

/// Scene generation rejects empty panel arrays.
#[test]
fn scene_generation_rejects_empty_panel_arrays() {
    let transport = FakeTransport::new(vec![Ok(body(
        json!({"candidates":[{"content":{"parts":[{"text":"[]"}]}}]}),
    )
    .expect("response body must serialize"))]);
    let client = GeminiClient::new("key", transport);
    assert_eq!(
        client
            .scene("English", "demo", "en")
            .unwrap_err()
            .to_string(),
        "No panels found in scene JSON",
        "scene generation no longer rejects empty panel arrays with the frozen error wording"
    );
}

/// Image generation keeps the IMAGE modality and square aspect ratio.
#[test]
fn image_generation_keeps_the_image_modality_and_square_aspect_ratio() -> Result<()> {
    let transport = FakeTransport::new(vec![Ok(body(
        json!({"candidates":[{"content":{"parts":[{"inlineData":{"data":"AQID"}}]}}]}),
    )?)]);
    let requests = transport.requests.clone();
    let client = GeminiClient::new("key", transport);
    let _bytes =
        client.image(&json!({"manga_panel":{"panels":[{"scene":{"description":"A cat"}}]}}))?;
    let request = serde_json::from_str::<Value>(&requests.borrow()[0].1)?;
    assert_eq!(
        (
            request["generationConfig"]["responseModalities"][0].as_str(),
            request["generationConfig"]["imageConfig"]["aspectRatio"].as_str(),
            request["safetySettings"]
                .as_array()
                .map(|items| items.len())
        ),
        (Some("IMAGE"), Some("1:1"), Some(4)),
        "image generation request no longer keeps the frozen modality and aspect-ratio contract"
    );
    Ok(())
}

/// Image generation surfaces blocked-response diagnostics.
#[test]
fn image_generation_surfaces_blocked_response_diagnostics() {
    let transport = FakeTransport::new(vec![Ok(body(json!({"candidates":[],"promptFeedback":{"blockReason":"SAFETY","blockReasonMessage":"blocked","safetyRatings":[{"category":"HARM_CATEGORY_HARASSMENT","probability":"MEDIUM","blocked":true}]}})).expect("response body must serialize"))]);
    let client = GeminiClient::new("key", transport);
    assert_eq!(
        client
            .image(&json!({"manga_panel":{"panels":[{"scene":{"description":"A cat"}}]}}))
            .unwrap_err()
            .to_string(),
        "No candidates in image response: SAFETY, blocked, flagged=[HARM_CATEGORY_HARASSMENT=MEDIUM]",
        "image generation no longer surfaces the frozen blocked-response diagnostics"
    );
}

/// TTS generation targets the 3.1 flash preview with a pooled voice.
#[test]
fn tts_generation_targets_the_3_1_flash_preview_with_a_pooled_voice() -> Result<()> {
    let transport = FakeTransport::new(vec![Ok(body(
        json!({"candidates":[{"content":{"parts":[{"inlineData":{"data":"AQID"}}]}}]}),
    )?)]);
    let requests = transport.requests.clone();
    let client = GeminiClient::new("key", transport);
    let bytes = client.speech("Say in natural English: {text}", "demo")?;
    let items = requests.borrow();
    let body = serde_json::from_str::<Value>(&items[0].1)?;
    let voice =
        body["generationConfig"]["speechConfig"]["voiceConfig"]["prebuiltVoiceConfig"]["voiceName"]
            .as_str()
            .unwrap_or_default();
    assert_eq!(
        (
            bytes,
            items.len(),
            items[0].0.as_str(),
            GeminiClient::new("key", FakeTransport::new(Vec::new()))
                .voices()
                .contains(&voice)
        ),
        (
            vec![1, 2, 3],
            1,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.1-flash-tts-preview:generateContent",
            true
        ),
        "tts generation no longer hits the 3.1 flash preview exactly once with a pooled voice"
    );
    Ok(())
}

/// TTS generation does not hide non-quota failures.
#[test]
fn tts_generation_does_not_hide_non_quota_failures() {
    let transport = FakeTransport::new(vec![Ok(TransportResponse {
        status: 500,
        body: String::from("{\"error\":{\"status\":\"INTERNAL\",\"message\":\"boom\"}}"),
    })]);
    let client = GeminiClient::new("key", transport);
    assert_eq!(
        client
            .speech("Say in natural English: {text}", "demo")
            .unwrap_err()
            .to_string(),
        "INTERNAL: boom",
        "tts generation no longer surfaces non-quota failures immediately"
    );
}
