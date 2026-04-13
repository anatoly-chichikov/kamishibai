//! Direct REST client for Gemini text, image, and TTS generation.

use std::env;

use anyhow::{Result, anyhow, bail};
use rand::RngExt;
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::assets;

const BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const SCENE_MODEL: &str = "gemini-3-flash-preview";
const IMAGE_MODEL: &str = "gemini-3.1-flash-image-preview";
const TTS_MODELS: [&str; 2] = ["gemini-2.5-flash-preview-tts", "gemini-2.5-pro-preview-tts"];
const VOICES: [&str; 30] = [
    "Achernar",
    "Achird",
    "Algenib",
    "Algieba",
    "Alnilam",
    "Aoede",
    "Autonoe",
    "Callirrhoe",
    "Charon",
    "Despina",
    "Enceladus",
    "Erinome",
    "Fenrir",
    "Gacrux",
    "Iapetus",
    "Kore",
    "Laomedeia",
    "Leda",
    "Orus",
    "Puck",
    "Pulcherrima",
    "Rasalgethi",
    "Sadachbia",
    "Sadaltager",
    "Schedar",
    "Sulafat",
    "Umbriel",
    "Vindemiatrix",
    "Zephyr",
    "Zubenelgenubi",
];

/// One transport response from the Gemini API.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransportResponse {
    pub status: u16,
    pub body: String,
}

/// Execute one POST request for the Gemini API.
pub trait Transport {
    /// Execute one JSON request and return the raw response.
    fn post(&self, url: &str, key: &str, body: &str) -> Result<TransportResponse>;
}

/// HTTP transport backed by reqwest.
#[derive(Clone, Debug, Default)]
pub struct HttpTransport {
    client: Client,
}

impl HttpTransport {
    /// Create one HTTP transport.
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

impl Transport for HttpTransport {
    /// Execute one JSON request and return the raw response.
    fn post(&self, url: &str, key: &str, body: &str) -> Result<TransportResponse> {
        let mut headers = HeaderMap::new();
        headers.insert("x-goog-api-key", HeaderValue::from_str(key)?);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let response = self
            .client
            .post(url)
            .headers(headers)
            .body(String::from(body))
            .send()?;
        Ok(TransportResponse {
            status: response.status().as_u16(),
            body: response.text()?,
        })
    }
}

/// Direct Gemini client with a pluggable transport.
#[derive(Clone, Debug)]
pub struct GeminiClient<T> {
    key: String,
    transport: T,
}

impl GeminiClient<HttpTransport> {
    /// Build the live Gemini client from GEMINI_API_KEY.
    pub fn from_env() -> Result<Self> {
        let Some(key) = env::var("GEMINI_API_KEY").ok() else {
            bail!("GEMINI_API_KEY environment variable is not set; export it before running");
        };
        Ok(Self::new(key, HttpTransport::new()))
    }
}

impl<T> GeminiClient<T>
where
    T: Transport,
{
    /// Create one Gemini client from an API key and transport.
    pub fn new(key: impl Into<String>, transport: T) -> Self {
        Self {
            key: key.into(),
            transport,
        }
    }

    /// Return the fixed TTS voice pool.
    pub fn voices(&self) -> &'static [&'static str; 30] {
        &VOICES
    }

    /// Translate one sentence into the enforced manga scene JSON shape.
    pub fn scene(&self, language: &str, sentence: &str, target: &str) -> Result<Value> {
        let prompt = assets::render_scene_prompt(language).replace("{sentence}", sentence);
        let response = self.request(SCENE_MODEL, &Request::text(prompt, None, None))?;
        let raw = response
            .candidates
            .iter()
            .flat_map(|candidate| candidate.content.as_ref().into_iter())
            .flat_map(|content| content.parts.iter())
            .filter_map(|part| part.text.as_ref())
            .cloned()
            .collect::<String>();
        let cleaned = unfence(raw.trim());
        let panels = serde_json::from_str::<Value>(cleaned)?;
        let Some(items) = panels.as_array() else {
            bail!("Expected a JSON array of panels");
        };
        let mut scene = serde_json::from_str::<Value>(assets::manga_template())?;
        scene["manga_panel"]["panels"] = Value::Array(items.clone());
        scene["manga_panel"]["meta"]["title"] = Value::String(sentence.chars().take(60).collect());
        scene["manga_panel"]["meta"]["description"] = Value::String(String::from(sentence));
        scene["manga_panel"]["meta"]["target_lang"] = Value::String(String::from(target));
        enforce(&mut scene);
        validate(&scene)?;
        Ok(scene)
    }

