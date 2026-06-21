use std::env;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use rand::RngExt;
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::Value;

use crate::generation::{manga_template, render_scene_prompt};
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
    GenerationConfig, Request, Response, api_error, diagnosis, enforce, unfence, validate,
};

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

    /// Translate one sentence into the enforced manga scene JSON shape.
    pub fn scene(&self, language: &str, sentence: &str, target: &str) -> Result<Value> {
        let (scene, _) = self.scene_metered(language, sentence, target)?;
        Ok(scene)
    }

    /// Translate one sentence and return the request cost record.
    pub(crate) fn scene_metered(
        &self,
        language: &str,
        sentence: &str,
        target: &str,
    ) -> Result<(Value, CostRecord)> {
        let prompt = render_scene_prompt(language).replace("{sentence}", sentence);
        let (raw, cost) = self.text_metered(SCENE_MODEL, prompt)?;
        Ok((scene_from_raw(raw.as_str(), sentence, target)?, cost))
    }

    /// Translate one sentence and report usage before local scene validation.
    pub(crate) fn scene_observed<F>(
        &self,
        language: &str,
        sentence: &str,
        target: &str,
        mut observe: F,
    ) -> Result<Value>
    where
        F: FnMut(CostRecord),
    {
        let prompt = render_scene_prompt(language).replace("{sentence}", sentence);
        let raw = self.text_observed(SCENE_MODEL, prompt, &mut observe)?;
        scene_from_raw(raw.as_str(), sentence, target)
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
        let url = format!("{}/{TEXT_MODEL}:generateContent", base_url());
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
        F: FnMut(CostRecord),
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
        F: FnMut(CostRecord),
    {
        let metered = self.request_metered(
            IMAGE_MODEL,
            &Request::text(
                serde_json::to_string_pretty(scene)?,
                Some(GenerationConfig::image()),
                Some(GenerationConfig::image_safety()),
            ),
        )?;
        observe(metered.cost.clone());
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
        F: FnMut(CostRecord),
    {
        let metered = self.request_metered(
            TTS_MODEL,
            &Request::text(
                String::from(prompt),
                Some(GenerationConfig::audio(voice())),
                None,
            ),
        )?;
        observe(metered.cost.clone());
        speech_from_response(&metered.response, text)
    }

    fn request_metered(&self, model: &str, request: &Request) -> Result<MeteredResponse> {
        let url = format!("{}/{model}:generateContent", base_url());
        let body = serde_json::to_string(request)?;
        let response = self
            .transport
            .post(url.as_str(), self.key.as_str(), body.as_str())?;
        if !(200..300).contains(&response.status) {
            return Err(api_error(response.body.as_str()));
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
        F: FnMut(CostRecord),
    {
        let metered = self.request_metered(model, &Request::text(prompt, None, None))?;
        observe(metered.cost.clone());
        let raw = response_text(&metered.response);
        if raw.trim().is_empty() {
            bail!("No text content in Gemini response");
        }
        Ok(raw)
    }
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

fn scene_from_raw(raw: &str, sentence: &str, target: &str) -> Result<Value> {
    let cleaned = unfence(raw.trim());
    let panels = serde_json::from_str::<Value>(cleaned)?;
    let Some(items) = panels.as_array() else {
        bail!("Expected a JSON array of panels");
    };
    let mut scene = serde_json::from_str::<Value>(manga_template())?;
    scene["manga_panel"]["panels"] = Value::Array(items.clone());
    scene["manga_panel"]["meta"]["title"] = Value::String(sentence.chars().take(60).collect());
    scene["manga_panel"]["meta"]["description"] = Value::String(String::from(sentence));
    scene["manga_panel"]["meta"]["target_lang"] = Value::String(target.to_ascii_lowercase());
    enforce(&mut scene);
    validate(&scene)?;
    Ok(scene)
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
    }

    impl FakeTransport {
        fn new(responses: Vec<Result<TransportResponse>>) -> Self {
            Self {
                responses: Rc::new(RefCell::new(responses)),
            }
        }
    }

    impl Transport for FakeTransport {
        fn post(&self, _url: &str, _key: &str, _body: &str) -> Result<TransportResponse> {
            self.responses.borrow_mut().remove(0)
        }
    }

    fn body(value: serde_json::Value) -> Result<TransportResponse> {
        Ok(TransportResponse {
            status: 200,
            body: serde_json::to_string(&value)?,
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
                "totalTokenCount": 120
            }
        }))]);
        let client = GeminiClient::new("key", transport);
        let draft = CardDraft::new("wound", "noun sense", LanguagePair::new("en", "ru"));
        let (_revision, cost) = client
            .correct_card_metered(&draft, "make it a verb", &LanguagePair::new("en", "ru"))
            .expect("card correction must decode");
        assert_eq!(
            cost.cost().nanos(),
            330_000,
            "card correction must preserve Gemini usage cost for the regenerated meta"
        );
    }
}
