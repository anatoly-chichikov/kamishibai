//! Tests for the direct Gemini REST client.

use std::cell::{Cell, RefCell};
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

/// Transport that rejects the composer only when it regresses to a response schema.
#[derive(Clone, Debug)]
struct ComposerSchemaRejectingTransport {
    calls: Rc<Cell<usize>>,
    requests: Rc<RefCell<Vec<Value>>>,
}

impl ComposerSchemaRejectingTransport {
    /// Create one schema-sensitive production-scene transport.
    fn new() -> Self {
        Self {
            calls: Rc::new(Cell::new(0)),
            requests: Rc::new(RefCell::new(Vec::new())),
        }
    }
}

impl Transport for ComposerSchemaRejectingTransport {
    /// Accept typed analysis while rejecting a schema-bearing composer request.
    fn post(&self, _url: &str, _key: &str, body: &str) -> Result<TransportResponse> {
        let request = serde_json::from_str::<Value>(body)?;
        let index = self.calls.get();
        self.calls.set(index + 1);
        self.requests.borrow_mut().push(request.clone());
        if index == 1
            && request
                .pointer("/generationConfig/responseFormat/text/schema")
                .is_some()
        {
            return Ok(api_failure(
                400,
                "INVALID_ARGUMENT",
                "composer response schema is unsupported",
            ));
        }
        let value = match index {
            0 => scene_features(),
            _ => dynamic_scene(),
        };
        scene_body(&value)
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

/// Return one production feature plan for a single motivated camera view.
fn scene_features() -> Value {
    json!({
        "semantic_beat_count": 1,
        "semantic_relation": "single_moment",
        "coverage_audit": [
            {"panel_count": 1, "added_view": "the complete sleeping cat", "source_support": "the cat is sleeping on the windowsill", "verdict": "selected", "reason": "one continuous view carries the event"},
            {"panel_count": 2, "added_view": "a second angle", "source_support": "no second fact supports it", "verdict": "redundant_or_unsupported", "reason": "it repeats the same state"},
            {"panel_count": 3, "added_view": "a reaction", "source_support": "no reaction is stated", "verdict": "redundant_or_unsupported", "reason": "it invents a reaction"},
            {"panel_count": 4, "added_view": "a consequence", "source_support": "no consequence is stated", "verdict": "redundant_or_unsupported", "reason": "it invents a consequence"}
        ],
        "panel_count": 1,
        "panel_relation": "single_moment",
        "panel_emphasis": "equal",
        "decomposition_mode": "single_tableau",
        "motion_vector": "still",
        "intensity": "quiet",
        "spatial_relation": "same_space",
        "transition_type": "none",
        "reading_direction": "left_to_right_top_to_bottom",
        "literal_anchor": "a cat sleeping on the windowsill",
        "camera_arc": {
            "strategy": "single_view",
            "progression": "one held wide objective view",
            "motivation": "the quiet uninterrupted state is strongest without a cut",
            "continuity": {"axis_mode": "not_applicable", "axis": "", "screen_direction": "stationary", "eyeline_policy": "not_applicable"}
        },
        "shots": [{
            "id": "s1",
            "semantic_beat_index": 1,
            "role": "action",
            "visible_anchor": "the complete sleeping cat on the sill",
            "source_support": "the cat is sleeping on the windowsill",
            "shot_scale": "wide",
            "viewpoint": "objective",
            "viewpoint_anchor": "",
            "framing": "single",
            "angle": "eye_level",
            "depth_plan": "deep",
            "camera_motivation": "the wide view proves both sleep and location",
            "information_gain": "the cat and windowsill relation are visible together",
            "transition_trigger": "scene_open"
        }],
        "selection_logic": "one quiet continuous view preserves the literal sentence"
    })
}

/// Return one geometry-free production scene response with no special device.
fn dynamic_scene() -> Value {
    json!({
        "semantic_spine": {
            "literal_event": "A cat sleeps on the windowsill",
            "semantic_focus": "sleep",
            "emotional_relation": "calm",
            "intensity": 1,
            "visual_relation": "containment",
            "memory_hook": "one paw hanging over the sill",
            "metaphor": {"mode": "none", "mapping": "", "literal_anchor": "sleeping cat"}
        },
        "page_design": {
            "rhythm": "single_tableau",
            "special_device": {
                "kind": "none",
                "reason": "ordinary geometry preserves the calm",
                "source_panel": "",
                "target_panel": "",
                "subject_id": ""
            },
            "eye_flow_summary": "the sill leads directly to the sleeping cat"
        },
        "panels": [{
                "shot_id": "s1",
                "narrative_role": "peak",
                "semantic_job": "show the complete sleeping cat on the sill",
                "attentional_frame": "mono",
                "narrative_weight": "primary",
                "transition_from_previous": "none",
                "continuity": {
                    "shared_environment_id": "",
                    "subject_phase": "",
                    "axis_relation_from_previous": "not_applicable",
                    "screen_direction": "stationary",
                    "eyeline_enabled": false,
                    "eyeline_looker_id": "",
                    "eyeline_target_anchor": "",
                    "eyeline_direction": "none",
                    "match_on_action_enabled": false,
                    "match_on_action_subject_id": "",
                    "match_on_action_action": ""
                },
                "scene": {
                    "description": "The complete cat sleeps quietly on the windowsill",
                    "subjects": [{
                        "id": "cat",
                        "figure": "small tabby cat",
                        "pose": "curled on the windowsill",
                        "expression": "eyes peacefully closed",
                        "blocking": "fully visible against the broad window"
                    }],
                    "environment": {
                        "setting": "sunlit apartment room",
                        "foreground": ["chair edge"],
                        "midground": ["windowsill"],
                        "background": ["blank skyline silhouettes"]
                    },
                    "camera": {
                        "shot_scale": "wide",
                        "viewpoint": "objective",
                        "viewpoint_subject_id": "",
                        "framing": "single",
                        "angle": "eye_level",
                        "focus": "room and windowsill",
                        "depth_plan": "deep",
                        "eye_flow_exit": "toward the cat on the right"
                    },
                    "motion_treatment": "none",
                    "lighting": "soft window light",
                    "mood": "calm"
                }
            }]
    })
}

/// Wrap one structured value as a successful Gemini text response.
fn scene_body(value: &Value) -> Result<TransportResponse> {
    body(json!({"candidates": [{"content": {"parts": [{"text": serde_json::to_string(value)?}]}}]}))
}

/// Wrap one structured value as a metered successful Gemini text response.
fn metered_scene_body(value: &Value) -> Result<TransportResponse> {
    body(json!({
        "candidates": [{"content": {"parts": [{"text": serde_json::to_string(value)?}]}}],
        "usageMetadata": {
            "promptTokenCount": 10,
            "candidatesTokenCount": 5,
            "totalTokenCount": 15
        }
    }))
}

/// Return one structured Gemini API failure.
fn api_failure(status: u16, code: &str, message: &str) -> TransportResponse {
    TransportResponse {
        status,
        body: json!({"error": {"status": code, "message": message}}).to_string(),
    }
}

/// Return the two responses consumed by the production scene pipeline.
fn scene_responses(scene: &Value) -> Result<Vec<Result<TransportResponse>>> {
    Ok(vec![
        Ok(scene_body(&scene_features())?),
        Ok(scene_body(scene)?),
    ])
}

/// Free-form completion keeps the legacy request bytes without a generation config.
#[test]
fn free_form_completion_keeps_the_legacy_request_bytes() -> Result<()> {
    let transport = FakeTransport::new(vec![Ok(body(json!({
        "candidates": [{"content": {"parts": [{"text": "ok"}]}}]
    }))?)]);
    let requests = transport.requests.clone();
    let client = GeminiClient::new("key", transport);
    let response = client.complete("gemini-3.6-flash", String::from("compose"))?;
    assert_eq!(
        (response.as_str(), requests.borrow()[0].1.as_str()),
        ("ok", r#"{"contents":[{"parts":[{"text":"compose"}]}]}"#,),
        "free-form completion request bytes drifted from the legacy contract"
    );
    Ok(())
}

/// Structured completion sends the JSON schema through responseFormat.text.
#[test]
fn structured_completion_uses_the_json_response_format() -> Result<()> {
    let transport = FakeTransport::new(vec![Ok(body(json!({
        "candidates": [{"content": {"parts": [{"text": "{\"panels\":[]}"}]}}]
    }))?)]);
    let requests = transport.requests.clone();
    let client = GeminiClient::new("key", transport);
    let response = client.complete_json(
        "gemini-3.6-flash",
        String::from("compose"),
        &json!({"type":"object","additionalProperties":false,"required":["panels"]}),
    )?;
    assert_eq!(
        (response.as_str(), requests.borrow()[0].1.as_str()),
        (
            r#"{"panels":[]}"#,
            r#"{"contents":[{"parts":[{"text":"compose"}]}],"generationConfig":{"responseFormat":{"text":{"mimeType":"APPLICATION_JSON","schema":{"additionalProperties":false,"required":["panels"],"type":"object"}}}}}"#,
        ),
        "structured completion request does not preserve the documented responseFormat.text shape"
    );
    Ok(())
}

/// Unsupported response-schema keywords fail before reaching the transport.
#[test]
fn unsupported_response_schema_keywords_fail_before_transport() {
    let transport = FakeTransport::new(Vec::new());
    let requests = transport.requests.clone();
    let result = GeminiClient::new("key", transport).complete_json(
        "gemini-3.6-flash",
        String::from("compose"),
        &json!({"type":"string","minLength":1}),
    );
    assert_eq!(
        (result.is_err(), requests.borrow().len()),
        (true, 0),
        "an undocumented response-schema keyword reached the Gemini transport"
    );
}

/// Malformed subschemas and nested unsupported keywords fail before reaching the transport.
#[test]
fn malformed_response_subschemas_fail_before_transport() {
    let transport = FakeTransport::new(Vec::new());
    let requests = transport.requests.clone();
    let client = GeminiClient::new("key", transport);
    let items = client.complete_json(
        "gemini-3.6-flash",
        String::from("compose"),
        &json!({"type":"array","items":[]}),
    );
    let additional = client.complete_json(
        "gemini-3.6-flash",
        String::from("compose"),
        &json!({"type":"object","additionalProperties":"false"}),
    );
    let nested = client.complete_json(
        "gemini-3.6-flash",
        String::from("compose"),
        &json!({
            "type":"object",
            "additionalProperties": {
                "type":"object",
                "properties": {"term":{"type":"string","minLength":1}}
            }
        }),
    );
    let nested_items = client.complete_json(
        "gemini-3.6-flash",
        String::from("compose"),
        &json!({"type":"array","items":{"type":"string","minLength":1}}),
    );
    let prefix_items = client.complete_json(
        "gemini-3.6-flash",
        String::from("compose"),
        &json!({"type":"array","prefixItems":[{"type":"string","minLength":1}]}),
    );
    assert_eq!(
        (
            items.is_err(),
            additional.is_err(),
            nested.is_err(),
            nested_items.is_err(),
            prefix_items.is_err(),
            requests.borrow().len()
        ),
        (true, true, true, true, true, 0),
        "a malformed or unsupported nested response schema reached the Gemini transport"
    );
}

/// JSON mode requests valid JSON without imposing a response schema.
#[test]
fn json_mode_uses_the_legacy_response_mime_type() -> Result<()> {
    let transport = FakeTransport::new(vec![Ok(body(json!({
        "candidates": [{"content": {"parts": [{"text": "{\"panels\":[]}"}]}}]
    }))?)]);
    let requests = transport.requests.clone();
    let client = GeminiClient::new("key", transport);
    let response = client.complete_json_mode("gemini-3.6-flash", String::from("compose"))?;
    assert_eq!(
        (response.as_str(), requests.borrow()[0].1.as_str()),
        (
            r#"{"panels":[]}"#,
            r#"{"contents":[{"parts":[{"text":"compose"}]}],"generationConfig":{"responseMimeType":"application/json"}}"#,
        ),
        "JSON mode request does not preserve the documented responseMimeType shape"
    );
    Ok(())
}

/// Understanding uses Flash and returns the multi-sense row shape.
#[test]
fn understanding_uses_flash_and_returns_sense_rows() -> Result<()> {
    let transport = FakeTransport::new(vec![Ok(body(json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "text": "{\"target_lang\":\"en\",\"items\":[{\"term\":\"wrecked\",\"senses\":[{\"understanding\":\"past tense of \\\"wreck\\\" — destroyed or crashed\",\"tag\":null}],\"selected\":0,\"ok\":true},{\"term\":\"окно\",\"senses\":[{\"understanding\":\"this is Russian, not the target language\",\"tag\":null}],\"selected\":0,\"ok\":false}]}"
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
            understood.candidates()[0].senses().len(),
            understood.candidates()[0].ok(),
            understood.candidates()[1].term(),
            understood.candidates()[1].ok(),
        ),
        (
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.6-flash:generateContent",
            true,
            "en",
            "wrecked",
            "past tense of \"wreck\" — destroyed or crashed",
            1,
            true,
            "окно",
            false,
        ),
        "understanding must use Flash, return human-language sense rows, and mark off-language rows ok=false"
    );
    Ok(())
}

/// Add more uses Flash and returns new senses.
#[test]
fn bulk_correction_uses_flash_and_returns_new_senses() -> Result<()> {
    let transport = FakeTransport::new(vec![Ok(body(json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "text": "{\"senses\":[{\"understanding\":\"Сущ. ставка игрока как банк раздачи.\",\"tag\":\"покер\"}],\"message\":null}"
                }]
            }
        }]
    }))?)]);
    let client = GeminiClient::new("key", transport);
    let updated = client.correct_bulk(
        &WordCandidate::new("wound", "ambiguous between noun and past-tense verb", true),
        "in poker",
        &LanguagePair::new("en", "ru"),
    )?;
    assert_eq!(
        (
            updated.senses()[0].understanding(),
            updated.senses()[0].tag(),
            updated.message_text()
        ),
        ("Сущ. ставка игрока как банк раздачи.", Some("покер"), None,),
        "bulk correction must use Flash output to append a tagged sense"
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
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.6-flash:generateContent",
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
    let requests = valid.requests.clone();
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
        (
            valid_ok,
            rejects_key(&rejected_error),
            requests.borrow()[0].0.as_str(),
        ),
        (
            true,
            true,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.6-flash:generateContent",
        ),
        "key validation must pass on any 2xx and flag an invalid-key response as a rejected key"
    );
}

