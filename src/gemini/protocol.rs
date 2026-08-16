use std::error::Error;
use std::fmt;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One protocol request to the Gemini API.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct Request {
    contents: Vec<Content>,
    #[serde(rename = "generationConfig", skip_serializing_if = "Option::is_none")]
    generation_config: Option<GenerationConfig>,
    #[serde(rename = "safetySettings", skip_serializing_if = "Option::is_none")]
    safety_settings: Option<Vec<SafetySetting>>,
}

impl Request {
    /// Return one text-only Gemini request.
    pub(super) fn text(
        text: String,
        generation_config: Option<GenerationConfig>,
        safety_settings: Option<Vec<SafetySetting>>,
    ) -> Self {
        Self {
            contents: vec![Content {
                parts: vec![RequestPart {
                    text: Some(text),
                    inline_data: None,
                }],
            }],
            generation_config,
            safety_settings,
        }
    }

    /// Return one text-plus-image Gemini request.
    pub(super) fn vision(
        text: String,
        mime_type: &str,
        data: String,
        generation_config: GenerationConfig,
    ) -> Self {
        Self {
            contents: vec![Content {
                parts: vec![
                    RequestPart {
                        text: Some(text),
                        inline_data: None,
                    },
                    RequestPart {
                        text: None,
                        inline_data: Some(RequestInlineData {
                            mime_type: String::from(mime_type),
                            data,
                        }),
                    },
                ],
            }],
            generation_config: Some(generation_config),
            safety_settings: Some(GenerationConfig::image_safety()),
        }
    }

