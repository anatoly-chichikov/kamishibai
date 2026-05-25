//! Tests for the direct Gemini REST client.

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;
use kamishibai::gemini::{GeminiClient, Transport, TransportResponse, rejects_key};
use kamishibai::session::{CardDraft, CardMeta, LanguagePair, RawInputBatch, WordCandidate};
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

/// Understanding uses Flash and returns the simple {term, understanding, ok} shape.
#[test]
fn understanding_uses_flash_and_returns_simple_understanding_rows() -> Result<()> {
    let transport = FakeTransport::new(vec![Ok(body(json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "text": "{\"target_lang\":\"en\",\"items\":[{\"term\":\"wrecked\",\"understanding\":\"past tense of \\\"wreck\\\" — destroyed or crashed\",\"ok\":true},{\"term\":\"окно\",\"understanding\":\"this is Russian, not the target language; will not be turned into a card\",\"ok\":false}]}"
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
            prompt.contains("Supported target languages"),
            understood.guess().code(),
            understood.candidates()[0].term(),
            understood.candidates()[0].understanding(),
            understood.candidates()[0].ok(),
            understood.candidates()[1].term(),
            understood.candidates()[1].ok(),
        ),
        (
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.5-flash:generateContent",
            true,
            "en",
            "wrecked",
            "past tense of \"wreck\" — destroyed or crashed",
            true,
            "окно",
            false,
        ),
        "understanding must use Flash, return simple human-language understanding rows, and mark off-language rows ok=false"
    );
    Ok(())
}

/// Bulk correction uses Flash and updates the understanding sentence.
#[test]
fn bulk_correction_uses_flash_and_updates_understanding() -> Result<()> {
    let transport = FakeTransport::new(vec![Ok(body(json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "text": "{\"target_lang\":\"en\",\"items\":[{\"term\":\"wound\",\"understanding\":\"noun: a wound on the body, not the past tense of wind\",\"ok\":true}]}"
                }]
            }
        }]
    }))?)]);
    let client = GeminiClient::new("key", transport);
    let updated = client.correct_bulk(
        &[WordCandidate::new(
            "wound",
            "ambiguous between noun and past-tense verb",
            true,
        )],
        "treat it as a noun",
        &LanguagePair::new("en", "ru"),
    )?;
    assert_eq!(
        (
            updated[0].term(),
            updated[0].understanding(),
            updated[0].ok()
        ),
        (
            "wound",
            "noun: a wound on the body, not the past tense of wind",
            true,
        ),
        "bulk correction must use Flash output to refine the understanding sentence"
    );
    Ok(())
}

/// Card-meta generation uses Flash and returns the full rich meta.
#[test]
fn card_meta_generation_uses_flash_and_returns_full_meta() -> Result<()> {
    let transport = FakeTransport::new(vec![Ok(body(json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "text": "{\"pronunciation\":\"ˈbɒrəʊ\",\"transcription\":\"kən aɪ ˈbɒrəʊ jɔː ˈpɛn\",\"meaning\":\"одолжить\",\"importance\":8,\"source_sentence\":\"Можно одолжить твою ручку?\",\"source_highlight\":\"одолжить\",\"source_hint\":\"Когда ручка не твоя, а надо записать — вежливо просишь на время.\",\"source_context\":\"Нейтрально-вежливый глагол.\",\"target_sentence\":\"Can I borrow your pen?\"}"
                }]
            }
        }]
    }))?)]);
    let requests = transport.requests.clone();
    let client = GeminiClient::new("key", transport);
    let meta_out = client.generate_card_meta(
        "borrow",
        "verb sense — to take something temporarily",
        &LanguagePair::new("en", "ru"),
    )?;
    assert_eq!(
        (
            requests.borrow()[0].0.as_str(),
            meta_out.pronunciation(),
            meta_out.target_sentence(),
            meta_out.source_highlight(),
            meta_out.importance(),
        ),
        (
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.5-flash:generateContent",
            "ˈbɒrəʊ",
            "Can I borrow your pen?",
            "одолжить",
            8,
        ),
        "card-meta generation must hit the Flash model and decode every rich field"
    );
    Ok(())
}

