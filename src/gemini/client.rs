use std::env;
use std::fmt;
use std::time::Duration;

use anyhow::Context;
use anyhow::{Result, anyhow, bail};
use rand::RngExt;
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use crate::application::LearningTarget;
use crate::generation::layout::{LayoutRegistry, feature_prompt_data, render_feature_prompt};
use crate::generation::manga::{
    FidelityCheck, FidelityReview, LiteralZoomCheck, LiteralZoomReview, RecallCard, RecallReview,
    TextCheck, TextReview,
};
use crate::generation::prompts::layout_scene_prompt;
use crate::languages::catalog;
use crate::session::{
    AxisSet, CardDraft, CardMeta, CardRevision, CostRecord, LanguagePair, LearningGuess,
    RawInputBatch, Register, Sense, SenseCorrection, SentenceAxis, SentenceKind,
    SentenceLabelSelection, SentenceLabels, SentenceLevel, Understood, WordCandidate,
};

use super::codec::{decode, encode};
use super::cost::priced;
use super::prompts::{
    render_bulk_prompt, render_card_meta_prompt, render_card_prompt, render_intake_prompt,
};
use super::protocol::{
    GeminiApiError, GenerationConfig, RejectedReply, Request, Response, ThinkingLevel, api_error,
    diagnosis, unfence,
};
use super::scene::compose;

const BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const HTTP_TIMEOUT: Duration = Duration::from_secs(300);
const CREDENTIAL_TIMEOUT: Duration = Duration::from_secs(20);

