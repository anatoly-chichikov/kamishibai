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
                parts: vec![RequestPart { text: Some(text) }],
            }],
            generation_config,
            safety_settings,
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
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct GenerationConfig {
    #[serde(rename = "responseModalities", skip_serializing_if = "Option::is_none")]
    response_modalities: Option<Vec<String>>,
    #[serde(rename = "imageConfig", skip_serializing_if = "Option::is_none")]
    image_config: Option<ImageConfig>,
    #[serde(rename = "speechConfig", skip_serializing_if = "Option::is_none")]
    speech_config: Option<SpeechConfig>,
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
        }
    }
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

/// Token usage returned by one Gemini response.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub(super) struct UsageMetadata {
    #[serde(rename = "promptTokenCount", default)]
    pub(super) prompt_token_count: u64,
    #[serde(rename = "candidatesTokenCount", default)]
    pub(super) candidates_token_count: u64,
    #[serde(rename = "totalTokenCount", default)]
    pub(super) total_token_count: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct Candidate {
    pub(super) content: Option<ResponseContent>,
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
    status: String,
    message: Option<String>,
    reasons: Vec<String>,
}

impl GeminiApiError {
    /// Create one Gemini API error from status, message, and detail reasons.
    pub fn new(status: impl Into<String>, message: Option<String>, reasons: Vec<String>) -> Self {
        Self {
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

/// Convert one error body into a typed anyhow error.
pub(super) fn api_error(body: &str) -> anyhow::Error {
    match serde_json::from_str::<ErrorEnvelope>(body) {
        Ok(error) => {
            let reasons = error
                .error
                .details
                .into_iter()
                .filter_map(|detail| detail.reason)
                .collect();
            anyhow!(GeminiApiError::new(
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