/// Per-card correction uses Flash and may revise term, understanding, and full meta.
#[test]
fn card_correction_uses_flash_to_recompose_term_understanding_and_meta() -> Result<()> {
    let transport = FakeTransport::new(vec![Ok(body(json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "text": "{\"term\":\"wound\",\"understanding\":\"verb: to wound someone — past tense of wind in another sense was wrong\",\"pronunciation\":\"waʊnd\",\"transcription\":\"aɪ waʊnd ðə klɒk\",\"meaning\":\"завести\",\"importance\":6,\"source_sentence\":\"Я завел часы.\",\"source_highlight\":\"завел\",\"source_hint\":\"Поворачивал что-то круглое, чтобы оно начало работать.\",\"source_context\":\"Глагол про механические часы.\",\"target_sentence\":\"I wound the clock.\"}"
                }]
            }
        }]
    }))?)]);
    let client = GeminiClient::new("key", transport);
    let meta_seed = CardMeta::new(
        "/wound/",
        "/wound seed/",
        "рана",
        5,
        "src",
        "wound",
        "hint",
        "context",
        "Example.",
    );
    let draft = CardDraft::new("wound", "noun: a wound", LanguagePair::new("en", "ru"))
        .with_meta(meta_seed, None);
    let revision = client.correct_card(
        &draft,
        "treat as past tense of wind",
        &LanguagePair::new("en", "ru"),
    )?;
    let (term, understanding, meta_out) = revision.into_parts();
    assert_eq!(
        (
            term,
            understanding,
            meta_out.target_sentence().to_string(),
            meta_out.source_highlight().to_string(),
            meta_out.importance(),
        ),
        (
            String::from("wound"),
            String::from("verb: to wound someone — past tense of wind in another sense was wrong"),
            String::from("I wound the clock."),
            String::from("завел"),
            6,
        ),
        "card correction must recompose term, understanding, and full meta from Flash JSON"
    );
    Ok(())
}

/// Missing API keys surface the configured startup error wording.
#[test]
fn missing_api_keys_surface_a_setup_hint() {
    let error = GeminiClient::from_saved(None).unwrap_err().to_string();
    assert!(
        error.contains("no Gemini API key"),
        "missing api keys no longer surface the configured startup error wording: {error}"
    );
}

/// Invalid API-key responses are classified without treating all 403s as key failures.
#[test]
fn api_key_errors_are_classified_narrowly() {
    let invalid = FakeTransport::new(vec![Ok(TransportResponse {
        status: 400,
        body: String::from(
            "{\"error\":{\"status\":\"INVALID_ARGUMENT\",\"message\":\"API key not valid. Please pass a valid API key.\",\"details\":[{\"@type\":\"type.googleapis.com/google.rpc.ErrorInfo\",\"reason\":\"API_KEY_INVALID\"}]}}",
        ),
    })]);
    let generic = FakeTransport::new(vec![Ok(TransportResponse {
        status: 403,
        body: String::from(
            "{\"error\":{\"status\":\"PERMISSION_DENIED\",\"message\":\"Access denied for this model\"}}",
        ),
    })]);
    let invalid_error = GeminiClient::new("key", invalid)
        .understand(&RawInputBatch::new("wreck"), "ru")
        .unwrap_err();
    let generic_error = GeminiClient::new("key", generic)
        .understand(&RawInputBatch::new("wreck"), "ru")
        .unwrap_err();
    assert_eq!(
        (rejects_key(&invalid_error), rejects_key(&generic_error)),
        (true, false),
        "Gemini key rejection classification must not collapse every permission failure into a bad key"
    );
}

/// Key validation accepts any 2xx and flags a rejected key without parsing the body.
#[test]
fn validate_key_accepts_2xx_and_flags_rejected_keys() {
    let valid = FakeTransport::new(vec![Ok(TransportResponse {
        status: 200,
        body: String::from("{}"),
    })]);
    let rejected = FakeTransport::new(vec![Ok(TransportResponse {
        status: 400,
        body: String::from(
            "{\"error\":{\"status\":\"INVALID_ARGUMENT\",\"message\":\"API key not valid. Please pass a valid API key.\",\"details\":[{\"@type\":\"type.googleapis.com/google.rpc.ErrorInfo\",\"reason\":\"API_KEY_INVALID\"}]}}",
        ),
    })]);
    let valid_ok = GeminiClient::new("key", valid).validate_key().is_ok();
    let rejected_error = GeminiClient::new("key", rejected)
        .validate_key()
        .unwrap_err();
    assert_eq!(
        (valid_ok, rejects_key(&rejected_error)),
        (true, true),
        "key validation must pass on any 2xx and flag an invalid-key response as a rejected key"
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
    .expect("response meta must serialize"))]);
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
    let transport = FakeTransport::new(vec![Ok(body(json!({"candidates":[],"promptFeedback":{"blockReason":"SAFETY","blockReasonMessage":"blocked","safetyRatings":[{"category":"HARM_CATEGORY_HARASSMENT","probability":"MEDIUM","blocked":true}]}})).expect("response meta must serialize"))]);
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