/// Return the Gemini API base URL, honoring a non-empty `KAMISHIBAI_GEMINI_URL`
/// override (offline tests point it at a local listener; proxies can too).
fn base_url() -> String {
    env::var("KAMISHIBAI_GEMINI_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| String::from(BASE_URL))
}

fn model_catalog_url() -> String {
    let base = base_url();
    catalog_url(base.as_str())
}

fn catalog_url(base: &str) -> String {
    let separator = if base.contains('?') { '&' } else { '?' };
    format!("{base}{separator}pageSize=1000")
}

const TEXT_MODEL: &str = "gemini-3.6-flash";
const META_MODEL: &str = TEXT_MODEL;
const FEATURE_MODEL: &str = "gemini-3.5-flash-lite";
const SCENE_MODEL: &str = TEXT_MODEL;
const IMAGE_MODEL: &str = "gemini-3.1-flash-image";
const RECALL_MODEL: &str = "gemini-3.5-flash-lite";
const FIDELITY_MODEL: &str = TEXT_MODEL;
const LITERAL_ZOOM_MODEL: &str = TEXT_MODEL;
const TEXT_JUDGE_MODEL: &str = "gemini-3.5-flash-lite";
const TTS_MODEL: &str = "gemini-3.1-flash-tts-preview";
const FEATURE_MAX_OUTPUT_TOKENS: u32 = 4_096;
const COMPOSER_MAX_OUTPUT_TOKENS: u32 = 8_192;
const RECALL_MAX_OUTPUT_TOKENS: u32 = 256;
const RECALL_RECOVERY_MAX_OUTPUT_TOKENS: u32 = 512;
const FIDELITY_MAX_OUTPUT_TOKENS: u32 = 512;
const TEXT_JUDGE_MAX_OUTPUT_TOKENS: u32 = 256;
const TEXT_JUDGE_RECOVERY_MAX_OUTPUT_TOKENS: u32 = 512;
const LITERAL_ZOOM_MAX_OUTPUT_TOKENS: u32 = 512;
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

/// Execute HTTP requests for the Gemini API.
pub trait Transport {
    /// Execute one authenticated GET request and return the raw response.
    fn get(&self, _url: &str, _key: &str) -> Result<TransportResponse> {
        bail!("authenticated GET is not supported by this transport")
    }

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

    /// Create one HTTP transport bounded for credential validation.
    pub(crate) fn credential() -> Self {
        Self::with_timeout(CREDENTIAL_TIMEOUT)
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
    /// Execute one authenticated GET request and return the raw response.
    fn get(&self, url: &str, key: &str) -> Result<TransportResponse> {
        let mut headers = HeaderMap::new();
        headers.insert("x-goog-api-key", HeaderValue::from_str(key)?);
        let response = self.client.get(url).headers(headers).send()?;
        Ok(TransportResponse {
            status: response.status().as_u16(),
            body: response.text()?,
        })
    }

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CredentialFailure {
    Invalid,
    ModelUnavailable,
    Retryable,
    Operational,
}

/// One sanitized credential-probe failure with stable retry semantics.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{message}")]
pub struct CredentialProbeError {
    failure: CredentialFailure,
    message: &'static str,
}

impl CredentialProbeError {
    fn new(failure: CredentialFailure, message: &'static str) -> Self {
        Self { failure, message }
    }

    /// Return whether Gemini rejected the supplied credential.
    #[must_use]
    pub fn rejects_key(&self) -> bool {
        matches!(self.failure, CredentialFailure::Invalid)
    }

    /// Return whether the configured generation model is unavailable.
    #[must_use]
    pub fn model_unavailable(&self) -> bool {
        matches!(self.failure, CredentialFailure::ModelUnavailable)
    }

    /// Return whether retrying after a transient provider failure can help.
    #[must_use]
    pub fn retryable(&self) -> bool {
        matches!(self.failure, CredentialFailure::Retryable)
    }
}

#[derive(Deserialize)]
struct ModelCatalog {
    #[serde(default)]
    models: Vec<ModelDescription>,
}

impl ModelCatalog {
    fn supports(&self, model: &str, method: &str) -> bool {
        let name = format!("models/{model}");
        self.models.iter().any(|candidate| {
            candidate.name == name
                && candidate
                    .supported_generation_methods
                    .iter()
                    .any(|candidate| candidate == method)
        })
    }
}

#[derive(Deserialize)]
struct ModelDescription {
    name: String,
    #[serde(default, rename = "supportedGenerationMethods")]
    supported_generation_methods: Vec<String>,
}

#[derive(Deserialize)]
struct CredentialErrorEnvelope {
    error: CredentialErrorBody,
}

#[derive(Deserialize)]
struct CredentialErrorBody {
    status: Option<String>,
    #[serde(default)]
    details: Vec<CredentialErrorDetail>,
}

#[derive(Deserialize)]
struct CredentialErrorDetail {
    reason: Option<String>,
}

/// Direct Gemini client with a pluggable transport.
#[derive(Clone)]
pub struct GeminiClient<T> {
    key: String,
    transport: T,
}

impl<T> fmt::Debug for GeminiClient<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GeminiClient")
            .field("key", &"[REDACTED]")
            .field("transport", &self.transport)
            .finish()
    }
}

impl GeminiClient<HttpTransport> {
    /// Build the live Gemini client from a saved key.
    pub fn from_saved(saved: Option<&str>) -> Result<Self> {
        let key = saved
            .filter(|value| !value.trim().is_empty())
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
            .filter(|value| !value.trim().is_empty());
        let key = env_key
            .or_else(|| {
                saved
                    .filter(|value| !value.trim().is_empty())
                    .map(String::from)
            })
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
            .structured_text_observed(
                FEATURE_MODEL,
                feature_prompt,
                &feature_schema,
                ThinkingLevel::Minimal,
                FEATURE_MAX_OUTPUT_TOKENS,
                &mut observe,
            )
            .context("scene feature extraction request failed")?;
        let features = registry.decode_features(unfence(feature_raw.trim()))?;
        let selection = registry
            .eligible(&features)?
            .rank()?
            .select(term, attempt)?;
        let composer_card = selection.composer_card()?;
        let composer =
            render_layout_scene(language, term, sentence, selection.json(), &composer_card)?;
        let composer_raw = self
            .json_text_observed(
                SCENE_MODEL,
                composer,
                ThinkingLevel::Low,
                COMPOSER_MAX_OUTPUT_TOKENS,
                &mut observe,
            )
            .context("scene composition request failed")?;
        compose(composer_raw.as_str(), sentence, target, &selection)
            .map_err(|error| error.context(RejectedReply::new("scene composer", composer_raw)))
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

    /// List available models to confirm the key and configured text model.
    pub fn validate_key(&self) -> Result<()> {
        self.probe_key().map_err(anyhow::Error::from)
    }

    /// Probe credentials without billable generation and classify every failure.
    pub fn probe_key(&self) -> std::result::Result<(), CredentialProbeError> {
        if self.key.trim().is_empty() || HeaderValue::from_str(self.key.as_str()).is_err() {
            return Err(CredentialProbeError::new(
                CredentialFailure::Invalid,
                "the supplied Gemini API key has an invalid format",
            ));
        }
        let response = self
            .transport
            .get(model_catalog_url().as_str(), self.key.as_str())
            .map_err(|_| {
                CredentialProbeError::new(
                    CredentialFailure::Retryable,
                    "could not reach Gemini while checking the API key",
                )
            })?;
        if !(200..300).contains(&response.status) {
            return Err(credential_status(response.status, response.body.as_str()));
        }
        let catalog =
            serde_json::from_str::<ModelCatalog>(response.body.as_str()).map_err(|_| {
                CredentialProbeError::new(
                    CredentialFailure::Operational,
                    "Gemini returned an unreadable model catalog",
                )
            })?;
        if !catalog.supports(TEXT_MODEL, "generateContent") {
            return Err(CredentialProbeError::new(
                CredentialFailure::ModelUnavailable,
                "the configured Gemini text model is unavailable for this API key",
            ));
        }
        Ok(())
    }

    /// Resolve raw user input into reviewed rows using the Flash text model.
    pub fn understand(
        &self,
        raw: &RawInputBatch,
        known: &str,
        target: &LearningTarget,
    ) -> Result<Understood> {
        let catalog = catalog();
        let prompt = render_intake_prompt(raw.text(), known, target, &catalog)?;
        let decoded: IntakeResponse =
            serde_json::from_str(unfence(self.text(TEXT_MODEL, prompt)?.trim()))?;
        let guess = match target {
            LearningTarget::Detect => LearningGuess::new(decoded.target_lang, true),
            LearningTarget::Explicit(expected) => {
                let returned = catalog
                    .resolve(decoded.target_lang.as_str())
                    .context("Gemini understanding returned an unsupported target language")?;
                if returned.as_ref() != expected.as_ref() {
                    bail!(
                        "Gemini understanding target '{}' violates required target '{}'",
                        returned,
                        expected
                    );
                }
                LearningGuess::new(expected.to_string(), true)
            }
        };
        Ok(Understood::new(
            guess,
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
        request: Option<&SentenceLabelSelection>,
    ) -> Result<CardMeta> {
        let (meta, _) = self.generate_card_meta_metered(term, understanding, pair, request)?;
        Ok(meta)
    }

    /// Build rich card meta and return the request cost record.
    pub(crate) fn generate_card_meta_metered(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        request: Option<&SentenceLabelSelection>,
    ) -> Result<(CardMeta, CostRecord)> {
        let catalog = catalog();
        let prompt = render_card_meta_prompt(term, understanding, pair, request, &catalog)?;
        let (raw, cost) = self.text_metered(META_MODEL, prompt)?;
        Ok((card_meta_from_raw(raw.as_str(), request)?, cost))
    }

    /// Build rich card meta and report usage before local JSON decoding.
    pub(crate) fn generate_card_meta_observed<F>(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        request: Option<&SentenceLabelSelection>,
        mut observe: F,
    ) -> Result<CardMeta>
    where
        F: FnMut(CostRecord) -> Result<()>,
    {
        let catalog = catalog();
        let prompt = render_card_meta_prompt(term, understanding, pair, request, &catalog)?;
        let raw = self.text_observed(META_MODEL, prompt, &mut observe)?;
        card_meta_from_raw(raw.as_str(), request)
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
        Ok((decoded.into_revision(label_selection(draft))?, cost))
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
        decoded.into_revision(label_selection(draft))
    }

    /// Render one compiled prose prompt into raw image bytes.
    pub fn image(&self, prompt: &str) -> Result<Vec<u8>> {
        let (bytes, _) = self.image_metered(prompt)?;
        Ok(bytes)
    }

    /// Render one compiled prose prompt and return the request cost record.
    pub(crate) fn image_metered(&self, prompt: &str) -> Result<(Vec<u8>, CostRecord)> {
        let metered = self.request_metered(
            IMAGE_MODEL,
            &Request::text(
                String::from(prompt),
                Some(GenerationConfig::image()),
                Some(GenerationConfig::image_safety()),
            ),
        )?;
        Ok((image_from_response(&metered.response)?, metered.cost))
    }

    /// Render one compiled prose prompt and report usage before local image decoding.
    pub(crate) fn image_observed<F>(&self, prompt: &str, mut observe: F) -> Result<Vec<u8>>
    where
        F: FnMut(CostRecord) -> Result<()>,
    {
        let metered = self.request_metered(
            IMAGE_MODEL,
            &Request::text(
                String::from(prompt),
                Some(GenerationConfig::image()),
                Some(GenerationConfig::image_safety()),
            ),
        )?;
        observe(metered.cost.clone())?;
        image_from_response(&metered.response)
    }

    /// Review one candidate illustration for answer-bearing visible writing.
    pub fn review_recall(
        &self,
        card: &RecallCard,
        scene: &Value,
        mime_type: &str,
        image: &[u8],
    ) -> Result<RecallReview> {
        self.review_recall_observed(card, scene, mime_type, image, |_| Ok(()))
    }

    /// Review one candidate illustration and report usage before verdict decoding.
    pub(crate) fn review_recall_observed<F>(
        &self,
        card: &RecallCard,
        scene: &Value,
        mime_type: &str,
        image: &[u8],
        mut observe: F,
    ) -> Result<RecallReview>
    where
        F: FnMut(CostRecord) -> Result<()>,
    {
        let prompt = card.prompt(scene)?;
        let schema = card.schema()?;
        let data = encode(image);
        let request = |tokens| -> Result<Request> {
            Ok(Request::vision(
                prompt.clone(),
                mime_type,
                data.clone(),
                GenerationConfig::vision_judge(schema.clone())?.with_max_output_tokens(tokens),
            ))
        };
        let mut metered =
            self.request_metered(RECALL_MODEL, &request(RECALL_MAX_OUTPUT_TOKENS)?)?;
        observe(metered.cost.clone())?;
        if metered.response.finish_reason() == Some("MAX_TOKENS") {
            metered =
                self.request_metered(RECALL_MODEL, &request(RECALL_RECOVERY_MAX_OUTPUT_TOKENS)?)?;
            observe(metered.cost.clone())?;
            if metered.response.finish_reason() == Some("MAX_TOKENS") {
                bail!(
                    "Gemini recall review exceeded the adaptive {}-token output ceiling",
                    RECALL_RECOVERY_MAX_OUTPUT_TOKENS
                );
            }
        }
        let raw = response_text(&metered.response);
        if raw.trim().is_empty() {
            bail!(
                "No text content in Gemini recall review response: {}",
                diagnosis(&metered.response)
            );
        }
        card.review(unfence(raw.trim()))
    }

    /// Review one full illustration against only its compact scene-fidelity contract.
    pub(crate) fn review_fidelity_observed<F>(
        &self,
        check: &FidelityCheck,
        mime_type: &str,
        image: &[u8],
        mut observe: F,
    ) -> Result<FidelityReview>
    where
        F: FnMut(CostRecord) -> Result<()>,
    {
        let prompt = check.prompt()?;
        let schema = check.schema()?;
        let request = Request::vision(
            prompt,
            mime_type,
            encode(image),
            GenerationConfig::structured_vision_judge(schema)?
                .with_thinking_level(ThinkingLevel::Minimal)
                .with_max_output_tokens(FIDELITY_MAX_OUTPUT_TOKENS),
        );
        let metered = self.request_metered(FIDELITY_MODEL, &request)?;
        observe(metered.cost.clone())?;
        if metered.response.finish_reason() == Some("MAX_TOKENS") {
            bail!(
                "Gemini fidelity review exceeded the {}-token output ceiling",
                FIDELITY_MAX_OUTPUT_TOKENS
            );
        }
        let raw = response_text(&metered.response);
        if raw.trim().is_empty() {
            bail!(
                "No text content in Gemini fidelity review response: {}",
                diagnosis(&metered.response)
            );
        }
        check.review(unfence(raw.trim()))
    }

    /// Review nine enlarged overlapping crops for literal-looking content in one request.
    pub(crate) fn review_literal_zoom_observed<F>(
        &self,
        check: &LiteralZoomCheck,
        images: &[Vec<u8>],
        mut observe: F,
    ) -> Result<LiteralZoomReview>
    where
        F: FnMut(CostRecord) -> Result<()>,
    {
        if images.len() != 9 {
            bail!("literal zoom review requires exactly nine ordered crops");
        }
        let prompt = check.prompt();
        let schema = check.schema()?;
        let data = images.iter().map(|image| encode(image)).collect::<Vec<_>>();
        let request = Request::vision_images(
            String::from(prompt),
            "image/png",
            data,
            GenerationConfig::structured_vision_judge(schema)?
                .with_thinking_level(ThinkingLevel::Minimal)
                .with_max_output_tokens(LITERAL_ZOOM_MAX_OUTPUT_TOKENS),
        );
        let metered = self.request_metered(LITERAL_ZOOM_MODEL, &request)?;
        observe(metered.cost.clone())?;
        if metered.response.finish_reason() == Some("MAX_TOKENS") {
            bail!(
                "Gemini literal zoom review exceeded the {}-token output ceiling",
                LITERAL_ZOOM_MAX_OUTPUT_TOKENS
            );
        }
        let raw = response_text(&metered.response);
        if raw.trim().is_empty() {
            bail!(
                "No text content in Gemini literal zoom review response: {}",
                diagnosis(&metered.response)
            );
        }
        check.review(unfence(raw.trim()))
    }

    /// Review one candidate illustration for literal writing without OCR.
    pub fn review_text(
        &self,
        check: &TextCheck,
        mime_type: &str,
        image: &[u8],
    ) -> Result<TextReview> {
        self.review_text_observed(check, mime_type, image, |_| Ok(()))
    }

    /// Review literal writing directly and report usage before verdict decoding.
    pub(crate) fn review_text_observed<F>(
        &self,
        check: &TextCheck,
        mime_type: &str,
        image: &[u8],
        mut observe: F,
    ) -> Result<TextReview>
    where
        F: FnMut(CostRecord) -> Result<()>,
    {
        let prompt = check.prompt()?;
        let schema = check.schema()?;
        let data = encode(image);
        let request = |tokens| -> Result<Request> {
            Ok(Request::vision(
                prompt.clone(),
                mime_type,
                data.clone(),
                GenerationConfig::vision_judge(schema.clone())?.with_max_output_tokens(tokens),
            ))
        };
        let mut metered =
            self.request_metered(TEXT_JUDGE_MODEL, &request(TEXT_JUDGE_MAX_OUTPUT_TOKENS)?)?;
        observe(metered.cost.clone())?;
        if metered.response.finish_reason() == Some("MAX_TOKENS") {
            metered = self.request_metered(
                TEXT_JUDGE_MODEL,
                &request(TEXT_JUDGE_RECOVERY_MAX_OUTPUT_TOKENS)?,
            )?;
            observe(metered.cost.clone())?;
            if metered.response.finish_reason() == Some("MAX_TOKENS") {
                bail!(
                    "Gemini text review exceeded the adaptive {}-token output ceiling",
                    TEXT_JUDGE_RECOVERY_MAX_OUTPUT_TOKENS
                );
            }
        }
        let raw = response_text(&metered.response);
        if raw.trim().is_empty() {
            bail!(
                "No text content in Gemini text review response: {}",
                diagnosis(&metered.response)
            );
        }
        check.review(unfence(raw.trim()))
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
        thinking: ThinkingLevel,
        max_output_tokens: u32,
        observe: &mut F,
    ) -> Result<String>
    where
        F: FnMut(CostRecord) -> Result<()>,
    {
        let config = GenerationConfig::json(schema.clone())?
            .with_thinking_level(thinking)
            .with_max_output_tokens(max_output_tokens);
        let request = Request::text(prompt.clone(), Some(config), None);
        let metered = match self.request_metered(model, &request) {
            Ok(metered) => metered,
            Err(error) if schema_rejected(&error) => self.request_metered(
                model,
                &Request::text(
                    prompt,
                    Some(
                        GenerationConfig::json_mode()
                            .with_thinking_level(thinking)
                            .with_max_output_tokens(max_output_tokens),
                    ),
                    None,
                ),
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

    fn json_text_observed<F>(
        &self,
        model: &str,
        prompt: String,
        thinking: ThinkingLevel,
        max_output_tokens: u32,
        observe: &mut F,
    ) -> Result<String>
    where
        F: FnMut(CostRecord) -> Result<()>,
    {
        let metered = self.request_metered(
            model,
            &Request::text(
                prompt,
                Some(
                    GenerationConfig::json_mode()
                        .with_thinking_level(thinking)
                        .with_max_output_tokens(max_output_tokens),
                ),
                None,
            ),
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

fn credential_status(status: u16, body: &str) -> CredentialProbeError {
    match status {
        401 => CredentialProbeError::new(
            CredentialFailure::Invalid,
            "Gemini rejected the supplied API key",
        ),
        400 | 403 if credential_rejected(body) => CredentialProbeError::new(
            CredentialFailure::Invalid,
            "Gemini rejected the supplied API key",
        ),
        404 => CredentialProbeError::new(
            CredentialFailure::ModelUnavailable,
            "the Gemini model catalog is unavailable",
        ),
        408 | 429 | 500..=599 => CredentialProbeError::new(
            CredentialFailure::Retryable,
            "Gemini credential validation is temporarily unavailable",
        ),
        _ => CredentialProbeError::new(
            CredentialFailure::Operational,
            "Gemini credential validation failed",
        ),
    }
}

fn credential_rejected(body: &str) -> bool {
    let Ok(envelope) = serde_json::from_str::<CredentialErrorEnvelope>(body) else {
        return false;
    };
    if envelope
        .error
        .status
        .as_deref()
        .is_some_and(credential_rejection_code)
    {
        return true;
    }
    envelope
        .error
        .details
        .iter()
        .filter_map(|detail| detail.reason.as_deref())
        .any(credential_rejection_code)
}

fn credential_rejection_code(code: &str) -> bool {
    matches!(
        code,
        "API_KEY_INVALID" | "UNAUTHENTICATED" | "PERMISSION_DENIED"
    )
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

fn card_meta_from_raw(raw: &str, request: Option<&SentenceLabelSelection>) -> Result<CardMeta> {
    let decoded: CardMetaResponse = serde_json::from_str(unfence(raw.trim()))?;
    decoded.into_meta(request)
}

fn label_selection(draft: &CardDraft) -> SentenceLabelSelection {
    if let Some(rewrite) = draft.rewrite() {
        return rewrite.selection().clone();
    }
    draft
        .meta()
        .and_then(CardMeta::sentence_labels)
        .map(SentenceLabelSelection::from_labels)
        .unwrap_or_default()
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
#[serde(deny_unknown_fields)]
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
    labels: SentenceLabelsResponse,
}

impl CardMetaResponse {
    fn into_meta(self, request: Option<&SentenceLabelSelection>) -> Result<CardMeta> {
        let labels = self.labels.into_labels()?;
        let labels = match request {
            Some(request) => {
                if labels.approx().len() != labels.approx().intersecting(request.pinned()).len() {
                    bail!("approximate sentence labels must name only requested axes");
                }
                if !request.accepts(&labels) {
                    bail!("sentence labels changed the requested initial preset");
                }
                request.reconciled(labels)
            }
            None if labels.approx().is_empty() => labels,
            None => bail!("initial sentence labels cannot report approximate axes"),
        };
        Ok(CardMeta::new(
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
        .with_sentence_labels(labels))
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
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
    labels: SentenceLabelsResponse,
}

impl CardCorrectionResponse {
    fn into_revision(self, selection: SentenceLabelSelection) -> Result<CardRevision> {
        let labels = self.labels.into_labels()?;
        if labels.approx().len() != labels.approx().intersecting(selection.pinned()).len() {
            bail!("approximate sentence labels must name only changed axes");
        }
        if !selection.accepts(&labels) {
            bail!("sentence labels changed the requested preset");
        }
        let labels = selection.reconciled(labels);
        let meta = CardMeta::new(
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
        .with_sentence_labels(labels);
        Ok(CardRevision::new(self.term, self.understanding, meta))
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SentenceLabelsResponse {
    register: Register,
    level: SentenceLevelProfile,
    #[serde(rename = "type")]
    kind: SentenceKind,
    #[serde(default)]
    approx: Vec<SentenceAxis>,
}

impl SentenceLabelsResponse {
    fn into_labels(self) -> Result<SentenceLabels> {
        let approx = AxisSet::from_axes(self.approx.iter().copied());
        if approx.len() != self.approx.len() {
            bail!("approximate sentence labels cannot contain duplicate axes");
        }
        Ok(SentenceLabels::new(
            self.register,
            self.level.level(),
            self.kind,
            AxisSet::default(),
            approx,
        ))
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SentenceLevelProfile {
    A1,
    A2,
    B1,
    B2,
    C1,
    C2,
}

impl SentenceLevelProfile {
    fn level(self) -> SentenceLevel {
        match self {
            Self::A1 => SentenceLevel::A1,
            Self::A2 => SentenceLevel::A2,
            Self::B1 => SentenceLevel::B1,
            Self::B2 => SentenceLevel::B2,
            Self::C1 => SentenceLevel::C1,
            Self::C2 => SentenceLevel::C2,
        }
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
        urls: Rc<RefCell<Vec<String>>>,
    }

    impl FakeTransport {
        fn new(responses: Vec<Result<TransportResponse>>) -> Self {
            Self {
                responses: Rc::new(RefCell::new(responses)),
                requests: Rc::new(RefCell::new(Vec::new())),
                urls: Rc::new(RefCell::new(Vec::new())),
            }
        }
    }

    impl Transport for FakeTransport {
        fn get(&self, url: &str, _key: &str) -> Result<TransportResponse> {
            self.urls.borrow_mut().push(String::from(url));
            self.requests.borrow_mut().push(String::from("GET"));
            self.responses.borrow_mut().remove(0)
        }

        fn post(&self, url: &str, _key: &str, body: &str) -> Result<TransportResponse> {
            self.urls.borrow_mut().push(String::from(url));
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

    fn sentence_labels_response(
        register: &str,
        level: &str,
        kind: &str,
        approx: Vec<&str>,
    ) -> Value {
        json!({
            "register": register,
            "level": level,
            "type": kind,
            "approx": approx
        })
    }

    fn card_meta_response(labels: Value) -> Value {
        json!({
            "pronunciation": "ka.naʁ",
            "transcription": "lə ka.naʁ naʒ",
            "meaning": "a duck",
            "importance": 5,
            "source_sentence": "The duck swims",
            "source_highlight": "duck",
            "source_hint": "Think of a pond",
            "source_context": "A concrete noun",
            "target_sentence": "Le canard nage",
            "labels": labels
        })
    }

    fn card_correction_response(labels: Value) -> Value {
        json!({
            "term": "canard",
            "understanding": "a duck",
            "pronunciation": "ka.naʁ",
            "transcription": "lə ka.naʁ naʒ",
            "meaning": "a duck",
            "importance": 5,
            "source_sentence": "The duck swims",
            "source_highlight": "duck",
            "source_hint": "Think of a pond",
            "source_context": "A concrete noun",
            "target_sentence": "Le canard nage",
            "labels": labels
        })
    }

    fn changed_register_selection() -> SentenceLabelSelection {
        SentenceLabelSelection::from_labels(&SentenceLabels::new(
            Register::Formal,
            SentenceLevel::B1,
            SentenceKind::Statement,
            AxisSet::from_axes([SentenceAxis::Register]),
            AxisSet::default(),
        ))
    }

    fn preserved_selection() -> SentenceLabelSelection {
        SentenceLabelSelection::from_labels(&SentenceLabels::new(
            Register::Neutral,
            SentenceLevel::B1,
            SentenceKind::Statement,
            AxisSet::default(),
            AxisSet::default(),
        ))
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

    fn recall_scene() -> Value {
        json!({
            "manga_panel": {
                "semantic_spine": {
                    "literal_event": "one frightened person reacts to danger",
                    "semantic_focus": "visible fear",
                    "visual_relation": "cause_and_effect",
                    "metaphor": {"literal_anchor": "the frightened person"}
                },
                "panels": [{
                    "id": "p1",
                    "semantic_job": "show the frightened person reacting",
                    "shot_contract": {"visible_anchor": "one visibly frightened person"},
                    "scene": {"subjects": [{
                        "id": "person",
                        "figure": "a person",
                        "pose": "recoiling from danger",
                        "expression": "frightened"
                    }]}
                }]
            }
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
    fn credential_validation_uses_a_shorter_timeout() {
        assert_eq!(
            (
                CREDENTIAL_TIMEOUT <= Duration::from_secs(20),
                CREDENTIAL_TIMEOUT < HTTP_TIMEOUT,
            ),
            (true, true),
            "credential validation must not inherit the five-minute generation timeout"
        );
    }

    #[test]
    fn credential_catalog_preserves_override_queries() {
        assert_eq!(
            catalog_url("http://127.0.0.1/models?fixture=available"),
            "http://127.0.0.1/models?fixture=available&pageSize=1000",
            "credential catalog pagination replaced an existing override query"
        );
    }

    #[test]
    fn card_correction_returns_request_cost() {
        let transport = FakeTransport::new(vec![body(json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "text": "{\"term\":\"wound\",\"understanding\":\"verb sense\",\"pronunciation\":\"waʊnd\",\"transcription\":\"aɪ waʊnd ðə klɒk\",\"meaning\":\"завести\",\"importance\":6,\"source_sentence\":\"Я завел часы.\",\"source_highlight\":\"завел\",\"source_hint\":\"Поворачивал что-то круглое.\",\"source_context\":\"Глагол про часы.\",\"target_sentence\":\"I wound the clock.\",\"labels\":{\"register\":\"neutral\",\"level\":\"b1\",\"type\":\"statement\",\"approx\":[]}}"
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
    fn card_responses_reject_fields_outside_the_exact_contract() {
        let meta = serde_json::from_value::<CardMetaResponse>(json!({
            "pronunciation": "ka.naʁ",
            "transcription": "lə ka.naʁ naʒ",
            "meaning": "a duck",
            "importance": 5,
            "source_sentence": "The duck swims",
            "source_highlight": "duck",
            "source_hint": "Think of a pond",
            "source_context": "A concrete noun",
            "target_sentence": "Le canard nage",
            "labels": {
                "register": "neutral",
                "level": "b1",
                "type": "statement",
                "approx": []
            },
            "unexpected": true
        }));
        let correction = serde_json::from_value::<CardCorrectionResponse>(json!({
            "term": "canard",
            "understanding": "a duck",
            "pronunciation": "ka.naʁ",
            "transcription": "lə ka.naʁ naʒ",
            "meaning": "a duck",
            "importance": 5,
            "source_sentence": "The duck swims",
            "source_highlight": "duck",
            "source_hint": "Think of a pond",
            "source_context": "A concrete noun",
            "target_sentence": "Le canard nage",
            "labels": {
                "register": "neutral",
                "level": "b1",
                "type": "statement",
                "approx": []
            },
            "unexpected": true
        }));
        assert_eq!(
            (meta.is_err(), correction.is_err()),
            (true, true),
            "card responses accepted fields outside their exact JSON contracts"
        );
    }

    #[test]
    fn sentence_labels_reject_unknown_tokens() {
        let response = serde_json::from_value::<CardMetaResponse>(card_meta_response(
            sentence_labels_response("ceremonial", "b1", "statement", Vec::new()),
        ));
        assert!(
            response.is_err(),
            "sentence labels accepted a token outside the closed register vocabulary"
        );
    }

    #[test]
    fn sentence_labels_reject_legacy_effort_tokens_on_the_model_wire() {
        let rejected = [
            "easy",
            "takes practice",
            "balanced",
            "challenging",
            "stretch",
            "simplified",
            "default",
            "extended",
        ]
        .map(|level| {
            serde_json::from_value::<CardMetaResponse>(card_meta_response(
                sentence_labels_response("neutral", level, "statement", Vec::new()),
            ))
            .is_err()
        });
        assert_eq!(
            rejected,
            [true, true, true, true, true, true, true, true],
            "Gemini labels accepted a legacy effort token"
        );
    }

    #[test]
    fn sentence_labels_map_every_lowercase_cefr_token() {
        let levels = ["a1", "a2", "b1", "b2", "c1", "c2"].map(|level| {
            serde_json::from_value::<CardMetaResponse>(card_meta_response(
                sentence_labels_response("neutral", level, "statement", Vec::new()),
            ))
            .expect("lowercase CEFR labels must decode")
            .into_meta(None)
            .expect("lowercase CEFR labels must validate")
        });
        assert_eq!(
            levels.map(|meta| {
                meta.sentence_labels()
                    .map(SentenceLabels::level)
                    .expect("CEFR response must keep labels")
            }),
            [
                SentenceLevel::A1,
                SentenceLevel::A2,
                SentenceLevel::B1,
                SentenceLevel::B2,
                SentenceLevel::C1,
                SentenceLevel::C2,
            ],
            "lowercase CEFR tokens failed to map onto the visible level scale"
        );
    }

    #[test]
    fn sentence_labels_reject_the_removed_grammar_field() {
        let mut labels = sentence_labels_response("neutral", "b1", "statement", Vec::new());
        labels
            .as_object_mut()
            .expect("label response must be a JSON object")
            .insert(String::from("grammar"), Value::Null);
        let response = serde_json::from_value::<CardMetaResponse>(card_meta_response(labels));
        assert!(
            response.is_err(),
            "sentence labels retained a compatibility path for the removed grammar field"
        );
    }

    #[test]
    fn initial_sentence_labels_reject_approximate_axes() {
        let response = serde_json::from_value::<CardMetaResponse>(card_meta_response(
            sentence_labels_response("neutral", "b1", "statement", vec!["register"]),
        ))
        .expect("initial labels must decode before semantic validation");
        assert!(
            response.into_meta(None).is_err(),
            "initial sentence labels accepted an approximate axis"
        );
    }

    #[test]
    fn requested_initial_labels_preserve_actual_attribution_and_target() {
        let request = SentenceLabelSelection::empty()
            .choosing(SentenceAxis::Level, 2)
            .choosing(SentenceAxis::Type, 1);
        let response = serde_json::from_value::<CardMetaResponse>(card_meta_response(
            sentence_labels_response("neutral", "b2", "statement", vec!["level", "type"]),
        ))
        .expect("requested initial labels must decode before semantic validation");
        let meta = response
            .into_meta(Some(&request))
            .expect("requested approximate labels must validate");
        let labels = meta
            .sentence_labels()
            .expect("requested initial metadata must retain labels");
        assert_eq!(
            (
                labels.level(),
                labels.kind(),
                labels.requested_token(SentenceAxis::Level),
                labels.requested_token(SentenceAxis::Type),
                labels.pinned().contains(SentenceAxis::Level),
                labels.pinned().contains(SentenceAxis::Type),
                labels.approx().contains(SentenceAxis::Level),
                labels.approx().contains(SentenceAxis::Type),
            ),
            (
                SentenceLevel::B2,
                SentenceKind::Statement,
                Some("b1"),
                Some("question"),
                true,
                true,
                true,
                true,
            ),
            "requested initial labels erased the actual attribution or requested target"
        );
    }

    #[test]
    fn exact_requested_initial_labels_accept_an_omitted_empty_approx() {
        let request = SentenceLabelSelection::empty()
            .choosing(SentenceAxis::Level, 2)
            .choosing(SentenceAxis::Type, 1);
        let mut labels = sentence_labels_response("neutral", "b1", "question", Vec::new());
        labels
            .as_object_mut()
            .expect("sentence labels must be an object")
            .remove("approx");
        let response = serde_json::from_value::<CardMetaResponse>(card_meta_response(labels))
            .expect("omitted empty approx must decode");
        assert!(
            response.into_meta(Some(&request)).is_ok(),
            "an omitted empty approx rejected an otherwise exact initial preset"
        );
    }

    #[test]
    fn omitted_empty_approx_cannot_hide_an_initial_preset_mismatch() {
        let request = SentenceLabelSelection::empty()
            .choosing(SentenceAxis::Level, 2)
            .choosing(SentenceAxis::Type, 1);
        let mut labels = sentence_labels_response("neutral", "a2", "statement", Vec::new());
        labels
            .as_object_mut()
            .expect("sentence labels must be an object")
            .remove("approx");
        let response = serde_json::from_value::<CardMetaResponse>(card_meta_response(labels))
            .expect("omitted empty approx must decode before preset validation");
        assert!(
            response.into_meta(Some(&request)).is_err(),
            "an omitted approx silently accepted a mismatched initial preset"
        );
    }

    #[test]
    fn requested_initial_labels_reject_approximation_on_an_unrequested_axis() {
        let request = SentenceLabelSelection::empty().choosing(SentenceAxis::Level, 2);
        let response = serde_json::from_value::<CardMetaResponse>(card_meta_response(
            sentence_labels_response("neutral", "b1", "statement", vec!["register"]),
        ))
        .expect("initial labels must decode before semantic validation");
        assert!(
            response.into_meta(Some(&request)).is_err(),
            "initial metadata approximated an axis the batch did not request"
        );
    }

    #[test]
    fn correction_rejects_a_changed_label_mismatch_without_approximation() {
        let response = serde_json::from_value::<CardCorrectionResponse>(card_correction_response(
            sentence_labels_response("casual", "b1", "statement", Vec::new()),
        ))
        .expect("correction labels must decode before semantic validation");
        assert!(
            response
                .into_revision(changed_register_selection())
                .is_err(),
            "card correction accepted a mismatched changed register without approximation"
        );
    }

    #[test]
    fn correction_rejects_drift_on_an_unchanged_label_axis() {
        let response = serde_json::from_value::<CardCorrectionResponse>(card_correction_response(
            sentence_labels_response("formal", "b2", "statement", Vec::new()),
        ))
        .expect("correction labels must decode before semantic validation");
        assert!(
            response
                .into_revision(changed_register_selection())
                .is_err(),
            "card correction changed the preserved CEFR label"
        );
    }

    #[test]
    fn correction_preserves_unchanged_axes_without_pinning_them() {
        let response = serde_json::from_value::<CardCorrectionResponse>(card_correction_response(
            sentence_labels_response("formal", "b1", "statement", Vec::new()),
        ))
        .expect("correction labels must decode before semantic validation");
        let revision = response
            .into_revision(changed_register_selection())
            .expect("matching unchanged axes must be accepted");
        let labels = revision
            .meta()
            .sentence_labels()
            .expect("accepted correction must keep labels");
        assert_eq!(
            (
                labels.register(),
                labels.level(),
                labels.kind(),
                labels.pinned().iter().collect::<Vec<_>>(),
            ),
            (
                Register::Formal,
                SentenceLevel::B1,
                SentenceKind::Statement,
                vec![SentenceAxis::Register],
            ),
            "card correction changed or pinned an unchanged label axis"
        );
    }

    #[test]
    fn correction_preserves_an_approximate_actual_label_and_requested_target() {
        let response = serde_json::from_value::<CardCorrectionResponse>(card_correction_response(
            sentence_labels_response("casual", "b1", "statement", vec!["register"]),
        ))
        .expect("correction labels must decode before semantic validation");
        let revision = response
            .into_revision(changed_register_selection())
            .expect("approximate changed mismatch must be accepted");
        let labels = revision
            .meta()
            .sentence_labels()
            .expect("reconciled revision must keep labels");
        assert_eq!(
            (
                labels.register(),
                labels.requested_token(SentenceAxis::Register),
                labels.pinned().contains(SentenceAxis::Register),
                labels.approx().contains(SentenceAxis::Register),
            ),
            (Register::Casual, Some("formal"), true, true),
            "approximate changed register erased the actual attribution or requested target"
        );
    }

    #[test]
    fn correction_rejects_approximation_on_a_preserved_axis() {
        let response = serde_json::from_value::<CardCorrectionResponse>(card_correction_response(
            sentence_labels_response("neutral", "b1", "statement", vec!["level"]),
        ))
        .expect("correction labels must decode before semantic validation");
        assert!(
            response.into_revision(preserved_selection()).is_err(),
            "card correction accepted approximation on a preserved label axis"
        );
    }

    #[test]
    fn sentence_labels_reject_duplicate_approximate_axes() {
        let response = serde_json::from_value::<CardMetaResponse>(card_meta_response(
            sentence_labels_response("neutral", "b1", "statement", vec!["register", "register"]),
        ))
        .expect("duplicate approximate axes must decode before semantic validation");
        assert!(
            response.into_meta(None).is_err(),
            "sentence labels accepted duplicate approximate axes"
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
    fn direct_text_judge_uses_structured_flash_vision_and_reports_its_cost() {
        let transport = FakeTransport::new(vec![text_body(&json!({
            "decision": "REJECT",
            "evidence": [{
                "reading": "שלום",
                "location": "center sign",
                "kind": "WRITING"
            }],
            "reason": "The word is clearly legible"
        }))]);
        let requests = transport.requests.clone();
        let client = GeminiClient::new("key", transport);
        let check = TextCheck::new("Hebrew");
        let mut costs = Vec::new();
        let review = client.review_text_observed(&check, "image/jpeg", &[1, 2, 3], |cost| {
            costs.push(cost);
            Ok(())
        });
        let payload = serde_json::from_str::<Value>(&requests.borrow()[0])
            .expect("direct text request must decode");
        assert_eq!(
            (
                review.is_ok_and(|review| !review.allows()),
                requests.borrow().len(),
                payload["contents"][0]["parts"][0]["text"]
                    .as_str()
                    .is_some_and(|prompt| prompt.contains("Hebrew")),
                payload["contents"][0]["parts"][1]["inlineData"]["data"].as_str(),
                payload["generationConfig"]["responseSchema"]["properties"]["evidence"]
                    ["items"]["properties"]["kind"]["enum"][0]
                    .as_str(),
                costs.first().map(|cost| cost.model()),
            ),
            (
                true,
                1,
                true,
                Some("AQID"),
                Some("WRITING"),
                Some("gemini-3.5-flash-lite"),
            ),
            "direct text validation lost its prompt, image, schema, verdict, or Flash-tier cost"
        );
    }

    #[test]
    fn literal_zoom_review_sends_nine_ordered_png_parts_in_one_request() {
        let transport = FakeTransport::new(vec![text_body(&json!({
            "literal_writing_present": false,
            "literal_evidence": [],
            "reason": "No literal writing appears in any enlarged crop"
        }))]);
        let requests = transport.requests.clone();
        let urls = transport.urls.clone();
        let client = GeminiClient::new("key", transport);
        let crops = (0_u8..9).map(|value| vec![value]).collect::<Vec<_>>();
        let mut costs = Vec::new();
        let review = client.review_literal_zoom_observed(
            &LiteralZoomCheck::new(),
            crops.as_slice(),
            |cost| {
                costs.push(cost);
                Ok(())
            },
        );
        let payload = serde_json::from_str::<Value>(&requests.borrow()[0])
            .expect("literal zoom request must decode");
        let parts = payload["contents"][0]["parts"]
            .as_array()
            .expect("literal zoom request parts must be an array");
        assert_eq!(
            [
                review.is_ok(),
                requests.borrow().len() == 1,
                parts.len() == 10,
                parts[0]["text"].as_str().is_some_and(|prompt| {
                    prompt.contains("nine enlarged overlapping crops")
                        && !prompt.contains("hidden_focus_term")
                        && !prompt.contains("CARD")
                }),
                parts[1]["inlineData"]["mimeType"].as_str() == Some("image/png"),
                parts[1]["inlineData"]["data"].as_str() == Some("AA=="),
                parts[9]["inlineData"]["data"].as_str() == Some("CA=="),
                payload["generationConfig"]["mediaResolution"].as_str()
                    == Some("MEDIA_RESOLUTION_HIGH"),
                payload["generationConfig"]["thinkingConfig"]["thinkingLevel"].as_str()
                    == Some("MINIMAL"),
                payload["generationConfig"]["maxOutputTokens"].as_u64() == Some(512),
                payload["generationConfig"]["responseFormat"]["text"]["schema"]["properties"]
                    ["literal_evidence"]["items"]["properties"]["kind"]["enum"]
                    .as_array()
                    .is_some_and(|items| items.iter().any(|item| item == "PSEUDO_WRITING")),
                payload["generationConfig"]["responseFormat"]["text"]["mimeType"].as_str()
                    == Some("APPLICATION_JSON"),
                payload["generationConfig"]["responseMimeType"].is_null(),
                payload["generationConfig"]["responseSchema"].is_null(),
                payload["generationConfig"]["temperature"].as_u64() == Some(0),
                costs.len() == 1,
                urls.borrow()[0].ends_with("/gemini-3.6-flash:generateContent"),
                costs.first().map(CostRecord::model) == Some("gemini-3.6-flash"),
                costs.first().map(|cost| cost.cost().nanos()) == Some(525_000),
            ],
            [true; 19],
            "literal zoom review lost crop order, sent card data, changed media policy, model, cost, or request count"
        );
    }

    #[test]
    fn dedicated_fidelity_review_uses_one_modern_full_image_request_without_card_data() {
        let transport = FakeTransport::new(vec![text_body(&json!({
            "scene_fidelity_decision": "REJECT",
            "scene_fidelity_evidence": [{
                "requirement": "person remains the same subject",
                "observed": "a visibly different person replaces the subject",
                "location": "left and right panels",
                "kind": "BROKEN_SUBJECT_CONTINUITY"
            }],
            "reason": "The repeated subject is substituted"
        }))]);
        let requests = transport.requests.clone();
        let urls = transport.urls.clone();
        let client = GeminiClient::new("key", transport);
        let check = FidelityCheck::new(&recall_scene()).expect("fidelity check must compose");
        let mut costs = Vec::new();
        let review = client.review_fidelity_observed(&check, "image/jpeg", &[1, 2, 3], |cost| {
            costs.push(cost);
            Ok(())
        });
        let payload = serde_json::from_str::<Value>(&requests.borrow()[0])
            .expect("fidelity request must decode");
        let prompt = payload["contents"][0]["parts"][0]["text"]
            .as_str()
            .expect("fidelity request must contain a prompt");
        assert_eq!(
            [
                review.is_ok(),
                requests.borrow().len() == 1,
                urls.borrow()[0].ends_with("/gemini-3.6-flash:generateContent"),
                prompt.contains("SCENE FIDELITY REFERENCE")
                    && prompt.contains("\"id\": \"person\"")
                    && !prompt.contains("hidden_focus_term")
                    && !prompt.contains("shown_source_sentence")
                    && !prompt.contains("CARD"),
                payload["contents"][0]["parts"].as_array().map(Vec::len) == Some(2),
                payload["contents"][0]["parts"][1]["inlineData"]["mimeType"].as_str()
                    == Some("image/jpeg"),
                payload["contents"][0]["parts"][1]["inlineData"]["data"].as_str()
                    == Some("AQID"),
                payload["generationConfig"]["responseFormat"]["text"]["mimeType"].as_str()
                    == Some("APPLICATION_JSON"),
                payload["generationConfig"]["responseFormat"]["text"]["schema"]
                    ["properties"]["scene_fidelity_evidence"]["items"]["properties"]["kind"]
                    ["enum"]
                    .as_array()
                    .is_some_and(|items| items.iter().any(|item| item == "BROKEN_SUBJECT_CONTINUITY")),
                payload["generationConfig"]["responseMimeType"].is_null()
                    && payload["generationConfig"]["responseSchema"].is_null(),
                payload["generationConfig"]["thinkingConfig"]["thinkingLevel"].as_str()
                    == Some("MINIMAL"),
                payload["generationConfig"]["mediaResolution"].as_str()
                    == Some("MEDIA_RESOLUTION_HIGH"),
                payload["generationConfig"]["temperature"].as_u64() == Some(0),
                payload["generationConfig"]["maxOutputTokens"].as_u64() == Some(512),
                costs.len() == 1
                    && costs.first().map(CostRecord::model) == Some("gemini-3.6-flash"),
            ],
            [true; 15],
            "dedicated fidelity request leaked card data or changed its one-shot modern vision contract"
        );
    }

    #[test]
    fn dedicated_fidelity_max_tokens_fails_without_an_adaptive_second_request() {
        let transport = FakeTransport::new(vec![body(json!({
            "candidates": [{
                "finishReason": "MAX_TOKENS",
                "content": {"parts": [{"text": ""}]}
            }],
            "usageMetadata": {
                "promptTokenCount": 100,
                "candidatesTokenCount": 512,
                "totalTokenCount": 612
            }
        }))]);
        let requests = transport.requests.clone();
        let client = GeminiClient::new("key", transport);
        let check = FidelityCheck::new(&recall_scene()).expect("fidelity check must compose");
        let result = client.review_fidelity_observed(&check, "image/jpeg", &[1, 2, 3], |_| Ok(()));
        assert_eq!(
            (result.is_err(), requests.borrow().len()),
            (true, 1),
            "dedicated fidelity review retried a max-token response or accepted a truncated verdict"
        );
    }

    #[test]
    fn literal_zoom_max_tokens_fails_without_an_adaptive_second_request() {
        let transport = FakeTransport::new(vec![body(json!({
            "candidates": [{
                "finishReason": "MAX_TOKENS",
                "content": {"parts": [{"text": ""}]}
            }],
            "usageMetadata": {
                "promptTokenCount": 100,
                "candidatesTokenCount": 256,
                "totalTokenCount": 356
            }
        }))]);
        let requests = transport.requests.clone();
        let client = GeminiClient::new("key", transport);
        let crops = vec![vec![1_u8]; 9];
        let result =
            client.review_literal_zoom_observed(&LiteralZoomCheck::new(), crops.as_slice(), |_| {
                Ok(())
            });
        assert_eq!(
            (result.is_err(), requests.borrow().len()),
            (true, 1),
            "literal zoom review retried a max-token response or accepted a truncated verdict"
        );
    }

    #[test]
    fn invalid_recall_verdict_still_reports_multimodal_request_cost() {
        let transport = FakeTransport::new(vec![body(json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "{not valid recall verdict"}]
                }
            }],
            "usageMetadata": {
                "promptTokenCount": 536,
                "candidatesTokenCount": 76,
                "thoughtsTokenCount": 0,
                "totalTokenCount": 612
            }
        }))]);
        let client = GeminiClient::new("key", transport);
        let card = RecallCard::new(
            crate::generation::manga::ShownRecall::new(
                "Russian",
                "Она выглядела испуганной.",
                "испуганной",
                "О сильном страхе.",
            ),
            crate::generation::manga::HiddenRecall::new(
                "English",
                "frightened",
                "She looked frightened.",
            ),
        );
        let mut costs = Vec::new();
        let result = client.review_recall_observed(
            &card,
            &recall_scene(),
            "image/jpeg",
            &[1, 2, 3],
            |cost| {
                costs.push(cost);
                Ok(())
            },
        );
        assert_eq!(
            (
                result.is_err(),
                costs.first().map(CostRecord::requests),
                costs.first().map(CostRecord::model),
                costs.first().map(|cost| cost.cost().nanos()),
            ),
            (true, Some(1), Some("gemini-3.5-flash-lite"), Some(350_800),),
            "invalid recall JSON discarded the billed multimodal Gemini request cost"
        );
    }

    #[test]
    fn max_tokens_recall_review_retries_once_with_more_room_and_observes_both_costs() {
        let transport = FakeTransport::new(vec![
            body(json!({
                "candidates": [{
                    "content": {"parts": [{"text": "{\"decision\":\"ALLOW\""}]},
                    "finishReason": "MAX_TOKENS"
                }],
                "usageMetadata": {
                    "promptTokenCount": 500,
                    "candidatesTokenCount": 256,
                    "thoughtsTokenCount": 0,
                    "totalTokenCount": 756
                }
            })),
            body(json!({
                "candidates": [{
                    "content": {
                        "parts": [{
                            "text": "{\"decision\":\"ALLOW\",\"evidence\":[],\"reason\":\"No answer-bearing writing is visible\"}"
                        }]
                    },
                    "finishReason": "STOP"
                }],
                "usageMetadata": {
                    "promptTokenCount": 500,
                    "candidatesTokenCount": 40,
                    "thoughtsTokenCount": 0,
                    "totalTokenCount": 540
                }
            })),
        ]);
        let requests = transport.requests.clone();
        let client = GeminiClient::new("key", transport);
        let card = RecallCard::new(
            crate::generation::manga::ShownRecall::new(
                "Russian",
                "Она выглядела испуганной.",
                "испуганной",
                "О сильном страхе.",
            ),
            crate::generation::manga::HiddenRecall::new(
                "English",
                "frightened",
                "She looked frightened.",
            ),
        );
        let mut costs = Vec::new();
        let review = client.review_recall_observed(
            &card,
            &recall_scene(),
            "image/jpeg",
            &[1, 2, 3],
            |cost| {
                costs.push(cost);
                Ok(())
            },
        );
        let payloads = requests
            .borrow()
            .iter()
            .map(|request| serde_json::from_str::<Value>(request))
            .collect::<Result<Vec<_>, _>>()
            .expect("recall requests must decode");
        assert_eq!(
            (
                review.is_ok_and(|review| review.allows()),
                payloads.len(),
                payloads[0]["generationConfig"]["maxOutputTokens"].as_u64(),
                payloads[1]["generationConfig"]["maxOutputTokens"].as_u64(),
                payloads[0]["contents"][0]["parts"][1]["inlineData"]["data"].as_str(),
                payloads[1]["contents"][0]["parts"][1]["inlineData"]["data"].as_str(),
                costs.iter().map(CostRecord::requests).collect::<Vec<_>>(),
                costs.iter().map(|cost| cost.cost().nanos()).sum::<u64>(),
            ),
            (
                true,
                2,
                Some(256),
                Some(512),
                Some("AQID"),
                Some("AQID"),
                vec![1, 1],
                1_040_000,
            ),
            "MAX_TOKENS recovery changed the image, exceeded two reviews, or hid billed usage"
        );
    }

    #[test]
    fn persistent_max_tokens_recall_review_stops_after_one_adaptive_retry() {
        let response = || {
            body(json!({
                "candidates": [{
                    "content": {"parts": [{"text": "{\"decision\":\"ALLOW\""}]},
                    "finishReason": "MAX_TOKENS"
                }],
                "usageMetadata": {
                    "promptTokenCount": 500,
                    "candidatesTokenCount": 256,
                    "thoughtsTokenCount": 0,
                    "totalTokenCount": 756
                }
            }))
        };
        let transport = FakeTransport::new(vec![response(), response()]);
        let requests = transport.requests.clone();
        let client = GeminiClient::new("key", transport);
        let card = RecallCard::new(
            crate::generation::manga::ShownRecall::new(
                "Russian",
                "Она выглядела испуганной.",
                "испуганной",
                "О сильном страхе.",
            ),
            crate::generation::manga::HiddenRecall::new(
                "English",
                "frightened",
                "She looked frightened.",
            ),
        );
        let mut costs = Vec::new();
        let result = client.review_recall_observed(
            &card,
            &recall_scene(),
            "image/jpeg",
            &[1, 2, 3],
            |cost| {
                costs.push(cost);
                Ok(())
            },
        );
        assert_eq!(
            (result.is_err(), requests.borrow().len(), costs.len()),
            (true, 2, 2),
            "persistent MAX_TOKENS escaped the single adaptive review retry"
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
                ThinkingLevel::Minimal,
                FEATURE_MAX_OUTPUT_TOKENS,
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
        let transport =
            FakeTransport::new(vec![text_body(&features), text_body(&semantic_scene())]);
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
        assert_eq!(
            (
                costs.len(),
                bodies.len(),
                (
                    bodies[0]
                        .pointer("/generationConfig/responseFormat/text/mimeType")
                        .and_then(Value::as_str),
                    bodies[0]
                        .pointer("/generationConfig/responseMimeType")
                        .is_none(),
                    bodies[0].pointer("/generationConfig/responseFormat/text/schema")
                        == Some(&feature_schema),
                    bodies[0]
                        .pointer("/generationConfig/thinkingConfig/thinkingLevel")
                        .and_then(Value::as_str),
                    bodies[0]
                        .pointer("/generationConfig/maxOutputTokens")
                        .and_then(Value::as_u64),
                    bodies[1]
                        .pointer("/generationConfig/responseMimeType")
                        .and_then(Value::as_str),
                    bodies[1]
                        .pointer("/generationConfig/responseFormat")
                        .is_none(),
                    bodies[1]
                        .pointer("/generationConfig/thinkingConfig/thinkingLevel")
                        .and_then(Value::as_str),
                    bodies[1]
                        .pointer("/generationConfig/maxOutputTokens")
                        .and_then(Value::as_u64),
                ),
                prompts[0].contains("reliability")
                    && !prompts[0].contains("splash-1-v1")
                    && !prompts[0].contains("LAYOUT REGISTRY"),
                prompts[1].contains("\"chosen_template_id\": \"splash-1-v1\""),
                !prompts[1].contains("\"bounds\"") && !prompts[1].contains("\"polygon\""),
                scene
                    .pointer("/manga_panel/meta/layout_selection/chosen_template_id")
                    .and_then(Value::as_str),
                scene
                    .pointer("/manga_panel/page_design/layout/template_id")
                    .and_then(Value::as_str),
            ),
            (
                2,
                2,
                (
                    Some("APPLICATION_JSON"),
                    true,
                    true,
                    Some("MINIMAL"),
                    Some(u64::from(FEATURE_MAX_OUTPUT_TOKENS)),
                    Some("application/json"),
                    true,
                    Some("LOW"),
                    Some(u64::from(COMPOSER_MAX_OUTPUT_TOKENS)),
                ),
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
        let cases = vec![
            vec![text_body(&invalid)],
            vec![text_body(&valid), text_body(&json!({}))],
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
            [(1, true, 1, 1), (2, true, 2, 2)],
            "registry scene failures discarded completed-stage costs or called a later stage"
        );
    }

    #[test]
    fn observer_failure_stops_the_scene_pipeline_before_the_next_request() {
        let transport = FakeTransport::new(vec![text_body(&json!({})), text_body(&json!({}))]);
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