    /// Render one scene JSON payload into raw image bytes.
    pub fn image(&self, scene: &Value, word: &str) -> Result<Vec<u8>> {
        let response = self.request(
            IMAGE_MODEL,
            &Request::text(
                serde_json::to_string_pretty(scene)?,
                Some(GenerationConfig::image()),
                Some(GenerationConfig::image_safety()),
            ),
        )?;
        if response.candidates.is_empty() {
            bail!(
                "No candidates in image response for '{}': {}",
                word,
                diagnosis(&response)
            );
        }
        let Some(content) = response.candidates[0].content.as_ref() else {
            bail!("No content in image response for '{}'", word);
        };
        for part in &content.parts {
            if let Some(data) = part.inline_data.as_ref() {
                return decode(&data.data);
            }
        }
        bail!("No image data found in response for '{}'", word);
    }

    /// Generate one PCM audio payload with RESOURCE_EXHAUSTED fallback.
    pub fn speech(&self, prompt: &str, text: &str) -> Result<Vec<u8>> {
        let voice = voice();
        for model in TTS_MODELS {
            match self.request(
                model,
                &Request::text(
                    String::from(prompt),
                    Some(GenerationConfig::audio(voice)),
                    None,
                ),
            ) {
                Ok(response) => {
                    if response.candidates.is_empty() {
                        bail!("No candidates in audio response for '{}'", text);
                    }
                    let Some(content) = response.candidates[0].content.as_ref() else {
                        bail!("No content in audio response for '{}'", text);
                    };
                    let Some(data) = content
                        .parts
                        .iter()
                        .find_map(|part| part.inline_data.as_ref())
                    else {
                        bail!("No content in audio response for '{}'", text);
                    };
                    return decode(&data.data);
                }
                Err(error) if exhausted(&error) => continue,
                Err(error) => return Err(error),
            }
        }
        bail!("Failed to generate audio on all models for '{}'", text);
    }