    /// Return one text-plus-multiple-images Gemini request.
    pub(super) fn vision_images(
        text: String,
        mime_type: &str,
        data: Vec<String>,
        generation_config: GenerationConfig,
    ) -> Self {
        let mut parts = Vec::with_capacity(data.len().saturating_add(1));
        parts.push(RequestPart {
            text: Some(text),
            inline_data: None,
        });
        parts.extend(data.into_iter().map(|data| RequestPart {
            text: None,
            inline_data: Some(RequestInlineData {
                mime_type: String::from(mime_type),
                data,
            }),
        }));
        Self {
            contents: vec![Content { parts }],
            generation_config: Some(generation_config),
            safety_settings: Some(GenerationConfig::image_safety()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct Content {
    parts: Vec<RequestPart>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct RequestPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(rename = "inlineData", skip_serializing_if = "Option::is_none")]
    inline_data: Option<RequestInlineData>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct RequestInlineData {
    #[serde(rename = "mimeType")]
    mime_type: String,
    data: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct GenerationConfig {
    #[serde(rename = "responseModalities", skip_serializing_if = "Option::is_none")]
    response_modalities: Option<Vec<String>>,
    #[serde(rename = "imageConfig", skip_serializing_if = "Option::is_none")]
    image_config: Option<ImageConfig>,
    #[serde(rename = "speechConfig", skip_serializing_if = "Option::is_none")]
    speech_config: Option<SpeechConfig>,
    #[serde(rename = "responseFormat", skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
    #[serde(rename = "responseMimeType", skip_serializing_if = "Option::is_none")]
    response_mime_type: Option<String>,
    #[serde(rename = "responseSchema", skip_serializing_if = "Option::is_none")]
    response_schema: Option<Value>,
    #[serde(rename = "maxOutputTokens", skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(rename = "thinkingConfig", skip_serializing_if = "Option::is_none")]
    thinking_config: Option<ThinkingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<u8>,
    #[serde(rename = "mediaResolution", skip_serializing_if = "Option::is_none")]
    media_resolution: Option<MediaResolution>,
}

impl GenerationConfig {
    /// Return the image generation configuration.
    pub(super) fn image() -> Self {
        Self {
            response_modalities: Some(vec![String::from("IMAGE")]),
            image_config: Some(ImageConfig {
                aspect_ratio: String::from("1:1"),
            }),
            speech_config: None,
            response_format: None,
            response_mime_type: None,
            response_schema: None,
            max_output_tokens: None,
            thinking_config: None,
            temperature: None,
            media_resolution: None,
        }
    }

    /// Return the relaxed image safety settings for the current API shape.
    pub(super) fn image_safety() -> Vec<SafetySetting> {
        vec![
            SafetySetting::new("HARM_CATEGORY_HARASSMENT"),
            SafetySetting::new("HARM_CATEGORY_HATE_SPEECH"),
            SafetySetting::new("HARM_CATEGORY_SEXUALLY_EXPLICIT"),
            SafetySetting::new("HARM_CATEGORY_DANGEROUS_CONTENT"),
        ]
    }

    /// Return the audio generation configuration.
    pub(super) fn audio(voice: &str) -> Self {
        Self {
            response_modalities: Some(vec![String::from("AUDIO")]),
            image_config: None,
            speech_config: Some(SpeechConfig {
                voice_config: VoiceConfig {
                    prebuilt_voice_config: PrebuiltVoiceConfig {
                        voice_name: String::from(voice),
                    },
                },
            }),
            response_format: None,
            response_mime_type: None,
            response_schema: None,
            max_output_tokens: None,
            thinking_config: None,
            temperature: None,
            media_resolution: None,
        }
    }

    /// Return the structured JSON response configuration.
    pub(super) fn json(schema: Value) -> Result<Self> {
        validate_response_schema(&schema)?;
        Ok(Self {
            response_modalities: None,
            image_config: None,
            speech_config: None,
            response_format: Some(ResponseFormat {
                text: TextResponseFormat {
                    mime_type: String::from("APPLICATION_JSON"),
                    schema,
                },
            }),
            response_mime_type: None,
            response_schema: None,
            max_output_tokens: None,
            thinking_config: None,
            temperature: None,
            media_resolution: None,
        })
    }

    /// Return JSON mode without a response schema.
    pub(super) fn json_mode() -> Self {
        Self {
            response_modalities: None,
            image_config: None,
            speech_config: None,
            response_format: None,
            response_mime_type: Some(String::from("application/json")),
            response_schema: None,
            max_output_tokens: None,
            thinking_config: None,
            temperature: None,
            media_resolution: None,
        }
    }

    /// Return the bounded Gemini 3.5 structured vision-judge configuration.
    pub(super) fn vision_judge(schema: Value) -> Result<Self> {
        validate_response_schema(&schema)?;
        Ok(Self {
            response_modalities: None,
            image_config: None,
            speech_config: None,
            response_format: None,
            response_mime_type: Some(String::from("application/json")),
            response_schema: Some(schema),
            max_output_tokens: Some(256),
            thinking_config: None,
            temperature: Some(0),
            media_resolution: Some(MediaResolution::High),
        })
    }

    /// Return the Gemini 3.6 structured vision-judge configuration.
    pub(super) fn structured_vision_judge(schema: Value) -> Result<Self> {
        validate_response_schema(&schema)?;
        Ok(Self {
            response_modalities: None,
            image_config: None,
            speech_config: None,
            response_format: Some(ResponseFormat {
                text: TextResponseFormat {
                    mime_type: String::from("APPLICATION_JSON"),
                    schema,
                },
            }),
            response_mime_type: None,
            response_schema: None,
            max_output_tokens: None,
            thinking_config: None,
            temperature: Some(0),
            media_resolution: Some(MediaResolution::High),
        })
    }

    /// Return this configuration with one Gemini 3 thinking level.
    #[must_use]
    pub(super) fn with_thinking_level(mut self, level: ThinkingLevel) -> Self {
        self.thinking_config = Some(ThinkingConfig {
            thinking_level: Some(level),
            thinking_budget: None,
        });
        self
    }

    /// Return this configuration with one output-token ceiling.
    #[must_use]
    pub(super) fn with_max_output_tokens(mut self, tokens: u32) -> Self {
        self.max_output_tokens = Some(tokens);
        self
    }
}

/// One supported Gemini 3 thinking level for bounded scene generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(super) enum ThinkingLevel {
    Minimal,
    Low,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ThinkingConfig {
    #[serde(rename = "thinkingLevel", skip_serializing_if = "Option::is_none")]
    thinking_level: Option<ThinkingLevel>,
    #[serde(rename = "thinkingBudget", skip_serializing_if = "Option::is_none")]
    thinking_budget: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
enum MediaResolution {
    #[serde(rename = "MEDIA_RESOLUTION_HIGH")]
    High,
}

fn validate_response_schema(schema: &Value) -> Result<()> {
    let root = schema
        .as_object()
        .ok_or_else(|| anyhow!("Gemini response schema must be a JSON object"))?;
    for (keyword, value) in root {
        if !matches!(
            keyword.as_str(),
            "type"
                | "title"
                | "description"
                | "properties"
                | "required"
                | "additionalProperties"
                | "enum"
                | "format"
                | "minimum"
                | "maximum"
                | "items"
                | "prefixItems"
                | "minItems"
                | "maxItems"
        ) {
            bail!("Gemini response schema uses unsupported keyword '{keyword}'");
        }
        match keyword.as_str() {
            "properties" => {
                let properties = value.as_object().ok_or_else(|| {
                    anyhow!("Gemini response schema properties must be an object")
                })?;
                for property in properties.values() {
                    validate_response_schema(property)?;
                }
            }
            "items" => {
                if !value.is_object() {
                    bail!("Gemini response schema items must be an object");
                }
                validate_response_schema(value)?;
            }
            "additionalProperties" => match value {
                Value::Bool(_) => {}
                Value::Object(_) => validate_response_schema(value)?,
                _ => {
                    bail!(
                        "Gemini response schema additionalProperties must be a boolean or object"
                    );
                }
            },
            "prefixItems" => {
                let items = value.as_array().ok_or_else(|| {
                    anyhow!("Gemini response schema prefixItems must be an array")
                })?;
                for item in items {
                    validate_response_schema(item)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ResponseFormat {
    text: TextResponseFormat,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct TextResponseFormat {
    #[serde(rename = "mimeType")]
    mime_type: String,
    schema: Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ImageConfig {
    #[serde(rename = "aspectRatio")]
    aspect_ratio: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SpeechConfig {
    #[serde(rename = "voiceConfig")]
    voice_config: VoiceConfig,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct VoiceConfig {
    #[serde(rename = "prebuiltVoiceConfig")]
    prebuilt_voice_config: PrebuiltVoiceConfig,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct PrebuiltVoiceConfig {
    #[serde(rename = "voiceName")]
    voice_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct SafetySetting {
    category: String,
    threshold: String,
}

impl SafetySetting {
    /// Create one relaxed safety setting.
    fn new(category: &str) -> Self {
        Self {
            category: String::from(category),
            threshold: String::from("BLOCK_NONE"),
        }
    }
}

/// One parsed Gemini API response.
#[derive(Clone, Debug, Deserialize)]
pub(super) struct Response {
    #[serde(default)]
    pub(super) candidates: Vec<Candidate>,
    #[serde(rename = "usageMetadata")]
    pub(super) usage_metadata: Option<UsageMetadata>,
    #[serde(rename = "promptFeedback")]
    prompt_feedback: Option<PromptFeedback>,
}

impl Response {
    /// Return the first candidate's terminal generation reason when Gemini supplies one.
    pub(super) fn finish_reason(&self) -> Option<&str> {
        self.candidates
            .first()
            .and_then(|candidate| candidate.finish_reason.as_deref())
    }
}

/// Token usage returned by one Gemini response.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub(super) struct UsageMetadata {
    #[serde(rename = "promptTokenCount", default)]
    pub(super) prompt_token_count: u64,
    #[serde(rename = "candidatesTokenCount", default)]
    pub(super) candidates_token_count: u64,
    #[serde(rename = "thoughtsTokenCount", default)]
    pub(super) thoughts_token_count: u64,
    #[serde(rename = "totalTokenCount", default)]
    pub(super) total_token_count: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct Candidate {
    pub(super) content: Option<ResponseContent>,
    #[serde(rename = "finishReason")]
    finish_reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct ResponseContent {
    #[serde(default)]
    pub(super) parts: Vec<ResponsePart>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct ResponsePart {
    pub(super) text: Option<String>,
    #[serde(rename = "inlineData")]
    pub(super) inline_data: Option<InlineData>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct InlineData {
    pub(super) data: String,
}

#[derive(Clone, Debug, Deserialize)]
struct PromptFeedback {
    #[serde(rename = "blockReason")]
    block_reason: Option<String>,
    #[serde(rename = "blockReasonMessage")]
    block_reason_message: Option<String>,
    #[serde(rename = "safetyRatings", default)]
    safety_ratings: Vec<SafetyRating>,
}

#[derive(Clone, Debug, Deserialize)]
struct SafetyRating {
    category: String,
    probability: String,
    blocked: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
struct ErrorEnvelope {
    error: ApiError,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiError {
    status: Option<String>,
    message: Option<String>,
    #[serde(default)]
    details: Vec<ApiErrorDetail>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiErrorDetail {
    reason: Option<String>,
}

/// Structured Gemini API error returned by the REST API.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeminiApiError {
    http_status: u16,
    status: String,
    message: Option<String>,
    reasons: Vec<String>,
}

impl GeminiApiError {
    /// Create one Gemini API error from status, message, and detail reasons.
    pub fn new(status: impl Into<String>, message: Option<String>, reasons: Vec<String>) -> Self {
        Self {
            http_status: 0,
            status: status.into(),
            message,
            reasons,
        }
    }

    fn from_http(
        http_status: u16,
        status: impl Into<String>,
        message: Option<String>,
        reasons: Vec<String>,
    ) -> Self {
        Self {
            http_status,
            status: status.into(),
            message,
            reasons,
        }
    }

    /// Return whether the API response clearly rejects the configured key.
    #[must_use]
    pub fn rejects_key(&self) -> bool {
        if self.reasons.iter().any(|reason| key_reason(reason)) {
            return true;
        }
        if self
            .message
            .as_ref()
            .map(|message| key_message(message))
            .unwrap_or(false)
        {
            return true;
        }
        if self.status == "UNAUTHENTICATED" {
            return true;
        }
        false
    }

    /// Return whether one typed request was rejected as an invalid argument.
    #[must_use]
    pub(super) fn rejects_schema(&self) -> bool {
        self.http_status == 400 && self.status == "INVALID_ARGUMENT" && !self.rejects_key()
    }
}

impl fmt::Display for GeminiApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.status)?;
        if let Some(message) = self.message.as_ref()
            && !message.is_empty()
        {
            write!(formatter, ": {message}")?;
        }
        Ok(())
    }
}

impl Error for GeminiApiError {}

fn key_reason(reason: &str) -> bool {
    matches!(
        reason,
        "API_KEY_INVALID"
            | "API_KEY_SERVICE_BLOCKED"
            | "API_KEY_HTTP_REFERRER_BLOCKED"
            | "API_KEY_IP_ADDRESS_BLOCKED"
            | "API_KEY_ANDROID_APP_BLOCKED"
            | "API_KEY_IOS_APP_BLOCKED"
    )
}

fn key_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("api key")
        && (lower.contains("not valid")
            || lower.contains("invalid")
            || lower.contains("blocked")
            || lower.contains("disabled")
            || lower.contains("expired"))
}

/// Remove Markdown fences from one JSON payload.
pub(super) fn unfence(text: &str) -> &str {
    let mut value = text.trim();
    if let Some(item) = value.strip_prefix("```json") {
        value = item.trim_start();
    } else if let Some(item) = value.strip_prefix("```") {
        value = item.trim_start();
    }
    if let Some(item) = value.strip_suffix("```") {
        value = item.trim_end();
    }
    value
}

/// Enforce the frozen manga panel geometry and text constraints.
pub(super) fn enforce(scene: &mut Value) {
    let Some(items) = scene["manga_panel"]["panels"].as_array_mut() else {
        return;
    };
    for panel in items {
        panel["bleed"] = Value::Bool(false);
        let x = number(
            panel
                .pointer("/bounds/x")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .max(16),
        );
        let y = number(
            panel
                .pointer("/bounds/y")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .max(16),
        );
        let width = panel
            .pointer("/bounds/width")
            .and_then(Value::as_i64)
            .unwrap_or(992)
            .min(1008 - x.as_i64().unwrap_or(16));
        let height = panel
            .pointer("/bounds/height")
            .and_then(Value::as_i64)
            .unwrap_or(992)
            .min(1008 - y.as_i64().unwrap_or(16));
        panel["bounds"]["x"] = x;
        panel["bounds"]["y"] = y;
        panel["bounds"]["width"] = number(width);
        panel["bounds"]["height"] = number(height);
        if panel["scene"].is_object() {
            panel["scene"]["text_in_frame"] = Value::String(String::from("none"));
        } else if panel["description"].is_string() {
            panel["text_in_frame"] = Value::String(String::from("none"));
        }
    }
}

/// Validate one enforced scene payload.
pub(super) fn validate(scene: &Value) -> Result<()> {
    let Some(items) = scene["manga_panel"]["panels"].as_array() else {
        bail!("No panels found in scene JSON");
    };
    if items.is_empty() {
        bail!("No panels found in scene JSON");
    }
    Ok(())
}

/// Return one diagnostic summary for a blocked Gemini response.
pub(super) fn diagnosis(response: &Response) -> String {
    let Some(feedback) = response.prompt_feedback.as_ref() else {
        return String::from("no prompt_feedback");
    };
    let mut parts = vec![
        feedback
            .block_reason
            .clone()
            .unwrap_or_else(|| String::from("unknown")),
    ];
    if let Some(message) = feedback.block_reason_message.as_ref()
        && !message.is_empty()
    {
        parts.push(message.clone());
    }
    let flagged = feedback
        .safety_ratings
        .iter()
        .filter(|item| {
            item.blocked.unwrap_or(false)
                || !matches!(item.probability.as_str(), "NEGLIGIBLE" | "LOW")
        })
        .map(|item| format!("{}={}", item.category, item.probability))
        .collect::<Vec<_>>();
    if !flagged.is_empty() {
        parts.push(format!("flagged=[{}]", flagged.join(", ")));
    }
    parts.join(", ")
}

/// One model reply that arrived intact but did not survive decoding, kept
/// verbatim so the caller can archive what was actually thrown away.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RejectedReply {
    stage: &'static str,
    body: String,
}

impl RejectedReply {
    /// Wrap the reply of one named stage.
    #[must_use]
    pub fn new(stage: &'static str, body: impl Into<String>) -> Self {
        Self {
            stage,
            body: body.into(),
        }
    }

    /// Return the reply verbatim, exactly as the model sent it.
    #[must_use]
    pub fn body(&self) -> &str {
        self.body.as_str()
    }
}

impl fmt::Display for RejectedReply {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "the {} reply was rejected", self.stage)
    }
}

impl Error for RejectedReply {}

/// Convert one error body into a typed anyhow error.
pub(super) fn api_error(http_status: u16, body: &str) -> anyhow::Error {
    match serde_json::from_str::<ErrorEnvelope>(body) {
        Ok(error) => {
            let reasons = error
                .error
                .details
                .into_iter()
                .filter_map(|detail| detail.reason)
                .collect();
            anyhow!(GeminiApiError::from_http(
                http_status,
                error
                    .error
                    .status
                    .unwrap_or_else(|| String::from("UNKNOWN")),
                error.error.message,
                reasons,
            ))
        }
        Err(_) => anyhow!(body.to_owned()),
    }
}

fn number(value: i64) -> Value {
    Value::Number(value.into())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn legacy_text_image_and_audio_requests_keep_their_exact_bytes() {
        let text = serde_json::to_string(&Request::text(String::from("compose"), None, None))
            .expect("text request must serialize");
        let image = serde_json::to_string(&Request::text(
            String::from("scene"),
            Some(GenerationConfig::image()),
            Some(GenerationConfig::image_safety()),
        ))
        .expect("image request must serialize");
        let audio = serde_json::to_string(&Request::text(
            String::from("speak"),
            Some(GenerationConfig::audio("Aoede")),
            None,
        ))
        .expect("audio request must serialize");
        assert_eq!(
            (text.as_str(), image.as_str(), audio.as_str()),
            (
                r#"{"contents":[{"parts":[{"text":"compose"}]}]}"#,
                r#"{"contents":[{"parts":[{"text":"scene"}]}],"generationConfig":{"responseModalities":["IMAGE"],"imageConfig":{"aspectRatio":"1:1"}},"safetySettings":[{"category":"HARM_CATEGORY_HARASSMENT","threshold":"BLOCK_NONE"},{"category":"HARM_CATEGORY_HATE_SPEECH","threshold":"BLOCK_NONE"},{"category":"HARM_CATEGORY_SEXUALLY_EXPLICIT","threshold":"BLOCK_NONE"},{"category":"HARM_CATEGORY_DANGEROUS_CONTENT","threshold":"BLOCK_NONE"}]}"#,
                r#"{"contents":[{"parts":[{"text":"speak"}]}],"generationConfig":{"responseModalities":["AUDIO"],"speechConfig":{"voiceConfig":{"prebuiltVoiceConfig":{"voiceName":"Aoede"}}}}}"#,
            ),
            "a legacy Gemini request changed while structured output was added"
        );
    }

    #[test]
    fn structured_json_scene_controls_serialize_as_exact_rest_json() {
        let config = GenerationConfig::json(json!({
            "type": "object",
            "properties": {
                "term": {"type": "string"}
            },
            "required": ["term"]
        }))
        .expect("scene schema must be supported")
        .with_thinking_level(ThinkingLevel::Low)
        .with_max_output_tokens(4096);
        let request =
            serde_json::to_string(&Request::text(String::from("features"), Some(config), None))
                .expect("scene request must serialize");
        assert_eq!(
            request,
            r#"{"contents":[{"parts":[{"text":"features"}]}],"generationConfig":{"responseFormat":{"text":{"mimeType":"APPLICATION_JSON","schema":{"properties":{"term":{"type":"string"}},"required":["term"],"type":"object"}}},"maxOutputTokens":4096,"thinkingConfig":{"thinkingLevel":"LOW"}}}"#,
            "structured scene controls changed their Gemini REST shape"
        );
    }

    #[test]
    fn json_mode_scene_controls_serialize_minimal_without_token_ceiling() {
        let config = GenerationConfig::json_mode().with_thinking_level(ThinkingLevel::Minimal);
        let request =
            serde_json::to_string(&Request::text(String::from("compose"), Some(config), None))
                .expect("scene request must serialize");
        assert_eq!(
            request,
            r#"{"contents":[{"parts":[{"text":"compose"}]}],"generationConfig":{"responseMimeType":"application/json","thinkingConfig":{"thinkingLevel":"MINIMAL"}}}"#,
            "minimal JSON-mode thinking changed its Gemini REST shape"
        );
    }

    #[test]
    fn ordinary_json_requests_omit_scene_controls() {
        let structured = serde_json::to_string(&Request::text(
            String::from("meta"),
            Some(
                GenerationConfig::json(json!({
                    "type": "object",
                    "properties": {
                        "term": {"type": "string"}
                    }
                }))
                .expect("meta schema must be supported"),
            ),
            None,
        ))
        .expect("structured request must serialize");
        let mode = serde_json::to_string(&Request::text(
            String::from("fallback"),
            Some(GenerationConfig::json_mode()),
            None,
        ))
        .expect("JSON-mode request must serialize");
        assert_eq!(
            (structured.as_str(), mode.as_str()),
            (
                r#"{"contents":[{"parts":[{"text":"meta"}]}],"generationConfig":{"responseFormat":{"text":{"mimeType":"APPLICATION_JSON","schema":{"properties":{"term":{"type":"string"}},"type":"object"}}}}}"#,
                r#"{"contents":[{"parts":[{"text":"fallback"}]}],"generationConfig":{"responseMimeType":"application/json"}}"#,
            ),
            "ordinary JSON requests unexpectedly acquired scene controls"
        );
    }

    #[test]
    fn response_schema_whitelist_distinguishes_keywords_from_property_names() {
        let supported = GenerationConfig::json(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "minLength": {
                    "type": "array",
                    "items": {"type": "string"},
                    "minItems": 1,
                    "maxItems": 4
                }
            },
            "required": ["minLength"]
        }));
        let unsupported = GenerationConfig::json(json!({
            "type": "object",
            "properties": {
                "term": {"type": "string", "minLength": 1}
            }
        }));
        assert_eq!(
            (supported.is_ok(), unsupported.is_err()),
            (true, true),
            "schema whitelist rejected a property name or accepted a nested unsupported keyword"
        );
    }

    #[test]
    fn response_schema_whitelist_rejects_invalid_subschema_shapes() {
        let scalar_items = GenerationConfig::json(json!({
            "type": "array",
            "items": "string"
        }));
        let array_items = GenerationConfig::json(json!({
            "type": "array",
            "items": [{"type": "string"}]
        }));
        let scalar_additional = GenerationConfig::json(json!({
            "type": "object",
            "additionalProperties": "false"
        }));
        let array_additional = GenerationConfig::json(json!({
            "type": "object",
            "additionalProperties": [{"type": "string"}]
        }));
        assert_eq!(
            (
                scalar_items.is_err(),
                array_items.is_err(),
                scalar_additional.is_err(),
                array_additional.is_err()
            ),
            (true, true, true, true),
            "schema whitelist accepted a malformed items or additionalProperties subschema"
        );
    }

    #[test]
    fn response_schema_whitelist_recurses_through_additional_properties() {
        let boolean = GenerationConfig::json(json!({
            "type": "object",
            "additionalProperties": true
        }));
        let object = GenerationConfig::json(json!({
            "type": "object",
            "additionalProperties": {"type": "string"}
        }));
        let nested_unsupported = GenerationConfig::json(json!({
            "type": "object",
            "additionalProperties": {
                "type": "object",
                "properties": {
                    "term": {"type": "string", "minLength": 1}
                }
            }
        }));
        let nested_items_unsupported = GenerationConfig::json(json!({
            "type": "array",
            "items": {"type": "string", "minLength": 1}
        }));
        assert_eq!(
            (
                boolean.is_ok(),
                object.is_ok(),
                nested_unsupported.is_err(),
                nested_items_unsupported.is_err()
            ),
            (true, true, true, true),
            "a supported subschema form was rejected or nested validation was bypassed"
        );
    }
}
