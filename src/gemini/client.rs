use std::env;
use std::time::Duration;

use anyhow::Context;
use anyhow::{Result, anyhow, bail};
use rand::RngExt;
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::Value;

use crate::generation::layout::{LayoutRegistry, feature_prompt_data, render_feature_prompt};
use crate::generation::prompts::{layout_scene_prompt, layout_selector_prompt};
use crate::languages::catalog;
use crate::session::{
    CardDraft, CardMeta, CardRevision, CostRecord, LanguagePair, LearningGuess, RawInputBatch,
    Sense, SenseCorrection, Understood, WordCandidate,
};

use super::codec::decode;
use super::cost::priced;
use super::prompts::{
    render_bulk_prompt, render_card_meta_prompt, render_card_prompt, render_intake_prompt,
};
use super::protocol::{
    GeminiApiError, GenerationConfig, Request, Response, api_error, diagnosis, unfence,
};
use super::scene::compose;

const BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const HTTP_TIMEOUT: Duration = Duration::from_secs(300);

/// Return the Gemini API base URL, honoring a non-empty `KAMISHIBAI_GEMINI_URL`
/// override (offline tests point it at a local listener; proxies can too).
fn base_url() -> String {
    env::var("KAMISHIBAI_GEMINI_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| String::from(BASE_URL))
}
const TEXT_MODEL: &str = "gemini-3.6-flash";
const META_MODEL: &str = TEXT_MODEL;
const SCENE_MODEL: &str = TEXT_MODEL;
const IMAGE_MODEL: &str = "gemini-3.1-flash-image";
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
#[derive(Clone, Debug)]
pub struct HttpTransport {
    client: Client,
}

impl HttpTransport {
    /// Create one HTTP transport.
    pub fn new() -> Self {
        Self::with_timeout(HTTP_TIMEOUT)
    }

    fn with_timeout(timeout: Duration) -> Self {
        Self {
            client: Client::builder()
                .timeout(timeout)
                .build()
                .expect("invariant: reqwest client must build with a fixed timeout"),
        }
    }
}

impl Default for HttpTransport {
    /// Create the default HTTP transport with the production request timeout.
    fn default() -> Self {
        Self::new()
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
                    "no Gemini API key found — save one with 'kamishibai config --key', \
                     set GEMINI_API_KEY, or paste one on the TUI Welcome"
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

    /// Translate one term and sentence through the production registry scene pipeline.
    pub fn scene(&self, language: &str, term: &str, sentence: &str, target: &str) -> Result<Value> {
        self.scene_observed(language, term, sentence, target, 0, |_| Ok(()))
    }

    /// Compose one registry-selected scene and report every structured request cost.
    pub(crate) fn scene_observed<F>(
        &self,
        language: &str,
        term: &str,
        sentence: &str,
        target: &str,
        attempt: u8,
        mut observe: F,
    ) -> Result<Value>
    where
        F: FnMut(CostRecord) -> Result<()>,
    {
        let registry = LayoutRegistry::embedded()?;
        let feature_data = feature_prompt_data(language, term, sentence)?;
        let feature_prompt = render_feature_prompt(&feature_data)?;
        let feature_schema = registry.feature_schema()?;
        let feature_raw = self
            .structured_text_observed(SCENE_MODEL, feature_prompt, &feature_schema, &mut observe)
            .context("scene feature extraction request failed")?;
        let features = registry.decode_features(unfence(feature_raw.trim()))?;
        let eligible = registry.eligible(&features)?;
        let selector = render_layout_selector(
            language,
            term,
            sentence,
            features.json(),
            &eligible.selector_cards()?,
        )?;
        let selector_schema = eligible.selector_schema()?;
        let selector_raw = self
            .structured_text_observed(SCENE_MODEL, selector, &selector_schema, &mut observe)
            .context("scene layout selection request failed")?;
        let ranking = eligible.decode_ranking(unfence(selector_raw.trim()))?;
        let selection = ranking.select(term, attempt)?;
        let composer_card = selection.composer_card()?;
        let composer =
            render_layout_scene(language, term, sentence, selection.json(), &composer_card)?;
        let composer_raw = self
            .json_text_observed(SCENE_MODEL, composer, &mut observe)
            .context("scene composition request failed")?;
        compose(composer_raw.as_str(), sentence, target, &selection)
    }

    /// Send one free-form prompt to a text model and return the raw textual
    /// response. Used by eval/dev tooling that swaps prompts without going
    /// through the typed `understand` / `generate_card_meta` paths.
    pub fn complete(&self, model: &str, prompt: String) -> Result<String> {
        self.text(model, prompt)
    }

    /// Send one prompt with a JSON response schema and return the raw JSON text.
    pub fn complete_json(&self, model: &str, prompt: String, schema: &Value) -> Result<String> {
        if !schema.is_object() {
            bail!("Gemini response schema must be a JSON object");
        }
        let metered = self.request_metered(
            model,
            &Request::text(prompt, Some(GenerationConfig::json(schema.clone())?), None),
        )?;
        let raw = response_text(&metered.response);
        if raw.trim().is_empty() {
            bail!("No text content in Gemini response");
        }
        Ok(raw)
    }

    /// Send one prompt in JSON mode without constraining its nested schema.
    pub fn complete_json_mode(&self, model: &str, prompt: String) -> Result<String> {
        let metered = self.request_metered(
            model,
            &Request::text(prompt, Some(GenerationConfig::json_mode()), None),
        )?;
        let raw = response_text(&metered.response);
        if raw.trim().is_empty() {
            bail!("No text content in Gemini response");
        }
        Ok(raw)
    }

    /// Probe the API with one tiny request to confirm the key is accepted.
    ///
    /// Any `2xx` counts as a valid key — the body is not parsed, so a thin or
    /// unusual completion still passes. A non-`2xx` is mapped through
    /// `api_error`, so the caller can tell a rejected key (`rejects_key`) from a
    /// transport or quota failure and message accordingly.
    pub fn validate_key(&self) -> Result<()> {
        let url = format!("{}/{TEXT_MODEL}:generateContent", base_url());
        let body = serde_json::to_string(&Request::text(String::from("ping"), None, None))?;
        let response = self
            .transport
            .post(url.as_str(), self.key.as_str(), body.as_str())?;
        if (200..300).contains(&response.status) {
            return Ok(());
        }
        Err(api_error(response.status, response.body.as_str()))
    }

    /// Resolve raw user input into reviewed rows using the Flash text model.
    pub fn understand(&self, raw: &RawInputBatch, my: &str) -> Result<Understood> {
        let catalog = catalog();
        let prompt = render_intake_prompt(raw.text(), my, &catalog)?;
        let decoded: IntakeResponse =
            serde_json::from_str(unfence(self.text(TEXT_MODEL, prompt)?.trim()))?;
        Ok(Understood::new(
            LearningGuess::new(decoded.target_lang, true),
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
        let (meta, _) = self.generate_card_meta_metered(term, understanding, pair)?;
        Ok(meta)
    }

    /// Build rich card meta and return the request cost record.
    pub(crate) fn generate_card_meta_metered(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
    ) -> Result<(CardMeta, CostRecord)> {
        let catalog = catalog();
        let prompt = render_card_meta_prompt(term, understanding, pair, &catalog)?;
        let (raw, cost) = self.text_metered(META_MODEL, prompt)?;
        Ok((card_meta_from_raw(raw.as_str())?, cost))
    }

    /// Build rich card meta and report usage before local JSON decoding.
    pub(crate) fn generate_card_meta_observed<F>(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        mut observe: F,
    ) -> Result<CardMeta>
    where
        F: FnMut(CostRecord) -> Result<()>,
    {
        let catalog = catalog();
        let prompt = render_card_meta_prompt(term, understanding, pair, &catalog)?;
        let raw = self.text_observed(META_MODEL, prompt, &mut observe)?;
        card_meta_from_raw(raw.as_str())
    }

    /// Recompose one card draft after a per-card refinement.
    pub fn correct_card(
        &self,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<CardRevision> {
        let (revision, _) = self.correct_card_metered(draft, comment, pair)?;
        Ok(revision)
    }

    /// Recompose one card draft and return the request cost record.
    pub(crate) fn correct_card_metered(
        &self,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<(CardRevision, CostRecord)> {
        let catalog = catalog();
        let prompt = render_card_prompt(draft, comment, pair, &catalog)?;
        let (raw, cost) = self.text_metered(META_MODEL, prompt)?;
        let decoded: CardCorrectionResponse = serde_json::from_str(unfence(raw.trim()))?;
        let term = decoded.term.clone();
        let understanding = decoded.understanding.clone();
        Ok((
            CardRevision::new(term, understanding, decoded.into_meta()),
            cost,
        ))
    }

    /// Recompose one card draft and report usage before local JSON decoding.
    pub(crate) fn correct_card_observed<F>(
        &self,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
        mut observe: F,
    ) -> Result<CardRevision>
    where
        F: FnMut(CostRecord) -> Result<()>,
    {
        let catalog = catalog();
        let prompt = render_card_prompt(draft, comment, pair, &catalog)?;
        let raw = self.text_observed(META_MODEL, prompt, &mut observe)?;
        let decoded: CardCorrectionResponse = serde_json::from_str(unfence(raw.trim()))?;
        let term = decoded.term.clone();
        let understanding = decoded.understanding.clone();
        Ok(CardRevision::new(term, understanding, decoded.into_meta()))
    }

    /// Render one scene JSON payload into raw image bytes.
    pub fn image(&self, scene: &Value) -> Result<Vec<u8>> {
        let (bytes, _) = self.image_metered(scene)?;
        Ok(bytes)
    }

    /// Render one scene JSON payload and return the request cost record.
    pub(crate) fn image_metered(&self, scene: &Value) -> Result<(Vec<u8>, CostRecord)> {
        let metered = self.request_metered(
            IMAGE_MODEL,
            &Request::text(
                serde_json::to_string_pretty(scene)?,
                Some(GenerationConfig::image()),
                Some(GenerationConfig::image_safety()),
            ),
        )?;
        Ok((image_from_response(&metered.response)?, metered.cost))
    }

    /// Render one scene JSON payload and report usage before local image decoding.
    pub(crate) fn image_observed<F>(&self, scene: &Value, mut observe: F) -> Result<Vec<u8>>
    where
        F: FnMut(CostRecord) -> Result<()>,
    {
        let metered = self.request_metered(
            IMAGE_MODEL,
            &Request::text(
                serde_json::to_string_pretty(scene)?,
                Some(GenerationConfig::image()),
                Some(GenerationConfig::image_safety()),
            ),
        )?;
        observe(metered.cost.clone())?;
        image_from_response(&metered.response)
    }

    /// Generate one PCM audio payload from the configured TTS model.
    pub fn speech(&self, prompt: &str, text: &str) -> Result<Vec<u8>> {
        let (data, _) = self.speech_metered(prompt, text)?;
        Ok(data)
    }

    /// Generate one PCM audio payload and return the request cost record.
    pub(crate) fn speech_metered(&self, prompt: &str, text: &str) -> Result<(Vec<u8>, CostRecord)> {
        let metered = self.request_metered(
            TTS_MODEL,
            &Request::text(
                String::from(prompt),
                Some(GenerationConfig::audio(voice())),
                None,
            ),
        )?;
        Ok((speech_from_response(&metered.response, text)?, metered.cost))
    }

    /// Generate one PCM audio payload and report usage before local audio decoding.
    pub(crate) fn speech_observed<F>(
        &self,
        prompt: &str,
        text: &str,
        mut observe: F,
    ) -> Result<Vec<u8>>
    where
        F: FnMut(CostRecord) -> Result<()>,
    {
        let metered = self.request_metered(
            TTS_MODEL,
            &Request::text(
                String::from(prompt),
                Some(GenerationConfig::audio(voice())),
                None,
            ),
        )?;
        observe(metered.cost.clone())?;
        speech_from_response(&metered.response, text)
    }

    fn request_metered(&self, model: &str, request: &Request) -> Result<MeteredResponse> {
        let url = format!("{}/{model}:generateContent", base_url());
        let body = serde_json::to_string(request)?;
        let response = self
            .transport
            .post(url.as_str(), self.key.as_str(), body.as_str())?;
        if !(200..300).contains(&response.status) {
            return Err(api_error(response.status, response.body.as_str()));
        }
        let parsed: Response = serde_json::from_str(&response.body)?;
        let cost = priced(model, parsed.usage_metadata.as_ref());
        Ok(MeteredResponse {
            response: parsed,
            cost,
        })
    }

    fn text(&self, model: &str, prompt: String) -> Result<String> {
        Ok(self.text_metered(model, prompt)?.0)
    }

    fn text_metered(&self, model: &str, prompt: String) -> Result<(String, CostRecord)> {
        let metered = self.request_metered(model, &Request::text(prompt, None, None))?;
        let raw = response_text(&metered.response);
        if raw.trim().is_empty() {
            bail!("No text content in Gemini response");
        }
        Ok((raw, metered.cost))
    }

    fn text_observed<F>(&self, model: &str, prompt: String, observe: &mut F) -> Result<String>
    where
        F: FnMut(CostRecord) -> Result<()>,
    {
        let metered = self.request_metered(model, &Request::text(prompt, None, None))?;
        observe(metered.cost.clone())?;
        let raw = response_text(&metered.response);
        if raw.trim().is_empty() {
            bail!("No text content in Gemini response");
        }
        Ok(raw)
    }

    fn structured_text_observed<F>(
        &self,
        model: &str,
        prompt: String,
        schema: &Value,
        observe: &mut F,
    ) -> Result<String>
    where
        F: FnMut(CostRecord) -> Result<()>,
    {
        let config = GenerationConfig::json(schema.clone())?;
        let request = Request::text(prompt.clone(), Some(config), None);
        let metered = match self.request_metered(model, &request) {
            Ok(metered) => metered,
            Err(error) if schema_rejected(&error) => self.request_metered(
                model,
                &Request::text(prompt, Some(GenerationConfig::json_mode()), None),
            )?,
            Err(error) => return Err(error),
        };
        observe(metered.cost.clone())?;
        let raw = response_text(&metered.response);
        if raw.trim().is_empty() {
            bail!("No text content in Gemini response");
        }
        Ok(raw)
    }

    fn json_text_observed<F>(&self, model: &str, prompt: String, observe: &mut F) -> Result<String>
    where
        F: FnMut(CostRecord) -> Result<()>,
    {
        let metered = self.request_metered(
            model,
            &Request::text(prompt, Some(GenerationConfig::json_mode()), None),
        )?;
        observe(metered.cost.clone())?;
        let raw = response_text(&metered.response);
        if raw.trim().is_empty() {
            bail!("No text content in Gemini response");
        }
        Ok(raw)
    }
}

fn schema_rejected(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<GeminiApiError>()
        .is_some_and(GeminiApiError::rejects_schema)
}

fn render_layout_selector(
    language: &str,
    term: &str,
    sentence: &str,
    features: &Value,
    registry: &Value,
) -> Result<String> {
    Ok(layout_selector_prompt()
        .replace("{language}", language)
        .replace("{term}", term)
        .replace("{sentence}", sentence)
        .replace("{scene_features}", &serde_json::to_string_pretty(features)?)
        .replace(
            "{layout_registry}",
            &serde_json::to_string_pretty(registry)?,
        ))
}

fn render_layout_scene(
    language: &str,
    term: &str,
    sentence: &str,
    selection: &Value,
    layout: &Value,
) -> Result<String> {
    Ok(layout_scene_prompt()
        .replace("{language}", language)
        .replace("{term}", term)
        .replace("{sentence}", sentence)
        .replace(
            "{layout_selection}",
            &serde_json::to_string_pretty(selection)?,
        )
        .replace("{selected_layout}", &serde_json::to_string_pretty(layout)?))
}

struct MeteredResponse {
    response: Response,
    cost: CostRecord,
}

fn response_text(response: &Response) -> String {
    response
        .candidates
        .iter()
        .flat_map(|candidate| candidate.content.as_ref().into_iter())
        .flat_map(|content| content.parts.iter())
        .filter_map(|part| part.text.as_ref())
        .cloned()
        .collect::<String>()
}

fn card_meta_from_raw(raw: &str) -> Result<CardMeta> {
    let decoded: CardMetaResponse = serde_json::from_str(unfence(raw.trim()))?;
    Ok(decoded.into_meta())
}

fn image_from_response(response: &Response) -> Result<Vec<u8>> {
    if response.candidates.is_empty() {
        bail!("No candidates in image response: {}", diagnosis(response));
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

fn speech_from_response(response: &Response, text: &str) -> Result<Vec<u8>> {
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io::Read;
    use std::net::TcpListener;
    use std::rc::Rc;
    use std::thread;
    use std::time::Duration;

    use serde_json::json;

    use super::*;

    #[derive(Clone, Debug)]
    struct FakeTransport {
        responses: Rc<RefCell<Vec<Result<TransportResponse>>>>,
        requests: Rc<RefCell<Vec<String>>>,
    }

    impl FakeTransport {
        fn new(responses: Vec<Result<TransportResponse>>) -> Self {
            Self {
                responses: Rc::new(RefCell::new(responses)),
                requests: Rc::new(RefCell::new(Vec::new())),
            }
        }
    }

    impl Transport for FakeTransport {
        fn post(&self, _url: &str, _key: &str, body: &str) -> Result<TransportResponse> {
            self.requests.borrow_mut().push(String::from(body));
            self.responses.borrow_mut().remove(0)
        }
    }

    fn body(value: serde_json::Value) -> Result<TransportResponse> {
        Ok(TransportResponse {
            status: 200,
            body: serde_json::to_string(&value)?,
        })
    }

    fn text_body(value: &Value) -> Result<TransportResponse> {
        body(json!({
            "candidates": [{"content": {"parts": [{"text": serde_json::to_string(value)?}]}}],
            "usageMetadata": {
                "promptTokenCount": 100,
                "candidatesTokenCount": 20,
                "thoughtsTokenCount": 30,
                "totalTokenCount": 150
            }
        }))
    }

    fn coverage_audit(panel_count: usize) -> Value {
        Value::Array(
            (1..=4)
                .map(|count| {
                    let verdict = match count.cmp(&panel_count) {
                        std::cmp::Ordering::Less => "insufficient",
                        std::cmp::Ordering::Equal => "selected",
                        std::cmp::Ordering::Greater => "redundant_or_unsupported",
                    };
                    json!({
                        "panel_count": count,
                        "added_view": format!("candidate view {count}"),
                        "source_support": format!("source support audit {count}"),
                        "verdict": verdict,
                        "reason": format!("coverage decision {count}")
                    })
                })
                .collect(),
        )
    }

    fn camera_arc() -> Value {
        json!({
            "strategy": "single_view",
            "progression": "one stable wide objective view",
            "motivation": "the uninterrupted state is strongest in one continuous setup",
            "continuity": {
                "axis_mode": "not_applicable",
                "axis": "",
                "screen_direction": "stationary",
                "eyeline_policy": "not_applicable"
            }
        })
    }

    fn planned_shot(id: &str, beat: usize, anchor: &str, support: &str) -> Value {
        json!({
            "id": id,
            "semantic_beat_index": beat,
            "role": "action",
            "visible_anchor": anchor,
            "source_support": support,
            "shot_scale": "wide",
            "viewpoint": "objective",
            "viewpoint_anchor": "",
            "framing": "single",
            "angle": "low",
            "depth_plan": "layered",
            "camera_motivation": "the complete mechanism makes continued operation visible",
            "information_gain": format!("supported machine state {id}"),
            "transition_trigger": if id == "s1" { "scene_open" } else { "new_action" }
        })
    }

    fn semantic_scene() -> Value {
        json!({
            "semantic_spine": {
                "literal_event": "A system runs without interruption",
                "semantic_focus": "reliability",
                "emotional_relation": "confidence",
                "intensity": 2,
                "visual_relation": "balance",
                "memory_hook": "one stable machine under a steady indicator light",
                "metaphor": {"mode": "none", "mapping": "", "literal_anchor": "stable machine"}
            },
            "page_design": {
                "rhythm": "single_tableau",
                "special_device": {
                    "kind": "none",
                    "reason": "one continuous tableau communicates the whole sentence",
                    "source_panel": "",
                    "target_panel": "",
                    "subject_id": ""
                },
                "eye_flow_summary": "the machine silhouette leads toward its steady light"
            },
            "panels": [{
                "shot_id": "s1",
                "narrative_role": "peak",
                "semantic_job": "show the system operating steadily",
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
                    "description": "A complete machine runs steadily in one continuous room",
                    "subjects": [{
                        "id": "machine",
                        "figure": "a compact industrial machine",
                        "pose": "fully visible and operating without vibration",
                        "expression": "mechanically steady",
                        "blocking": "centered with open space around every edge"
                    }],
                    "environment": {
                        "setting": "quiet equipment room",
                        "foreground": ["clean floor rails"],
                        "midground": ["the complete machine"],
                        "background": ["blank equipment cabinets"]
                    },
                    "camera": {
                        "shot_scale": "wide",
                        "viewpoint": "objective",
                        "viewpoint_subject_id": "",
                        "framing": "single",
                        "angle": "low",
                        "focus": "the stable machine and indicator light",
                        "depth_plan": "layered",
                        "eye_flow_exit": "toward the steady light"
                    },
                    "motion_treatment": "pose_only",
                    "lighting": "even maintenance lighting",
                    "mood": "assured"
                }
            }]
        })
    }

    #[test]
    fn http_transport_honors_request_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener must bind");
        let url = format!(
            "http://{}/slow",
            listener
                .local_addr()
                .expect("test listener must have address")
        );
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("test listener must accept");
            let mut buffer = [0; 1024];
            let _ = stream.read(&mut buffer);
            thread::sleep(Duration::from_millis(250));
        });
        let error = HttpTransport::with_timeout(Duration::from_millis(25))
            .post(url.as_str(), "key", "{}")
            .expect_err("slow server must time out");
        server.join().expect("test server must finish");
        assert!(
            error.chain().any(|cause| cause
                .downcast_ref::<reqwest::Error>()
                .is_some_and(reqwest::Error::is_timeout)),
            "HTTP transport ignored the configured request timeout"
        );
    }

    #[test]
    fn card_correction_returns_request_cost() {
        let transport = FakeTransport::new(vec![body(json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "text": "{\"term\":\"wound\",\"understanding\":\"verb sense\",\"pronunciation\":\"waʊnd\",\"transcription\":\"aɪ waʊnd ðə klɒk\",\"meaning\":\"завести\",\"importance\":6,\"source_sentence\":\"Я завел часы.\",\"source_highlight\":\"завел\",\"source_hint\":\"Поворачивал что-то круглое.\",\"source_context\":\"Глагол про часы.\",\"target_sentence\":\"I wound the clock.\"}"
                    }]
                }
            }],
            "usageMetadata": {
                "promptTokenCount": 100,
                "candidatesTokenCount": 20,
                "thoughtsTokenCount": 30,
                "totalTokenCount": 150
            }
        }))]);
        let client = GeminiClient::new("key", transport);
        let draft = CardDraft::new("wound", "noun sense", LanguagePair::new("en", "ru"));
        let (_revision, cost) = client
            .correct_card_metered(&draft, "make it a verb", &LanguagePair::new("en", "ru"))
            .expect("card correction must decode");
        assert_eq!(
            cost.cost().nanos(),
            525_000,
            "card correction must preserve Gemini usage cost for the regenerated meta"
        );
    }

    #[test]
    fn invalid_card_correction_json_still_reports_request_cost() {
        let transport = FakeTransport::new(vec![body(json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "{not valid correction json"}]
                }
            }],
            "usageMetadata": {
                "promptTokenCount": 100,
                "candidatesTokenCount": 20,
                "thoughtsTokenCount": 30,
                "totalTokenCount": 150
            }
        }))]);
        let client = GeminiClient::new("key", transport);
        let draft = CardDraft::new("wound", "noun sense", LanguagePair::new("en", "ru"));
        let mut costs = Vec::new();
        let result = client.correct_card_observed(
            &draft,
            "make it a verb",
            &LanguagePair::new("en", "ru"),
            |cost| {
                costs.push(cost);
                Ok(())
            },
        );
        assert_eq!(
            (
                result.is_err(),
                costs.first().map(CostRecord::requests),
                costs.first().map(|cost| cost.cost().nanos()),
            ),
            (true, Some(1), Some(525_000)),
            "invalid correction JSON discarded the billed Gemini request cost"
        );
    }

    #[test]
    fn typed_schema_fallback_observes_only_the_successful_json_request() {
        let transport = FakeTransport::new(vec![
            Ok(TransportResponse {
                status: 400,
                body: json!({
                    "error": {
                        "status": "INVALID_ARGUMENT",
                        "message": "response schema is too complex"
                    }
                })
                .to_string(),
            }),
            text_body(&json!({"ok": true})),
        ]);
        let requests = transport.requests.clone();
        let client = GeminiClient::new("key", transport);
        let mut costs = Vec::new();
        let raw = client
            .structured_text_observed(
                SCENE_MODEL,
                String::from("same prompt"),
                &json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {"ok": {"type": "boolean"}},
                    "required": ["ok"]
                }),
                &mut |cost| {
                    costs.push(cost);
                    Ok(())
                },
            )
            .expect("schema rejection must recover through JSON mode");
        let bodies = requests
            .borrow()
            .iter()
            .map(|body| serde_json::from_str::<Value>(body))
            .collect::<Result<Vec<_>, _>>()
            .expect("fallback requests must stay valid JSON");
        assert_eq!(
            (
                raw,
                costs.len(),
                costs.first().map(CostRecord::requests),
                costs.first().map(|cost| cost.cost().nanos()),
                bodies.len(),
                bodies[0]["contents"][0]["parts"][0]["text"].as_str(),
                bodies[1]["contents"][0]["parts"][0]["text"].as_str(),
                bodies[0]
                    .pointer("/generationConfig/responseFormat/text/schema")
                    .is_some(),
                bodies[1]
                    .pointer("/generationConfig/responseMimeType")
                    .and_then(Value::as_str),
            ),
            (
                String::from("{\"ok\":true}"),
                1,
                Some(1),
                Some(525_000),
                2,
                Some("same prompt"),
                Some("same prompt"),
                true,
                Some("application/json"),
            ),
            "schema fallback changed the prompt, call count, or successful cost observation"
        );
    }

    #[test]
    fn registry_scene_uses_typed_analysis_and_schema_free_composition() {
        let features = json!({
            "semantic_beat_count": 1,
            "semantic_relation": "single_moment",
            "coverage_audit": coverage_audit(1),
            "panel_count": 1,
            "panel_relation": "single_moment",
            "panel_emphasis": "equal",
            "decomposition_mode": "single_tableau",
            "motion_vector": "still",
            "intensity": "quiet",
            "spatial_relation": "same_space",
            "transition_type": "none",
            "reading_direction": "left_to_right_top_to_bottom",
            "literal_anchor": "one stable machine",
            "camera_arc": camera_arc(),
            "shots": [planned_shot("s1", 1, "one stable machine in its complete room", "the system has high reliability")],
            "selection_logic": "one indivisible state carries the sentence"
        });
        let ranking = json!({
            "ranked_candidates": [{
                "template_id": "splash-1-v1",
                "adaptation": "exact",
                "reason": "one indivisible quiet state needs one continuous tableau"
            }]
        });
        let transport = FakeTransport::new(vec![
            text_body(&features),
            text_body(&ranking),
            text_body(&semantic_scene()),
        ]);
        let requests = transport.requests.clone();
        let client = GeminiClient::new("key", transport);
        let mut costs = Vec::new();
        let scene = client
            .scene_observed(
                "English",
                "reliability",
                "The reliability of this system is very high",
                "en",
                0,
                |cost| {
                    costs.push(cost);
                    Ok(())
                },
            )
            .expect("registry scene pipeline must compose one valid splash");
        let bodies = requests
            .borrow()
            .iter()
            .map(|body| serde_json::from_str::<Value>(body))
            .collect::<Result<Vec<_>, _>>()
            .expect("recorded requests must stay valid JSON");
        let prompts = bodies
            .iter()
            .map(|body| {
                body.pointer("/contents/0/parts/0/text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        let registry = LayoutRegistry::embedded().expect("embedded registry must decode");
        let feature_schema = registry
            .feature_schema()
            .expect("feature response schema must build");
        let decoded = registry
            .decode_features(
                serde_json::to_string(&features)
                    .expect("feature fixture must encode")
                    .as_str(),
            )
            .expect("feature fixture must decode through production validation");
        let eligible = registry
            .eligible(&decoded)
            .expect("feature fixture must have eligible layouts");
        let selector_schema = eligible
            .selector_schema()
            .expect("selector response schema must build");
        assert_eq!(
            (
                costs.len(),
                bodies.len(),
                (
                    bodies[..2].iter().all(|body| {
                        body.pointer("/generationConfig/responseFormat/text/mimeType")
                            .and_then(Value::as_str)
                            == Some("APPLICATION_JSON")
                    }),
                    bodies[..2].iter().all(|body| {
                        body.pointer("/generationConfig/responseMimeType").is_none()
                    }),
                    bodies[0].pointer("/generationConfig/responseFormat/text/schema")
                        == Some(&feature_schema),
                    bodies[1].pointer("/generationConfig/responseFormat/text/schema")
                        == Some(&selector_schema),
                    bodies[2]
                        .pointer("/generationConfig/responseMimeType")
                        .and_then(Value::as_str),
                    bodies[2]
                        .pointer("/generationConfig/responseFormat")
                        .is_none(),
                    bodies[2]
                        .pointer("/generationConfig/maxOutputTokens")
                        .is_none(),
                ),
                prompts[0].contains("reliability")
                    && !prompts[0].contains("splash-1-v1")
                    && !prompts[0].contains("LAYOUT REGISTRY"),
                prompts[1].contains("splash-1-v1"),
                prompts[2].contains("\"chosen_template_id\": \"splash-1-v1\""),
                !prompts[2].contains("\"bounds\"") && !prompts[2].contains("\"polygon\""),
                scene
                    .pointer("/manga_panel/meta/layout_selection/chosen_template_id")
                    .and_then(Value::as_str),
                scene
                    .pointer("/manga_panel/page_design/layout/template_id")
                    .and_then(Value::as_str),
            ),
            (
                3,
                3,
                (true, true, true, true, Some("application/json"), true, true,),
                true,
                true,
                true,
                true,
                Some("splash-1-v1"),
                Some("splash-1-v1"),
            ),
            "registry scene pipeline lost its typed analysis, schema-free composition, or selection provenance"
        );
    }

    #[test]
    fn registry_scene_preserves_costs_from_every_completed_stage_on_failure() {
        let valid = json!({
            "semantic_beat_count": 1,
            "semantic_relation": "single_moment",
            "coverage_audit": coverage_audit(1),
            "panel_count": 1,
            "panel_relation": "single_moment",
            "panel_emphasis": "equal",
            "decomposition_mode": "single_tableau",
            "motion_vector": "still",
            "intensity": "quiet",
            "spatial_relation": "same_space",
            "transition_type": "none",
            "reading_direction": "left_to_right_top_to_bottom",
            "literal_anchor": "one stable machine",
            "camera_arc": camera_arc(),
            "shots": [planned_shot("s1", 1, "one stable machine", "the system has high reliability")],
            "selection_logic": "one indivisible state carries the sentence"
        });
        let invalid = json!({
            "semantic_beat_count": 2,
            "semantic_relation": "single_moment",
            "coverage_audit": coverage_audit(2),
            "panel_count": 2,
            "panel_relation": "single_moment",
            "panel_emphasis": "equal",
            "decomposition_mode": "one_to_one",
            "motion_vector": "still",
            "intensity": "quiet",
            "spatial_relation": "same_space",
            "transition_type": "none",
            "reading_direction": "left_to_right_top_to_bottom",
            "literal_anchor": "contradictory beats",
            "camera_arc": {
                "strategy": "push_in",
                "progression": "wide to close",
                "motivation": "the second unsupported beat would intensify",
                "continuity": {"axis_mode": "not_applicable", "axis": "", "screen_direction": "stationary", "eyeline_policy": "not_applicable"}
            },
            "shots": [
                planned_shot("s1", 1, "first", "first fact"),
                {
                    "id": "s2",
                    "semantic_beat_index": 2,
                    "role": "action",
                    "visible_anchor": "second",
                    "source_support": "second fact",
                    "shot_scale": "close",
                    "viewpoint": "objective",
                    "viewpoint_anchor": "",
                    "framing": "single",
                    "angle": "low",
                    "depth_plan": "layered",
                    "camera_motivation": "the unsupported beat would intensify",
                    "information_gain": "unsupported second state",
                    "transition_trigger": "new_action"
                }
            ],
            "selection_logic": "invalid on purpose"
        });
        let ranking = json!({
            "ranked_candidates": [{
                "template_id": "splash-1-v1",
                "adaptation": "exact",
                "reason": "one indivisible quiet state"
            }]
        });
        let cases = vec![
            vec![text_body(&invalid)],
            vec![
                text_body(&valid),
                text_body(&json!({"ranked_candidates": []})),
            ],
            vec![
                text_body(&valid),
                text_body(&ranking),
                text_body(&json!({})),
            ],
        ];
        let observed = cases
            .into_iter()
            .enumerate()
            .map(|(index, responses)| {
                let transport = FakeTransport::new(responses);
                let requests = transport.requests.clone();
                let client = GeminiClient::new("key", transport);
                let mut costs = Vec::new();
                let failed = client
                    .scene_observed(
                        "English",
                        "reliability",
                        "The reliability of this system is very high",
                        "en",
                        0,
                        |cost| {
                            costs.push(cost);
                            Ok(())
                        },
                    )
                    .is_err();
                (index + 1, failed, costs.len(), requests.borrow().len())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            observed,
            [(1, true, 1, 1), (2, true, 2, 2), (3, true, 3, 3)],
            "registry scene failures discarded completed-stage costs or called a later stage"
        );
    }

    #[test]
    fn observer_failure_stops_the_scene_pipeline_before_the_next_request() {
        let transport = FakeTransport::new(vec![
            text_body(&json!({})),
            text_body(&json!({})),
            text_body(&json!({})),
        ]);
        let requests = transport.requests.clone();
        let client = GeminiClient::new("key", transport);
        let result = client.scene_observed(
            "English",
            "reliability",
            "The reliability of this system is very high",
            "en",
            0,
            |_| Err(anyhow::anyhow!("cost persistence failed")),
        );
        assert_eq!(
            (result.is_err(), requests.borrow().len()),
            (true, 1),
            "a failed usage write allowed later scene stages to spend more"
        );
    }
}
