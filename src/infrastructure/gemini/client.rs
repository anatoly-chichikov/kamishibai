use std::env;

use anyhow::{Result, bail};
use rand::RngExt;
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::Value;

use crate::infrastructure::assets;

use super::codec::decode;
use super::protocol::{
    GenerationConfig, Request, Response, api_error, diagnosis, enforce, exhausted, unfence,
    validate,
};

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

fn voice() -> &'static str {
    let mut rng = rand::rng();
    let index = rng.random_range(0..VOICES.len());
    VOICES[index]
}
