use std::env;

use anyhow::{Result, anyhow, bail};
use rand::RngExt;
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::Value;

use crate::generation::{manga_template, render_scene_prompt};
use crate::languages::catalog;
use crate::session::{
    CardDraft, CardMeta, CardRevision, LanguagePair, RawInputBatch, Sense, SenseCorrection,
    TargetGuess, Understood, WordCandidate,
};

use super::codec::decode;
use super::prompts::{
    render_bulk_prompt, render_card_meta_prompt, render_card_prompt, render_intake_prompt,
};
use super::protocol::{
    GenerationConfig, Request, Response, api_error, diagnosis, enforce, unfence, validate,
};

const BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const TEXT_MODEL: &str = "gemini-3.5-flash";
const META_MODEL: &str = TEXT_MODEL;
const SCENE_MODEL: &str = TEXT_MODEL;
const IMAGE_MODEL: &str = "gemini-3.1-flash-image-preview";
const TTS_MODEL: &str = "gemini-3.1-flash-tts-preview";
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
    /// Build the live Gemini client from a saved key.
    pub fn from_saved(saved: Option<&str>) -> Result<Self> {
        let key = saved
            .filter(|value| !value.is_empty())
            .map(String::from)
            .ok_or_else(|| {
                anyhow!(
                    "no Gemini API key found in saved preferences; open Welcome and paste one or load GEMINI_API_KEY"
                )
            })?;
        Ok(Self::new(key, HttpTransport::new()))
    }

    /// Build the live Gemini client from `GEMINI_API_KEY`, falling back to a
    /// saved key. The env value wins when both are present so a shell-set key
    /// always overrides whatever was last persisted through the Welcome
    /// screen.
    pub fn from_env_or_saved(saved: Option<&str>) -> Result<Self> {
        let env_key = env::var("GEMINI_API_KEY")
            .ok()
            .filter(|value| !value.is_empty());
        let key = env_key
            .or_else(|| saved.filter(|value| !value.is_empty()).map(String::from))
            .ok_or_else(|| {
                anyhow!(
                    "no Gemini API key found in GEMINI_API_KEY or saved preferences; \
                     set GEMINI_API_KEY or start the TUI without WORDS_JSON and paste one on Welcome"
                )
            })?;
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
        let prompt = render_scene_prompt(language).replace("{sentence}", sentence);
        let raw = self.text(SCENE_MODEL, prompt)?;
        let cleaned = unfence(raw.trim());
        let panels = serde_json::from_str::<Value>(cleaned)?;
        let Some(items) = panels.as_array() else {
            bail!("Expected a JSON array of panels");
        };
        let mut scene = serde_json::from_str::<Value>(manga_template())?;
        scene["manga_panel"]["panels"] = Value::Array(items.clone());
        scene["manga_panel"]["meta"]["title"] = Value::String(sentence.chars().take(60).collect());
        scene["manga_panel"]["meta"]["description"] = Value::String(String::from(sentence));
        scene["manga_panel"]["meta"]["target_lang"] = Value::String(String::from(target));
        enforce(&mut scene);
        validate(&scene)?;
        Ok(scene)
    }

    /// Send one free-form prompt to a text model and return the raw textual
    /// response. Used by eval/dev tooling that swaps prompts without going
    /// through the typed `understand` / `generate_card_meta` paths.
    pub fn complete(&self, model: &str, prompt: String) -> Result<String> {
        self.text(model, prompt)
    }

    /// Probe the API with one tiny request to confirm the key is accepted.
    ///
    /// Any `2xx` counts as a valid key — the body is not parsed, so a thin or
    /// unusual completion still passes. A non-`2xx` is mapped through
    /// `api_error`, so the caller can tell a rejected key (`rejects_key`) from a
    /// transport or quota failure and message accordingly.
    pub fn validate_key(&self) -> Result<()> {
        let url = format!("{BASE_URL}/{TEXT_MODEL}:generateContent");
        let body = serde_json::to_string(&Request::text(String::from("ping"), None, None))?;
        let response = self
            .transport
            .post(url.as_str(), self.key.as_str(), body.as_str())?;
        if (200..300).contains(&response.status) {
            return Ok(());
        }
        Err(api_error(response.body.as_str()))
    }

    /// Resolve raw user input into reviewed rows using the Flash text model.
    pub fn understand(&self, raw: &RawInputBatch, my: &str) -> Result<Understood> {
        let catalog = catalog();
        let prompt = render_intake_prompt(raw.text(), my, &catalog)?;
        let decoded: IntakeResponse =
            serde_json::from_str(unfence(self.text(TEXT_MODEL, prompt)?.trim()))?;
        Ok(Understood::new(
            TargetGuess::new(decoded.target_lang, true),
            decoded
                .items
                .into_iter()
                .map(IntakeItem::candidate)
                .collect::<Result<Vec<_>>>()?,
        ))
    }

    /// Add missing senses after a user request from the review picker.
    pub fn correct_bulk(
        &self,
        candidate: &WordCandidate,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<SenseCorrection> {
        let catalog = catalog();
        let prompt = render_bulk_prompt(candidate, comment, pair, &catalog)?;
        let decoded: SenseCorrectionResponse =
            serde_json::from_str(unfence(self.text(TEXT_MODEL, prompt)?.trim()))?;
        Ok(decoded.correction())
    }

    /// Build the rich card meta for one term using the Flash text model.
    pub fn generate_card_meta(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
    ) -> Result<CardMeta> {
        let catalog = catalog();
        let prompt = render_card_meta_prompt(term, understanding, pair, &catalog)?;
        let decoded: CardMetaResponse =
            serde_json::from_str(unfence(self.text(META_MODEL, prompt)?.trim()))?;
        Ok(decoded.into_meta())
    }

    /// Recompose one card draft after a per-card refinement.
    pub fn correct_card(
        &self,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<CardRevision> {
        let catalog = catalog();
        let prompt = render_card_prompt(draft, comment, pair, &catalog)?;
        let decoded: CardCorrectionResponse =
            serde_json::from_str(unfence(self.text(META_MODEL, prompt)?.trim()))?;
        let term = decoded.term.clone();
        let understanding = decoded.understanding.clone();
        Ok(CardRevision::new(term, understanding, decoded.into_meta()))
    }

    /// Render one scene JSON payload into raw image bytes.
    pub fn image(&self, scene: &Value) -> Result<Vec<u8>> {
        let response = self.request(
            IMAGE_MODEL,
            &Request::text(
                serde_json::to_string_pretty(scene)?,
                Some(GenerationConfig::image()),
                Some(GenerationConfig::image_safety()),
            ),
        )?;
        if response.candidates.is_empty() {
            bail!("No candidates in image response: {}", diagnosis(&response));
        }
        let Some(content) = response.candidates[0].content.as_ref() else {
            bail!("No content in image response");
        };
        for part in &content.parts {
            if let Some(data) = part.inline_data.as_ref() {
                return decode(&data.data);
            }
        }
        bail!("No image data found in response");
    }

    /// Generate one PCM audio payload from the configured TTS model.
    pub fn speech(&self, prompt: &str, text: &str) -> Result<Vec<u8>> {
        let response = self.request(
            TTS_MODEL,
            &Request::text(
                String::from(prompt),
                Some(GenerationConfig::audio(voice())),
                None,
            ),
        )?;
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
        decode(&data.data)
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

    fn text(&self, model: &str, prompt: String) -> Result<String> {
        let response = self.request(model, &Request::text(prompt, None, None))?;
        let raw = response
            .candidates
            .iter()
            .flat_map(|candidate| candidate.content.as_ref().into_iter())
            .flat_map(|content| content.parts.iter())
            .filter_map(|part| part.text.as_ref())
            .cloned()
            .collect::<String>();
        if raw.trim().is_empty() {
            bail!("No text content in Gemini response");
        }
        Ok(raw)
    }
}

#[derive(Clone, Debug, Deserialize)]
struct IntakeResponse {
    target_lang: String,
    items: Vec<IntakeItem>,
}

#[derive(Clone, Debug, Deserialize)]
struct IntakeItem {
    term: String,
    #[serde(default)]
    understanding: Option<String>,
    #[serde(default)]
    senses: Vec<SenseItem>,
    #[serde(default)]
    selected: usize,
    ok: bool,
}

impl IntakeItem {
    fn candidate(self) -> Result<WordCandidate> {
        let term = nonempty(self.term.as_str(), "candidate");
        let mut senses = self
            .senses
            .into_iter()
            .map(SenseItem::sense)
            .collect::<Vec<_>>();
        if senses.is_empty()
            && let Some(understanding) = self.understanding
        {
            senses.push(Sense::plain(understanding));
        }
        let ok = self.ok && !senses.is_empty();
        if senses.is_empty() {
            senses.push(Sense::plain("модель не поняла слово"));
        }
        Ok(WordCandidate::with_senses(term, senses, self.selected, ok))
    }
}

#[derive(Clone, Debug, Deserialize)]
struct SenseItem {
    understanding: String,
    #[serde(default)]
    tag: Option<String>,
}

impl SenseItem {
    fn sense(self) -> Sense {
        Sense::new(self.understanding, self.tag)
    }
}

#[derive(Clone, Debug, Deserialize)]
struct SenseCorrectionResponse {
    #[serde(default)]
    senses: Vec<SenseItem>,
    #[serde(default)]
    message: Option<String>,
}

impl SenseCorrectionResponse {
    fn correction(self) -> SenseCorrection {
        let senses = self
            .senses
            .into_iter()
            .map(SenseItem::sense)
            .collect::<Vec<_>>();
        SenseCorrection::new(senses, self.message)
    }
}

#[derive(Clone, Debug, Deserialize)]
struct CardMetaResponse {
    pronunciation: String,
    transcription: String,
    meaning: String,
    importance: u8,
    source_sentence: String,
    source_highlight: String,
    source_hint: String,
    source_context: String,
    target_sentence: String,
}

impl CardMetaResponse {
    fn into_meta(self) -> CardMeta {
        CardMeta::new(
            self.pronunciation,
            self.transcription,
            self.meaning,
            self.importance,
            self.source_sentence,
            self.source_highlight,
            self.source_hint,
            self.source_context,
            self.target_sentence,
        )
    }
}

#[derive(Clone, Debug, Deserialize)]
struct CardCorrectionResponse {
    term: String,
    understanding: String,
    pronunciation: String,
    transcription: String,
    meaning: String,
    importance: u8,
    source_sentence: String,
    source_highlight: String,
    source_hint: String,
    source_context: String,
    target_sentence: String,
}

impl CardCorrectionResponse {
    fn into_meta(self) -> CardMeta {
        CardMeta::new(
            self.pronunciation,
            self.transcription,
            self.meaning,
            self.importance,
            self.source_sentence,
            self.source_highlight,
            self.source_hint,
            self.source_context,
            self.target_sentence,
        )
    }
}

fn voice() -> &'static str {
    let mut rng = rand::rng();
    let index = rng.random_range(0..VOICES.len());
    VOICES[index]
}

fn nonempty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        return String::from(fallback);
    }
    String::from(value)
}