/// Scene generation keeps typed analysis and schema-free semantic composition.
#[test]
fn scene_generation_uses_the_registry_as_the_only_production_path() -> Result<()> {
    let transport = FakeTransport::new(scene_responses(&dynamic_scene())?);
    let requests = transport.requests.clone();
    let client = GeminiClient::new("key", transport);
    let scene = client.scene(
        "English",
        "sleep",
        "The cat is sleeping on the windowsill",
        "en",
    )?;
    let endpoints = requests
        .borrow()
        .iter()
        .map(|request| request.0.clone())
        .collect::<Vec<_>>();
    let requests = requests
        .borrow()
        .iter()
        .map(|request| serde_json::from_str::<Value>(&request.1))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        (
            requests.len(),
            endpoints,
            (
                requests[0]
                    .pointer("/generationConfig/responseFormat/text/mimeType")
                    .and_then(Value::as_str),
                requests[0]
                    .pointer("/generationConfig/thinkingConfig/thinkingLevel")
                    .and_then(Value::as_str),
                requests[0]
                    .pointer("/generationConfig/maxOutputTokens")
                    .and_then(Value::as_u64),
                requests[1]
                    .pointer("/generationConfig/responseMimeType")
                    .and_then(Value::as_str),
                requests[1]
                    .pointer("/generationConfig/responseFormat")
                    .is_none(),
                requests[1]
                    .pointer("/generationConfig/thinkingConfig/thinkingLevel")
                    .and_then(Value::as_str),
                requests[1]
                    .pointer("/generationConfig/maxOutputTokens")
                    .and_then(Value::as_u64),
            ),
            (
                scene["manga_panel"]["meta"]["title"].as_str(),
                scene["manga_panel"]["meta"]["target_lang"].as_str(),
                scene["manga_panel"]["panels"][0]["bounds"]["x"].as_i64(),
                scene["manga_panel"]["panels"][0]["bounds"]["width"].as_i64(),
                scene["manga_panel"]["panels"][0]["scene"]["text_in_frame"].as_str(),
                scene["manga_panel"]["page_design"]["layout"]["template_id"].as_str(),
                scene["manga_panel"]["page_design"]["camera_arc"]["strategy"].as_str(),
            ),
        ),
        (
            2,
            vec![
                String::from(
                    "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.5-flash-lite:generateContent"
                ),
                String::from(
                    "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.6-flash:generateContent"
                ),
            ],
            (
                Some("APPLICATION_JSON"),
                Some("MINIMAL"),
                Some(4096),
                Some("application/json"),
                true,
                Some("LOW"),
                Some(8192),
            ),
            (
                Some("The cat is sleeping on the windowsill"),
                Some("en"),
                Some(16),
                Some(992),
                Some("none"),
                Some("splash-1-v1"),
                Some("single_view"),
            ),
        ),
        "public scene generation bypassed the typed-analysis and JSON-composition registry"
    );
    Ok(())
}