    fn request(&self, model: &str, request: &Request) -> Result<Response> {
        let url = format!("{BASE_URL}/{model}:generateContent");
        let body = serde_json::to_string(request)?;
        let response = self
            .transport
            .post(url.as_str(), self.key.as_str(), body.as_str())?;
        if !(200..300).contains(&response.status) {
            return Err(api_error(response.body.as_str()));
        }
        Ok(serde_json::from_str(&response.body)?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct Request {
    contents: Vec<Content>,
    #[serde(rename = "generationConfig", skip_serializing_if = "Option::is_none")]
    generation_config: Option<GenerationConfig>,
    #[serde(rename = "safetySettings", skip_serializing_if = "Option::is_none")]
    safety_settings: Option<Vec<SafetySetting>>,
}

impl Request {
    fn text(
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
struct GenerationConfig {
    #[serde(rename = "responseModalities", skip_serializing_if = "Option::is_none")]
    response_modalities: Option<Vec<String>>,
    #[serde(rename = "imageConfig", skip_serializing_if = "Option::is_none")]
    image_config: Option<ImageConfig>,
    #[serde(rename = "speechConfig", skip_serializing_if = "Option::is_none")]
    speech_config: Option<SpeechConfig>,
}

impl GenerationConfig {
    fn image() -> Self {
        Self {
            response_modalities: Some(vec![String::from("IMAGE")]),
            image_config: Some(ImageConfig {
                aspect_ratio: String::from("1:1"),
            }),
            speech_config: None,
        }
    }

    /// Return the relaxed image safety settings for the current API shape.
    fn image_safety() -> Vec<SafetySetting> {
        vec![
            SafetySetting::new("HARM_CATEGORY_HARASSMENT"),
            SafetySetting::new("HARM_CATEGORY_HATE_SPEECH"),
            SafetySetting::new("HARM_CATEGORY_SEXUALLY_EXPLICIT"),
            SafetySetting::new("HARM_CATEGORY_DANGEROUS_CONTENT"),
        ]
    }

    fn audio(voice: &str) -> Self {
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
struct SafetySetting {
    category: String,
    threshold: String,
}

impl SafetySetting {
    fn new(category: &str) -> Self {
        Self {
            category: String::from(category),
            threshold: String::from("BLOCK_NONE"),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct Response {
    #[serde(default)]
    candidates: Vec<Candidate>,
    #[serde(rename = "promptFeedback")]
    prompt_feedback: Option<PromptFeedback>,
}

#[derive(Clone, Debug, Deserialize)]
struct Candidate {
    content: Option<ResponseContent>,
}

#[derive(Clone, Debug, Deserialize)]
struct ResponseContent {
    #[serde(default)]
    parts: Vec<ResponsePart>,
}

#[derive(Clone, Debug, Deserialize)]
struct ResponsePart {
    text: Option<String>,
    #[serde(rename = "inlineData")]
    inline_data: Option<InlineData>,
}

#[derive(Clone, Debug, Deserialize)]
struct InlineData {
    data: String,
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
}

fn unfence(text: &str) -> &str {
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

fn enforce(scene: &mut Value) {
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

fn validate(scene: &Value) -> Result<()> {
    let Some(items) = scene["manga_panel"]["panels"].as_array() else {
        bail!("No panels found in scene JSON");
    };
    if items.is_empty() {
        bail!("No panels found in scene JSON");
    }
    Ok(())
}

fn diagnosis(response: &Response) -> String {
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

fn voice() -> &'static str {
    let mut rng = rand::rng();
    let index = rng.random_range(0..VOICES.len());
    VOICES[index]
}

fn api_error(body: &str) -> anyhow::Error {
    match serde_json::from_str::<ErrorEnvelope>(body) {
        Ok(error) => anyhow!(
            "{}{}",
            error
                .error
                .status
                .unwrap_or_else(|| String::from("UNKNOWN")),
            error
                .error
                .message
                .map(|message| format!(": {message}"))
                .unwrap_or_default()
        ),
        Err(_) => anyhow!(body.to_owned()),
    }
}

fn exhausted(error: &anyhow::Error) -> bool {
    error.to_string().contains("RESOURCE_EXHAUSTED")
}

fn number(value: i64) -> Value {
    Value::Number(value.into())
}

fn decode(data: &str) -> Result<Vec<u8>> {
    let mut value = Vec::new();
    let mut block = Vec::new();
    for item in data.chars().filter(|item| !item.is_whitespace()) {
        if item == '=' {
            block.push(64);
        } else {
            block.push(code(item)?);
        }
        if block.len() == 4 {
            append(&mut value, &block)?;
            block.clear();
        }
    }
    if !block.is_empty() {
        bail!("Malformed base64 response payload");
    }
    Ok(value)
}

fn code(item: char) -> Result<u8> {
    match item {
        'A'..='Z' => Ok((item as u8) - b'A'),
        'a'..='z' => Ok((item as u8) - b'a' + 26),
        '0'..='9' => Ok((item as u8) - b'0' + 52),
        '+' => Ok(62),
        '/' => Ok(63),
        _ => bail!("Malformed base64 response payload"),
    }
}

fn append(value: &mut Vec<u8>, block: &[u8]) -> Result<()> {
    if block.len() != 4 {
        bail!("Malformed base64 response payload");
    }
    let first = (block[0] << 2) | (block[1] >> 4);
    value.push(first);
    if block[2] == 64 {
        return Ok(());
    }
    let second = ((block[1] & 0x0f) << 4) | (block[2] >> 2);
    value.push(second);
    if block[3] == 64 {
        return Ok(());
    }
    let third = ((block[2] & 0x03) << 6) | block[3];
    value.push(third);
    Ok(())
}