/// A rejected typed feature schema falls back once inside the same scene attempt.
#[test]
fn scene_feature_schema_rejection_falls_back_once_to_json_mode() -> Result<()> {
    let transport = FakeTransport::new(vec![
        Ok(api_failure(
            400,
            "INVALID_ARGUMENT",
            "response schema is too complex",
        )),
        Ok(metered_scene_body(&scene_features())?),
        Ok(metered_scene_body(&dynamic_scene())?),
    ]);
    let requests = transport.requests.clone();
    let client = GeminiClient::new("key", transport);
    let scene = client.scene(
        "English",
        "sleep",
        "The cat is sleeping on the windowsill",
        "en",
    )?;
    let requests = requests
        .borrow()
        .iter()
        .map(|request| serde_json::from_str::<Value>(&request.1))
        .collect::<Result<Vec<_>, _>>()?;
    let prompts = requests
        .iter()
        .map(|request| {
            request
                .pointer("/contents/0/parts/0/text")
                .and_then(Value::as_str)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        (
            scene["manga_panel"]["meta"]["title"].as_str(),
            requests.len(),
            requests[0]
                .pointer("/generationConfig/responseFormat/text/schema")
                .is_some(),
            requests[1]
                .pointer("/generationConfig/responseMimeType")
                .and_then(Value::as_str),
            requests[1]
                .pointer("/generationConfig/responseFormat")
                .is_none(),
            requests[2]
                .pointer("/generationConfig/responseMimeType")
                .and_then(Value::as_str),
            prompts[0] == prompts[1],
        ),
        (
            Some("The cat is sleeping on the windowsill"),
            3,
            true,
            Some("application/json"),
            true,
            Some("application/json"),
            true,
        ),
        "a typed feature-schema rejection did not make exactly one same-prompt JSON-mode fallback"
    );
    Ok(())
}

/// A second invalid-argument response stops the bounded schema fallback.
#[test]
fn scene_schema_fallback_stops_after_one_json_mode_retry() {
    let transport = FakeTransport::new(vec![
        Ok(api_failure(
            400,
            "INVALID_ARGUMENT",
            "feature response schema is too complex",
        )),
        Ok(api_failure(
            400,
            "INVALID_ARGUMENT",
            "JSON mode request is still invalid",
        )),
    ]);
    let requests = transport.requests.clone();
    let failed = GeminiClient::new("key", transport)
        .scene(
            "English",
            "sleep",
            "The cat is sleeping on the windowsill",
            "en",
        )
        .is_err();
    assert_eq!(
        (failed, requests.borrow().len()),
        (true, 2),
        "schema fallback retried more than once inside one artifact attempt"
    );
}

/// A transport that would reject a schema-bearing composer accepts production JSON mode.
#[test]
fn scene_composer_never_exposes_a_response_schema_to_transport() -> Result<()> {
    let transport = ComposerSchemaRejectingTransport::new();
    let requests = transport.requests.clone();
    let scene = GeminiClient::new("key", transport).scene(
        "English",
        "sleep",
        "The cat is sleeping on the windowsill",
        "en",
    )?;
    let requests = requests.borrow();
    assert_eq!(
        (
            scene["manga_panel"]["meta"]["title"].as_str(),
            requests.len(),
            requests[1]
                .pointer("/generationConfig/responseMimeType")
                .and_then(Value::as_str),
            requests[1]
                .pointer("/generationConfig/responseFormat")
                .is_none(),
        ),
        (
            Some("The cat is sleeping on the windowsill"),
            2,
            Some("application/json"),
            true
        ),
        "production composer regressed to the schema-bearing request rejected by Gemini"
    );
    Ok(())
}

/// Authentication, quota, and transport failures never enter schema fallback.
#[test]
fn scene_schema_fallback_excludes_non_schema_failures() {
    let key = FakeTransport::new(vec![Ok(api_failure(
        400,
        "INVALID_ARGUMENT",
        "API key not valid",
    ))]);
    let key_requests = key.requests.clone();
    let key_error = GeminiClient::new("key", key)
        .scene(
            "English",
            "sleep",
            "The cat is sleeping on the windowsill",
            "en",
        )
        .expect_err("invalid key must fail before schema fallback");
    let quota = FakeTransport::new(vec![Ok(api_failure(
        429,
        "RESOURCE_EXHAUSTED",
        "quota exhausted",
    ))]);
    let quota_requests = quota.requests.clone();
    let quota_failed = GeminiClient::new("key", quota)
        .scene(
            "English",
            "sleep",
            "The cat is sleeping on the windowsill",
            "en",
        )
        .is_err();
    let network = FakeTransport::new(vec![Err(anyhow::anyhow!("connection refused"))]);
    let network_requests = network.requests.clone();
    let network_failed = GeminiClient::new("key", network)
        .scene(
            "English",
            "sleep",
            "The cat is sleeping on the windowsill",
            "en",
        )
        .is_err();
    assert_eq!(
        (
            key_requests.borrow().len(),
            quota_requests.borrow().len(),
            network_requests.borrow().len(),
            rejects_key(&key_error),
            quota_failed,
            network_failed,
        ),
        (1, 1, 1, true, true, true),
        "schema fallback retried authentication, quota, or transport failures"
    );
}

/// JSON composition repairs only structurally unambiguous missing closers.
#[test]
fn scene_generation_repairs_one_truncated_json_closer() -> Result<()> {
    let scene = dynamic_scene();
    let mut raw = serde_json::to_string(&scene)?;
    raw.pop()
        .expect("invariant: scene fixture must contain one object closer");
    let mut responses = scene_responses(&scene)?;
    responses[1] = body(json!({
        "candidates": [{"content": {"parts": [{"text": raw}]}}]
    }));
    let client = GeminiClient::new("key", FakeTransport::new(responses));
    let output = client.scene(
        "English",
        "sleep",
        "The cat is sleeping on the windowsill",
        "en",
    )?;
    assert_eq!(
        output["manga_panel"]["meta"]["title"].as_str(),
        Some("The cat is sleeping on the windowsill"),
        "one unambiguous missing JSON closer discarded an otherwise valid scene"
    );
    Ok(())
}

/// Dynamic scenes preserve narrative fields while static template roots stay authoritative.
#[test]
fn dynamic_scene_generation_preserves_narrative_fields_and_static_roots() -> Result<()> {
    let mut output = dynamic_scene();
    output["meta"] = json!({"spec_version": "agent-override"});
    output["art_style"] = json!({"medium": {"base": "agent-override"}});
    output["panel_layout"] = json!({"special_device_budget": 0});
    let transport = FakeTransport::new(scene_responses(&output)?);
    let client = GeminiClient::new("key", transport);
    let scene = client.scene(
        "English",
        "sleep",
        "The cat is sleeping on the windowsill",
        "en",
    )?;
    assert_eq!(
        (
            scene["manga_panel"]["semantic_spine"]["memory_hook"].as_str(),
            scene["manga_panel"]["page_design"]["dominant_panel"].as_str(),
            scene["manga_panel"]["panels"][0]["id"].as_str(),
            scene["manga_panel"]["meta"]["spec_version"].as_str(),
            scene["manga_panel"]["art_style"]["medium"]["base"].as_str(),
            scene["manga_panel"]["panel_layout"]["special_device_budget"].as_i64(),
            scene["manga_panel"]["panel_layout"]["active_permissions"]["inset"].as_bool(),
            scene["manga_panel"]["panels"][0]["continuity"]["eyeline"]["enabled"].as_bool(),
            scene["manga_panel"]["panels"][0]["continuity"]["match_on_action"]["enabled"].as_bool(),
            scene["manga_panel"]["panels"][0]["continuity"]
                .get("eyeline_enabled")
                .is_none(),
        ),
        (
            Some("one paw hanging over the sill"),
            Some("p1"),
            Some("p1"),
            Some("2.0.0"),
            Some("indian_ink"),
            Some(1),
            Some(false),
            Some(false),
            Some(false),
            true,
        ),
        "dynamic scene fields were lost or agent output replaced static production policy"
    );
    Ok(())
}

/// Composer camera drift is locally canonicalized from the motivated shot plan.
#[test]
fn dynamic_scene_generation_canonicalizes_camera_plan_drift() -> Result<()> {
    let mut output = dynamic_scene();
    output["panels"][0]["scene"]["camera"]["shot_scale"] = json!("close");
    output["panels"][0]["scene"]["camera"]["viewpoint"] = json!("over_the_shoulder");
    output["panels"][0]["scene"]["camera"]["viewpoint_subject_id"] = json!("cat");
    output["panels"][0]["scene"]["camera"]["framing"] = json!("group");
    output["panels"][0]["scene"]["camera"]["angle"] = json!("dutch");
    output["panels"][0]["scene"]["camera"]["depth_plan"] = json!("flat");
    let transport = FakeTransport::new(scene_responses(&output)?);
    let client = GeminiClient::new("key", transport);
    let scene = client.scene(
        "English",
        "sleep",
        "The cat is sleeping on the windowsill",
        "en",
    )?;
    assert_eq!(
        (
            scene.pointer("/manga_panel/panels/0/scene/camera/shot_scale"),
            scene.pointer("/manga_panel/panels/0/scene/camera/viewpoint"),
            scene.pointer("/manga_panel/panels/0/scene/camera/viewpoint_subject_id"),
            scene.pointer("/manga_panel/panels/0/scene/camera/framing"),
            scene.pointer("/manga_panel/panels/0/scene/camera/angle"),
            scene.pointer("/manga_panel/panels/0/scene/camera/depth_plan"),
            scene.pointer("/manga_panel/panels/0/scene/description"),
            scene.pointer("/manga_panel/panels/0/scene/subjects/0/pose"),
            scene.pointer("/manga_panel/panels/0/scene/subjects/0/blocking"),
        ),
        (
            Some(&json!("wide")),
            Some(&json!("objective")),
            Some(&json!("")),
            Some(&json!("single")),
            Some(&json!("eye_level")),
            Some(&json!("deep")),
            Some(&json!("The complete cat sleeps quietly on the windowsill")),
            Some(&json!("curled on the windowsill")),
            Some(&json!("fully visible against the broad window")),
        ),
        "camera canonicalization changed the planned setup or semantic scene"
    );
    Ok(())
}

/// Image generation keeps the IMAGE modality and square aspect ratio.
#[test]
fn image_generation_keeps_the_image_modality_and_square_aspect_ratio() -> Result<()> {
    let transport = FakeTransport::new(vec![Ok(body(
        json!({"candidates":[{"content":{"parts":[{"inlineData":{"data":"AQID"}}]}}]}),
    )?)]);
    let requests = transport.requests.clone();
    let client = GeminiClient::new("key", transport);
    let _bytes = client.image("A cat sleeps in a finished black-and-white manga panel")?;
    let request = serde_json::from_str::<Value>(&requests.borrow()[0].1)?;
    assert_eq!(
        (
            requests.borrow()[0].0.clone(),
            request["contents"][0]["parts"][0]["text"].as_str(),
            request["generationConfig"]["responseModalities"][0].as_str(),
            request["generationConfig"]["imageConfig"]["aspectRatio"].as_str(),
            request["safetySettings"]
                .as_array()
                .map(|items| items.len())
        ),
        (
            String::from(
                "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.1-flash-image:generateContent"
            ),
            Some("A cat sleeps in a finished black-and-white manga panel"),
            Some("IMAGE"),
            Some("1:1"),
            Some(4),
        ),
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
            .image("A cat sleeps in a finished black-and-white manga panel")
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
