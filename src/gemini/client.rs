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
use crate::languages::{LanguageCatalog, LanguageCode, catalog};
use crate::session::{
    ARTIFACT_ATTEMPT_CEILING, AxisSet, CardDraft, CardMeta, CardRevision, CostRecord, LanguagePair,
    LearningGuess, RawInputBatch, Register, Sense, SenseCorrection, SentenceAxis, SentenceKind,
    SentenceLabelSelection, SentenceLabels, SentenceLevel, Understood, WordCandidate,
};

use super::codec::{decode, encode};
use super::cost::priced;
use super::prompts::{
    render_bulk_prompt, render_card_meta_prompt, render_card_prompt, render_intake_prompt,
    render_phonetics_prompt,
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

const TEXT_MODEL: &str = "gemini-3.8-flash";
const META_MODEL: &str = TEXT_MODEL;
const FEATURE_MODEL: &str = TEXT_MODEL;
const SCENE_MODEL: &str = TEXT_MODEL;
const IMAGE_MODEL: &str = "gemini-3.1-flash-image";
const RECALL_MODEL: &str = TEXT_MODEL;
const FIDELITY_MODEL: &str = TEXT_MODEL;
const LITERAL_ZOOM_MODEL: &str = TEXT_MODEL;
const TEXT_JUDGE_MODEL: &str = TEXT_MODEL;
const TTS_MODEL: &str = "gemini-3.1-flash-tts-preview";
/// Output ceiling for one intake chunk.
///
/// Twenty words of the worst-case polysemous shape bill about 11.4k tokens, so
/// this leaves room to spare. It is deliberately low enough that reaching it
/// takes well under the transport timeout, which is what turns a truncated
/// reply into a named refusal instead of a hang.
const INTAKE_MAX_OUTPUT_TOKENS: u32 = 16_384;
const FEATURE_MAX_OUTPUT_TOKENS: u32 = 4_096;
const COMPOSER_MAX_OUTPUT_TOKENS: u32 = 8_192;
const RECALL_MAX_OUTPUT_TOKENS: u32 = 1024;
const RECALL_RECOVERY_MAX_OUTPUT_TOKENS: u32 = 4096;
const FIDELITY_MAX_OUTPUT_TOKENS: u32 = 2048;
const TEXT_JUDGE_MAX_OUTPUT_TOKENS: u32 = 1024;
const MAX_ALTERNATES: usize = 3;
const TEXT_JUDGE_RECOVERY_MAX_OUTPUT_TOKENS: u32 = 512;
const LITERAL_ZOOM_MAX_OUTPUT_TOKENS: u32 = 2048;
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
                ThinkingLevel::Low,
                FEATURE_MAX_OUTPUT_TOKENS,
                &mut observe,
            )
            .context("scene feature extraction request failed")?;
        let lenient = attempt.saturating_add(1) >= ARTIFACT_ATTEMPT_CEILING;
        let features = registry.decode_features_lenient(unfence(feature_raw.trim()), lenient)?;
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
        compose(composer_raw.as_str(), sentence, target, &selection, lenient)
            .map_err(|error| error.context(RejectedReply::new("scene composer", composer_raw)))
    }

    /// Send one free-form prompt to a text model and return the raw textual response.
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
            serde_json::from_str(unfence(self.intake_text(prompt)?.trim()))?;
        let guess = match target {
            LearningTarget::Detect => {
                let alternates = supported_alternates(
                    decoded.alternates.as_slice(),
                    decoded.target_lang.as_str(),
                    &catalog,
                );
                LearningGuess::new(decoded.target_lang, true).with_alternates(alternates)
            }
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

    /// Build card metadata and reviewed pronunciation for the draft's selected meaning.
    pub fn generate_draft_meta(
        &self,
        draft: &CardDraft,
        request: Option<&SentenceLabelSelection>,
    ) -> Result<CardMeta> {
        self.generate_draft_meta_metered(draft, request)
            .map(|(meta, _)| meta)
    }

    /// Refine only the supplied card's term and sentence pronunciation in its selected meaning.
    pub fn refine_pronunciation(&self, draft: &CardDraft, meta: CardMeta) -> Result<CardMeta> {
        self.phonetics_observed(draft, meta, draft.pair(), &mut |_| Ok(()))
    }

    /// Build rich card meta and return the request cost record.
    pub(crate) fn generate_card_meta_metered(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
        request: Option<&SentenceLabelSelection>,
    ) -> Result<(CardMeta, CostRecord)> {
        let draft = CardDraft::new(term, understanding, pair.clone());
        self.generate_draft_meta_metered(&draft, request)
    }

    fn generate_draft_meta_metered(
        &self,
        draft: &CardDraft,
        request: Option<&SentenceLabelSelection>,
    ) -> Result<(CardMeta, CostRecord)> {
        let mut costs = Vec::new();
        let meta = self.generate_draft_meta_observed(draft, request, |cost| {
            costs.push(cost);
            Ok(())
        })?;
        let cost = CostRecord::aggregate(&costs)
            .ok_or_else(|| anyhow!("Successful card generation did not report request usage"))?;
        Ok((meta, cost))
    }

    /// Build contextual rich card meta and report usage before local JSON decoding.
    pub(crate) fn generate_draft_meta_observed<F>(
        &self,
        draft: &CardDraft,
        request: Option<&SentenceLabelSelection>,
        mut observe: F,
    ) -> Result<CardMeta>
    where
        F: FnMut(CostRecord) -> Result<()>,
    {
        let catalog = catalog();
        let prompt = render_card_meta_prompt(draft, request, &catalog)?;
        let raw = self.card_text_observed(prompt, &mut observe)?;
        let senses = prioritized_senses(draft);
        let meta = card_meta_from_raw(raw.as_str(), request, &senses)?;
        self.phonetics_observed(draft, meta, draft.pair(), &mut observe)
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
        let mut costs = Vec::new();
        let revision = self.correct_card_observed(draft, comment, pair, |cost| {
            costs.push(cost);
            Ok(())
        })?;
        let cost = CostRecord::aggregate(&costs)
            .ok_or_else(|| anyhow!("Successful card correction did not report request usage"))?;
        Ok((revision, cost))
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
        let raw = self.card_text_observed(prompt, &mut observe)?;
        let decoded: CardCorrectionResponse = serde_json::from_str(unfence(raw.trim()))?;
        let revision = contextual_revision(draft, decoded.into_revision(label_selection(draft))?)?;
        let settled = draft.clone().with_revision(revision.clone(), None);
        let (term, understanding, meta) = revision.into_parts();
        let meta = self.phonetics_observed(&settled, meta, pair, &mut observe)?;
        Ok(CardRevision::new(term, understanding, meta))
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
                GenerationConfig::vision_judge(schema.clone())?
                    .with_thinking_level(ThinkingLevel::Low)
                    .with_max_output_tokens(tokens),
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
                .with_thinking_level(ThinkingLevel::Low)
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
                .with_thinking_level(ThinkingLevel::Low)
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
                GenerationConfig::vision_judge(schema.clone())?
                    .with_thinking_level(ThinkingLevel::Low)
                    .with_max_output_tokens(tokens),
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

    /// Run the batch-wide intake prompt under an explicit output ceiling.
    ///
    /// Kept apart from `text_metered` on purpose: that helper is shared with the
    /// free-form completion path, whose request bytes are frozen by contract.
    fn intake_text(&self, prompt: String) -> Result<String> {
        let request = Request::text(
            prompt,
            Some(GenerationConfig::bounded_text(INTAKE_MAX_OUTPUT_TOKENS)),
            None,
        );
        let metered = self.request_metered(TEXT_MODEL, &request)?;
        if metered.response.finish_reason() == Some("MAX_TOKENS") {
            bail!(
                "Gemini understanding hit the {}-token output ceiling; retry to resume from the words already understood",
                INTAKE_MAX_OUTPUT_TOKENS
            );
        }
        let raw = response_text(&metered.response);
        if raw.trim().is_empty() {
            bail!("No text content in Gemini understanding response");
        }
        Ok(raw)
    }

    fn text_metered(&self, model: &str, prompt: String) -> Result<(String, CostRecord)> {
        let metered = self.request_metered(model, &Request::text(prompt, None, None))?;
        let raw = response_text(&metered.response);
        if raw.trim().is_empty() {
            bail!("No text content in Gemini response");
        }
        Ok((raw, metered.cost))
    }

    fn card_text_observed<F>(&self, prompt: String, observe: &mut F) -> Result<String>
    where
        F: FnMut(CostRecord) -> Result<()>,
    {
        let metered = self.request_metered(
            META_MODEL,
            &Request::text(
                prompt,
                Some(GenerationConfig::json_mode().with_thinking_level(ThinkingLevel::High)),
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

    fn phonetics_observed<F>(
        &self,
        draft: &CardDraft,
        meta: CardMeta,
        pair: &LanguagePair,
        observe: &mut F,
    ) -> Result<CardMeta>
    where
        F: FnMut(CostRecord) -> Result<()>,
    {
        let prompt = render_phonetics_prompt(draft, &meta, pair)?;
        let metered = self.request_metered(
            META_MODEL,
            &Request::text(
                prompt,
                Some(GenerationConfig::json_mode().with_thinking_level(ThinkingLevel::Medium)),
                None,
            ),
        )?;
        observe(metered.cost.clone())?;
        let raw = response_text(&metered.response);
        if raw.trim().is_empty() {
            bail!("No text content in Gemini phonetics response");
        }
        let decoded: PhoneticsResponse = serde_json::from_str(unfence(raw.trim()))?;
        decoded.into_meta(meta)
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

fn card_meta_from_raw(
    raw: &str,
    request: Option<&SentenceLabelSelection>,
    senses: &[Sense],
) -> Result<CardMeta> {
    let decoded: CardMetaResponse = serde_json::from_str(unfence(raw.trim()))?;
    contextual_meta(decoded.into_meta(request)?, senses)
}

fn prioritized_senses(draft: &CardDraft) -> Vec<Sense> {
    let mut senses = draft
        .reviewed_senses()
        .iter()
        .enumerate()
        .collect::<Vec<_>>();
    senses.sort_by_key(|(index, _)| (*index != 0, draft.sense_priority(*index)));
    senses.into_iter().map(|(_, sense)| sense.clone()).collect()
}

fn contextual_revision(draft: &CardDraft, revision: CardRevision) -> Result<CardRevision> {
    let senses = draft
        .clone()
        .with_revision(revision.clone(), None)
        .reviewed_senses()
        .to_vec();
    let (term, understanding, meta) = revision.into_parts();
    let meta = if draft.term() == term {
        contextual_meta(meta, senses.as_slice())?
    } else {
        let context = reviewed_context(meta.source_context(), senses.as_slice())?;
        meta.with_source_context(context)
    };
    Ok(CardRevision::new(term, understanding, meta))
}

fn contextual_meta(meta: CardMeta, senses: &[Sense]) -> Result<CardMeta> {
    if senses.len() < 2 {
        return Ok(meta);
    }
    let context = reviewed_context(meta.source_context(), senses)?;
    Ok(meta.with_source_context(context))
}

fn reviewed_context(source_context: &str, senses: &[Sense]) -> Result<String> {
    let sections = context_sections(source_context);
    if sections.len() != 4
        || sections.iter().any(|section| {
            let mut lines = section.lines();
            !lines.next().is_some_and(context_header) || !lines.any(|line| !line.trim().is_empty())
        })
    {
        bail!("reviewed source context must contain exactly four nonempty headed sections");
    }
    let mut lines = sections[0].lines();
    let header = lines
        .next()
        .ok_or_else(|| anyhow!("reviewed source context must start with a bold header"))?;
    let mut first = Vec::with_capacity(senses.len().min(crate::session::MAX_CARD_MEANINGS) + 1);
    first.push(String::from(header));
    for (index, sense) in senses
        .iter()
        .take(crate::session::MAX_CARD_MEANINGS)
        .enumerate()
    {
        let text = reviewed_sense_text(sense);
        if index == 0 {
            first.push(format!("- **{text}**"));
        } else {
            first.push(format!("- {text}"));
        }
    }
    Ok(format!(
        "{}\n\n{}",
        first.join("\n"),
        sections[1..].join("\n\n")
    ))
}

fn reviewed_sense_text(sense: &Sense) -> String {
    let understanding = escaped_inline(one_line(sense.understanding()).as_str());
    match sense.tag() {
        Some(tag) => format!(
            "[{}] {understanding}",
            escaped_inline(one_line(tag).as_str())
        ),
        None => understanding,
    }
}

fn context_sections(source_context: &str) -> Vec<String> {
    let normalized = source_context.replace("\r\n", "\n").replace('\r', "\n");
    let mut sections = Vec::new();
    let mut section = Vec::new();
    for line in normalized.lines() {
        let trimmed = line.trim();
        if context_header(trimmed) {
            if !section.is_empty() {
                sections.push(section.join("\n").trim().to_string());
                section.clear();
            }
            section.push(trimmed);
        } else if !section.is_empty() {
            section.push(line.trim_end());
        } else if !trimmed.is_empty() {
            section.push(trimmed);
        }
    }
    if !section.is_empty() {
        sections.push(section.join("\n").trim().to_string());
    }
    sections
}

fn context_header(line: &str) -> bool {
    line.strip_prefix("**")
        .and_then(|text| text.strip_suffix("**"))
        .is_some_and(|text| !text.trim().is_empty() && !context_bullet(text))
}

fn context_bullet(line: &str) -> bool {
    let text = line
        .strip_prefix("**")
        .and_then(|text| text.strip_suffix("**"))
        .unwrap_or(line)
        .trim();
    let marker = text.strip_prefix(['-', '*', '•', '+']).or_else(|| {
        let numbered = text.trim_start_matches(|character: char| character.is_ascii_digit());
        (numbered.len() < text.len())
            .then(|| numbered.strip_prefix(['.', ')']))
            .flatten()
    });
    marker.is_some_and(|rest| rest.starts_with(char::is_whitespace))
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn escaped_inline(value: &str) -> String {
    value.replace('\\', "\\\\").replace('*', "\\*")
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
    #[serde(default)]
    alternates: Vec<String>,
    items: Vec<IntakeItem>,
}

/// Keep only alternates the app can actually switch to.
///
/// The model is asked for supported codes, most plausible first, but it answers
/// in free text: it can name a language the catalog does not carry, repeat
/// itself, or hand back the target it just chose. Anything the picker could not
/// honour is dropped rather than shown as an offer the app cannot keep.
fn supported_alternates(
    reported: &[String],
    target: &str,
    catalog: &LanguageCatalog,
) -> Vec<String> {
    let chosen = catalog.resolve(target).ok();
    let mut kept: Vec<String> = Vec::new();
    for code in reported {
        let Ok(resolved) = catalog.resolve(code.as_str()) else {
            continue;
        };
        let resolved = resolved.to_string();
        if chosen
            .as_ref()
            .is_some_and(|target| LanguageCode::as_ref(target) == resolved)
            || kept.contains(&resolved)
        {
            continue;
        }
        kept.push(resolved);
        if kept.len() == MAX_ALTERNATES {
            break;
        }
    }
    kept
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
struct PhoneticsResponse {
    pronunciation: String,
    transcription: String,
}

impl PhoneticsResponse {
    fn into_meta(self, meta: CardMeta) -> Result<CardMeta> {
        if self.pronunciation.trim().is_empty() || self.transcription.trim().is_empty() {
            bail!(
                "Gemini phonetics response must contain nonempty pronunciation and transcription"
            );
        }
        Ok(meta.with_phonetics(self.pronunciation, self.transcription))
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
        validate_source_hint(self.source_hint.as_str())?;
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
        validate_source_hint(self.source_hint.as_str())?;
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
        .with_sentence_labels(labels)
        .marked_rewritten();
        Ok(CardRevision::new(self.term, self.understanding, meta))
    }
}

fn validate_source_hint(source_hint: &str) -> Result<()> {
    if source_hint.contains("<near target word>") {
        bail!("source hint contains unresolved <near target word> placeholder");
    }
    Ok(())
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

    use ratatui::style::Modifier;
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

    fn phonetics_body(value: &Value) -> Result<TransportResponse> {
        body(json!({
            "candidates": [{"content": {"parts": [{"text": serde_json::to_string(&json!({
                "pronunciation": value["pronunciation"],
                "transcription": value["transcription"],
            }))?}]}}],
            "usageMetadata": {
                "promptTokenCount": 7,
                "candidatesTokenCount": 3,
                "thoughtsTokenCount": 11,
                "totalTokenCount": 21
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

    fn contextual_test_meta(source_context: &str) -> CardMeta {
        CardMeta::new(
            "ka.naʁ",
            "lə ka.naʁ naʒ",
            "a duck",
            5,
            "The duck swims",
            "duck",
            "Think of a pond",
            source_context,
            "Le canard nage",
        )
    }

    #[derive(Clone, Copy)]
    struct ReviewedCorpusRow {
        term: &'static str,
        learning: &'static str,
        senses: [(&'static str, Option<&'static str>); 6],
    }

    const REVIEWED_CORPUS: [ReviewedCorpusRow; 20] = [
        ReviewedCorpusRow {
            term: "light",
            learning: "en",
            senses: [
                ("illumination **without glare**", Some("physics *optics*")),
                ("not heavy", None),
                ("pale in colour", Some(r"design\palette")),
                ("not serious\n- comic in tone", Some("register")),
                ("to ignite a flame", Some("verb")),
                ("a traffic signal", Some("transport")),
            ],
        },
        ReviewedCorpusRow {
            term: "bank",
            learning: "en",
            senses: [
                ("a financial institution", Some("finance")),
                ("the edge of a river", Some("landscape")),
                ("to tilt an aircraft", Some("aviation")),
                ("a stored mass or reserve", None),
                ("to rely on someone", Some("informal")),
                ("a row of switches <and> controls", Some("engineering")),
            ],
        },
        ReviewedCorpusRow {
            term: "run",
            learning: "en",
            senses: [
                ("to move quickly on foot", None),
                ("to operate a machine", Some("verb")),
                ("to manage an organisation", Some("business")),
                ("to flow continuously", Some("liquid")),
                ("to extend across an area", Some("shape")),
                ("to compete for office", Some("politics")),
            ],
        },
        ReviewedCorpusRow {
            term: "set",
            learning: "en",
            senses: [
                ("to put something in place", Some("verb")),
                ("a collection of related things", Some("noun")),
                ("to become firm or solid", Some("material")),
                ("fixed or established", Some("adjective")),
                ("to sink below the horizon", Some("sun")),
                ("a television receiver", Some("device")),
            ],
        },
        ReviewedCorpusRow {
            term: "spring",
            learning: "en",
            senses: [
                ("the season after winter", Some("time")),
                ("a coiled elastic device", Some("mechanics")),
                ("to leap suddenly", Some("verb")),
                ("a natural source of water", Some("landscape")),
                ("to arise or originate", Some("figurative")),
                ("elastic energy or bounce", None),
            ],
        },
        ReviewedCorpusRow {
            term: "point",
            learning: "en",
            senses: [
                ("a precise location", Some("place")),
                ("the main idea of an argument", None),
                ("a unit in a score", Some("games")),
                ("a narrow headland", Some("geography")),
                ("the sharp end of an object", Some("shape")),
                ("to indicate with a finger", Some("verb")),
            ],
        },
        ReviewedCorpusRow {
            term: "match",
            learning: "en",
            senses: [
                ("a sporting contest", Some("sport")),
                ("a small stick for making fire", None),
                ("a suitable romantic partner", Some("relationship")),
                ("an equal in ability", Some("comparison")),
                ("to correspond in colour or form", Some("verb")),
                ("to pair two compatible things", Some("action")),
            ],
        },
        ReviewedCorpusRow {
            term: "file",
            learning: "en",
            senses: [
                ("a digital collection of data", Some("computing")),
                ("a folder of documents", Some("office")),
                ("a rough tool for smoothing", Some("tool")),
                ("to submit an official document", Some("law")),
                ("a line of people one behind another", None),
                ("to arrange papers systematically", Some("verb")),
            ],
        },
        ReviewedCorpusRow {
            term: "draft",
            learning: "en",
            senses: [
                ("a preliminary version of a text", Some("writing")),
                ("a current of cool air", None),
                ("compulsory military selection", Some("military")),
                ("beer served from a cask", Some("drink")),
                ("to select a player for a team", Some("sport")),
                ("the depth of a vessel below water", Some("nautical")),
            ],
        },
        ReviewedCorpusRow {
            term: "scale",
            learning: "en",
            senses: [
                ("a system of measurement", None),
                ("a device for weighing", Some("instrument")),
                ("to climb a steep surface", Some("verb")),
                ("one of the plates on a fish", Some("biology")),
                ("an ordered sequence of musical notes", Some("music")),
                ("to resize proportionally", Some("computing")),
            ],
        },
        ReviewedCorpusRow {
            term: "canard",
            learning: "fr",
            senses: [
                ("a duck", Some("animal")),
                ("a false report", Some("journalism")),
                ("an unfounded rumour", None),
                ("duck meat as food", Some("cooking")),
                ("a small forewing on an aircraft", Some("aviation")),
                ("a deliberately misleading story", Some("figurative")),
            ],
        },
        ReviewedCorpusRow {
            term: "feuille",
            learning: "fr",
            senses: [
                ("a leaf of a plant", Some("botany")),
                ("a sheet of paper", None),
                ("a newspaper", Some("press")),
                ("a thin layer of pastry", Some("cooking")),
                ("a worksheet", Some("school")),
                ("a very thin film of material", Some("technical")),
            ],
        },
        ReviewedCorpusRow {
            term: "banco",
            learning: "es",
            senses: [
                ("a financial bank", Some("finance")),
                ("a bench to sit on", None),
                ("a shoal of fish", Some("biology")),
                ("a workbench", Some("workshop")),
                ("a counter in a shop", Some("commerce")),
                ("a reserve of stored material", Some("technical")),
            ],
        },
        ReviewedCorpusRow {
            term: "cola",
            learning: "es",
            senses: [
                ("an animal's tail", Some("anatomy")),
                ("a queue of people", None),
                ("glue or adhesive", Some("material")),
                ("a cola soft drink", Some("drink")),
                ("the rear end of a train", Some("transport")),
                ("the tail of a comet", Some("astronomy")),
            ],
        },
        ReviewedCorpusRow {
            term: "Schloss",
            learning: "de",
            senses: [
                ("a castle or palace", Some("building")),
                ("a lock on a door", None),
                ("a clasp on jewellery", Some("object")),
                ("the action of a firearm", Some("technical")),
                ("a fastening mechanism", Some("engineering")),
                ("a concluding closure", Some("figurative")),
            ],
        },
        ReviewedCorpusRow {
            term: "Zug",
            learning: "de",
            senses: [
                ("a railway train", Some("transport")),
                ("a pulling force", Some("physics")),
                ("a move in a board game", Some("games")),
                ("a current of air", None),
                ("a procession of people", Some("group")),
                ("a distinctive character trait", Some("figurative")),
            ],
        },
        ReviewedCorpusRow {
            term: "мир",
            learning: "ru",
            senses: [
                ("peace rather than war", None),
                ("the world", Some("general")),
                ("a community or social sphere", Some("society")),
                ("harmony between people", Some("relationship")),
                ("secular life outside a monastery", Some("historical")),
                ("a traditional village assembly", Some("history")),
            ],
        },
        ReviewedCorpusRow {
            term: "ключ",
            learning: "ru",
            senses: [
                ("a key for a lock", None),
                ("a natural spring of water", Some("landscape")),
                ("a clue for solving a problem", Some("figurative")),
                ("a wrench or spanner", Some("tool")),
                ("a cryptographic key", Some("computing")),
                ("the key idea of an explanation", Some("abstract")),
            ],
        },
        ReviewedCorpusRow {
            term: "はし",
            learning: "ja",
            senses: [
                ("chopsticks written 箸", Some("object")),
                ("a bridge written 橋", Some("place")),
                ("an edge written 端", Some("position")),
                ("a ladder written 梯子 in compounds", Some("reading")),
                ("the end of a long object", Some("position")),
                (
                    "a word distinguished mainly by pitch accent",
                    Some("pronunciation"),
                ),
            ],
        },
        ReviewedCorpusRow {
            term: "行",
            learning: "zh",
            senses: [
                ("to go, pronounced xíng", Some("verb")),
                ("acceptable or okay, pronounced xíng", Some("spoken")),
                ("a profession, pronounced háng", Some("noun")),
                ("a row or line, pronounced háng", None),
                ("capable or competent", Some("adjective")),
                ("conduct or behaviour in compounds", Some("formal")),
            ],
        },
    ];

    fn one_line(value: &str) -> String {
        value.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn display_sense(sense: &Sense) -> String {
        match sense.tag() {
            Some(tag) => format!("[{}] {}", one_line(tag), one_line(sense.understanding())),
            None => one_line(sense.understanding()),
        }
    }

    fn corpus_source_context(variant: usize) -> String {
        let gap = if variant.is_multiple_of(3) {
            "\n\n"
        } else {
            "\n"
        };
        let relationship = if variant.is_multiple_of(5) {
            r"relationship: a *surface* contrast can appear beside C:\usage."
        } else {
            "relationship: the readings differ by concrete use, grammar, or domain."
        };
        let value = format!(
            "**Meaning.**\n- omitted model paraphrase\n- another model paraphrase{gap}{relationship}\n\n**Where you'll hear it.**\nIn ordinary conversations and the named specialist domain.\n\n**Where it's out of place.**\nChoose the narrower word when ambiguity would be costly.\n\n**Subtlety.**\nContext decides the intended reading."
        );
        if variant % 2 == 1 {
            value.replace('\n', "\r\n")
        } else {
            value
        }
    }

    fn bullet_shape(block: &crate::markdown::Block) -> Option<(String, bool, bool)> {
        let crate::markdown::Block::Bullet { chunks, .. } = block else {
            return None;
        };
        Some((
            chunks.iter().map(|chunk| chunk.text.as_str()).collect(),
            !chunks.is_empty() && chunks.iter().all(|chunk| chunk.bold),
            chunks.iter().any(|chunk| chunk.italic),
        ))
    }

    fn corpus_candidate(row: ReviewedCorpusRow) -> Result<WordCandidate> {
        let response = json!({
            "target_lang": row.learning,
            "items": [{
                "term": row.term,
                "senses": row.senses.iter().map(|(understanding, tag)| json!({
                    "understanding": understanding,
                    "tag": tag,
                })).collect::<Vec<_>>(),
                "selected": 0,
                "ok": true,
            }],
        });
        let client = GeminiClient::new("key", FakeTransport::new(vec![text_body(&response)]));
        let target = LearningTarget::Explicit(catalog().resolve(row.learning)?);
        let understood = client.understand(&RawInputBatch::new(row.term), "en", &target)?;
        understood
            .candidates()
            .first()
            .cloned()
            .ok_or_else(|| anyhow!("corpus intake returned no candidate for '{}'", row.term))
    }

    fn corpus_case_failures(row_index: usize, selected: usize) -> Vec<String> {
        let row = REVIEWED_CORPUS[row_index];
        let candidate = match corpus_candidate(row) {
            Ok(candidate) => candidate,
            Err(error) => return vec![format!("raw intake failed: {error}")],
        };
        let draft =
            CardDraft::from_candidate(&candidate, selected, LanguagePair::new(row.learning, "en"));
        let expected = std::iter::once(candidate.senses()[selected].clone())
            .chain(
                candidate
                    .senses()
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| *index != selected)
                    .map(|(_, sense)| sense.clone()),
            )
            .collect::<Vec<_>>();
        let mut failures = Vec::new();
        if draft.reviewed_senses() != expected.as_slice() {
            failures.push(String::from(
                "draft order or tags differ from What I Understood",
            ));
        }
        let mut response = card_meta_response(sentence_labels_response(
            "neutral",
            "b1",
            "statement",
            Vec::new(),
        ));
        response["source_context"] = json!(corpus_source_context(row_index * 6 + selected));
        let transport = FakeTransport::new(vec![text_body(&response), phonetics_body(&response)]);
        let requests = transport.requests.clone();
        let client = GeminiClient::new("key", transport);
        let context = match client.generate_draft_meta_observed(&draft, None, |_| Ok(())) {
            Ok(meta) => meta.source_context().to_string(),
            Err(error) => {
                failures.push(format!("contextual metadata failed: {error}"));
                return failures;
            }
        };
        if phonetic_input(&requests.borrow()[1])["reviewed_senses"]
            != json!(
                expected
                    .iter()
                    .map(|sense| json!({
                        "understanding": sense.understanding(),
                        "tag": sense.tag(),
                    }))
                    .collect::<Vec<_>>()
            )
        {
            failures.push(String::from(
                "IPA refinement lost a meaning outside the visible five",
            ));
        }
        let blocks = crate::markdown::parse_markdown(context.as_str());
        let bullets = blocks.iter().filter_map(bullet_shape).collect::<Vec<_>>();
        let expected_text = expected
            .iter()
            .take(crate::session::MAX_CARD_MEANINGS)
            .map(display_sense)
            .collect::<Vec<_>>();
        let actual_text = bullets
            .iter()
            .map(|(text, _, _)| text.clone())
            .collect::<Vec<_>>();
        if actual_text != expected_text {
            failures.push(format!(
                "markdown bullets differ: expected {expected_text:?}, got {actual_text:?}"
            ));
        }
        if bullets
            .first()
            .is_none_or(|(_, bold, italic)| !bold || *italic)
        {
            failures.push(String::from(
                "chosen meaning is not wholly bold and non-italic",
            ));
        }
        if bullets
            .iter()
            .skip(1)
            .any(|(_, bold, italic)| *bold || *italic)
        {
            failures.push(String::from("an alternative meaning acquired emphasis"));
        }
        let plain = crate::markdown::to_plain(blocks.as_slice());
        let plain_bullets = plain
            .lines()
            .filter_map(|line| line.strip_prefix("- "))
            .map(String::from)
            .collect::<Vec<_>>();
        if plain_bullets != expected_text {
            failures.push(String::from(
                "plain projection lost exact visible meaning text",
            ));
        }
        if blocks.iter().any(|block| match block {
            crate::markdown::Block::Paragraph(chunks)
            | crate::markdown::Block::Bullet { chunks, .. } => {
                chunks.iter().any(|chunk| chunk.italic)
            }
        }) {
            failures.push(String::from(
                "literal model or sense stars became italic markup",
            ));
        }
        if plain.contains("relationship:") {
            failures.push(String::from(
                "the glossary retained redundant relationship prose",
            ));
        }
        let html = crate::markdown::to_html(blocks.as_slice());
        let html_list = html
            .split_once("<ul")
            .and_then(|(_, tail)| tail.split_once("</ul>"))
            .map(|(list, _)| list)
            .unwrap_or_default();
        if html_list.matches("<li>").count() != crate::session::MAX_CARD_MEANINGS
            || html_list.matches("<strong>").count() != 1
        {
            failures.push(String::from(
                "Anki HTML changed the meaning count or emphasis",
            ));
        }
        let tui = crate::markdown::to_ratatui(blocks.as_slice());
        let tui_bullets = tui
            .iter()
            .filter(|line| {
                line.spans
                    .first()
                    .is_some_and(|span| span.content.as_ref().ends_with("• "))
            })
            .collect::<Vec<_>>();
        if tui_bullets.len() != crate::session::MAX_CARD_MEANINGS
            || tui_bullets.first().is_none_or(|line| {
                line.spans
                    .iter()
                    .skip(1)
                    .any(|span| !span.style.add_modifier.contains(Modifier::BOLD))
            })
            || tui_bullets.iter().skip(1).any(|line| {
                line.spans.iter().skip(1).any(|span| {
                    span.style
                        .add_modifier
                        .intersects(Modifier::BOLD | Modifier::ITALIC)
                })
            })
        {
            failures.push(String::from("TUI changed the meaning count or emphasis"));
        }
        failures
    }

    #[test]
    fn one_hundred_twenty_raw_to_card_context_paths_keep_order_tags_and_projections() {
        let failures = REVIEWED_CORPUS
            .iter()
            .enumerate()
            .flat_map(|(row, _)| {
                (0..6).flat_map(move |selected| {
                    corpus_case_failures(row, selected)
                        .into_iter()
                        .map(move |failure| {
                            format!("case {} selected {selected}: {failure}", row + 1)
                        })
                })
            })
            .collect::<Vec<_>>();
        assert!(
            failures.is_empty(),
            "120-case raw-to-card context corpus found defects:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn card_paths_cannot_reject_an_ordinary_article_absent_from_the_known_sentence() {
        let labels = sentence_labels_response("neutral", "a1", "statement", Vec::new());
        let mut metadata = card_meta_response(labels.clone());
        let mut correction = card_correction_response(labels);
        for response in [&mut metadata, &mut correction] {
            response["source_sentence"] = json!("We are going to the beach this weekend.");
            response["source_highlight"] = json!("to");
            response["source_hint"] = json!("Walking with towels toward the ocean on a sunny day.");
            response["target_sentence"] = json!("Vamos a la playa este fin de semana.");
        }
        correction["term"] = json!("a");
        let client = GeminiClient::new(
            "key",
            FakeTransport::new(vec![
                text_body(&metadata),
                phonetics_body(&metadata),
                text_body(&metadata),
                phonetics_body(&metadata),
                text_body(&correction),
                phonetics_body(&correction),
                text_body(&correction),
                phonetics_body(&correction),
            ]),
        );
        let pair = LanguagePair::new("es", "en");
        let draft = CardDraft::new("a", "toward a destination", pair.clone());
        let mut costs = Vec::new();
        let metered = client.generate_draft_meta_metered(&draft, None);
        let observed = client.generate_draft_meta_observed(&draft, None, |cost| {
            costs.push(cost);
            Ok(())
        });
        let corrected = client.correct_card_metered(&draft, "Keep this sense", &pair);
        let corrected_observed =
            client.correct_card_observed(&draft, "Keep this sense", &pair, |cost| {
                costs.push(cost);
                Ok(())
            });
        assert_eq!(
            (
                metered.is_ok(),
                observed.is_ok(),
                corrected.is_ok(),
                corrected_observed.is_ok(),
                costs.len()
            ),
            (true, true, true, true, 4),
            "an ordinary English article caused a false rejection of Spanish destination metadata"
        );
    }

    #[test]
    fn card_paths_cannot_reject_an_article_already_visible_in_the_known_sentence() {
        let labels = sentence_labels_response("neutral", "a1", "statement", Vec::new());
        let mut metadata = card_meta_response(labels.clone());
        let mut correction = card_correction_response(labels);
        for response in [&mut metadata, &mut correction] {
            response["source_sentence"] = json!("He has a black cat.");
            response["source_highlight"] = json!("has");
            response["source_hint"] = json!("Someone possesses a playful pet companion at home.");
            response["target_sentence"] = json!("Il a un chat noir.");
        }
        correction["term"] = json!("a");
        let client = GeminiClient::new(
            "key",
            FakeTransport::new(vec![
                text_body(&metadata),
                phonetics_body(&metadata),
                text_body(&metadata),
                phonetics_body(&metadata),
                text_body(&correction),
                phonetics_body(&correction),
                text_body(&correction),
                phonetics_body(&correction),
            ]),
        );
        let pair = LanguagePair::new("fr", "en");
        let draft = CardDraft::new("a", "has", pair.clone());
        let mut costs = Vec::new();
        let metered = client.generate_draft_meta_metered(&draft, None);
        let observed = client.generate_draft_meta_observed(&draft, None, |cost| {
            costs.push(cost);
            Ok(())
        });
        let corrected = client.correct_card_metered(&draft, "Keep this sense", &pair);
        let corrected_observed =
            client.correct_card_observed(&draft, "Keep this sense", &pair, |cost| {
                costs.push(cost);
                Ok(())
            });
        assert_eq!(
            (
                metered.is_ok(),
                observed.is_ok(),
                corrected.is_ok(),
                corrected_observed.is_ok(),
                costs.len()
            ),
            (true, true, true, true, 4),
            "an ordinary article already visible on the known-language face caused a false hint rejection"
        );
    }

    #[test]
    fn phonetic_generation_preserves_every_nonphonetic_field_and_label_provenance() {
        let request = SentenceLabelSelection::empty()
            .choosing(SentenceAxis::Level, 2)
            .choosing(SentenceAxis::Type, 1);
        let response = card_meta_response(sentence_labels_response(
            "neutral",
            "b2",
            "statement",
            vec!["level", "type"],
        ));
        let mut expected = serde_json::to_value(
            serde_json::from_value::<CardMetaResponse>(response.clone())
                .expect("metadata fixture must decode")
                .into_meta(Some(&request))
                .expect("metadata fixture must validate"),
        )
        .expect("metadata fixture must serialize");
        expected["pronunciation"] = json!("kanaʁ");
        expected["transcription"] = json!("lə kanaʁ naʒ");
        let client = GeminiClient::new(
            "key",
            FakeTransport::new(vec![text_body(&response), phonetics_body(&expected)]),
        );
        let draft = CardDraft::new("canard", "a duck", LanguagePair::new("fr", "en"));
        let actual = client
            .generate_draft_meta_metered(&draft, Some(&request))
            .expect("two-pass metadata must succeed");
        assert_eq!(
            (
                serde_json::to_value(actual.0).expect("metadata must serialize"),
                serde_json::to_value(actual.1).expect("cost must serialize")
            ),
            (
                expected,
                json!({"model":"gemini-3.8-flash","requests":2,"input_tokens":107,"output_tokens":64,"total_tokens":171,"cost":{"nanos":320250}})
            ),
            "the IPA pass changed nonphonetic fields, lost label provenance, or miscounted one response"
        );
    }

    #[test]
    fn phonetic_correction_audits_the_settled_identity_without_old_siblings() {
        let pair = LanguagePair::new("fr", "en");
        let candidate = WordCandidate::with_senses(
            "canard",
            vec![
                Sense::tagged("a duck", "animal"),
                Sense::tagged("a false report", "journalism"),
            ],
            0,
            true,
        );
        let draft = CardDraft::from_candidate(&candidate, 0, pair.clone());
        let mut response = card_correction_response(sentence_labels_response(
            "neutral",
            "b1",
            "statement",
            Vec::new(),
        ));
        response["term"] = json!("oie");
        response["understanding"] = json!("a goose");
        response["target_sentence"] = json!("Une oie nage");
        response["source_context"] = json!(
            "**Meaning.**\n- old bird\n\n**Usage.**\nAt a pond.\n\n**Pattern.**\nAn example.\n\n**Nuance.**\nA pairing."
        );
        let expected = contextual_revision(
            &draft,
            serde_json::from_value::<CardCorrectionResponse>(response.clone())
                .expect("correction fixture must decode")
                .into_revision(label_selection(&draft))
                .expect("correction fixture must validate"),
        )
        .expect("correction fixture context must validate");
        let mut expected_meta =
            serde_json::to_value(expected.meta()).expect("expected meta must serialize");
        expected_meta["pronunciation"] = json!("wa");
        expected_meta["transcription"] = json!("yn wa naʒ");
        let transport =
            FakeTransport::new(vec![text_body(&response), phonetics_body(&expected_meta)]);
        let requests = transport.requests.clone();
        let client = GeminiClient::new("key", transport);
        let actual = client
            .correct_card_observed(&draft, "Use the goose term", &pair, |_| Ok(()))
            .expect("two-pass correction must succeed");
        let input = requests
            .borrow()
            .get(1)
            .map(|request| phonetic_input(request));
        assert_eq!(
            (
                actual.term(),
                actual.understanding(),
                serde_json::to_value(actual.meta()).expect("actual meta must serialize"),
                input
            ),
            (
                "oie",
                "a goose",
                expected_meta,
                Some(json!({
                    "target_language":"fr", "term":"oie", "reviewed_senses":[{"understanding":"a goose","tag":null}],
                    "selected":0, "target_sentence":"Une oie nage", "pronunciation":"ka.naʁ", "transcription":"lə ka.naʁ naʒ"
                }))
            ),
            "the IPA pass inherited the obsolete term, senses, or tag, or changed nonphonetic correction fields"
        );
    }

    fn phonetic_input(request: &str) -> Value {
        let request: Value = serde_json::from_str(request).expect("IPA request must decode");
        let prompt = request["contents"][0]["parts"][0]["text"]
            .as_str()
            .expect("IPA prompt must be text");
        let start = prompt
            .find('{')
            .expect("IPA prompt must contain input JSON");
        serde_json::Deserializer::from_str(&prompt[start..])
            .into_iter::<Value>()
            .next()
            .expect("IPA input must exist")
            .expect("IPA input must decode")
    }

    #[test]
    fn phonetic_invalid_second_responses_keep_both_observed_charges() {
        let mut outcomes = Vec::new();
        for correction in [false, true] {
            for invalid in [
                json!({}),
                json!({"pronunciation":"p","transcription":"t","meaning":"changed"}),
                json!({"pronunciation":" ","transcription":"t"}),
                json!({"pronunciation":"p","transcription":"\n"}),
                json!({"pronunciation":7,"transcription":"t"}),
            ] {
                let labels = sentence_labels_response("neutral", "b1", "statement", Vec::new());
                let first = if correction {
                    card_correction_response(labels)
                } else {
                    card_meta_response(labels)
                };
                let transport = FakeTransport::new(vec![text_body(&first), text_body(&invalid)]);
                let requests = transport.requests.clone();
                let client = GeminiClient::new("key", transport);
                let pair = LanguagePair::new("fr", "en");
                let draft = CardDraft::new("canard", "a duck", pair.clone());
                let mut costs = Vec::new();
                let mut observe = |cost| {
                    costs.push(cost);
                    Ok(())
                };
                let failed = if correction {
                    client
                        .correct_card_observed(&draft, "Keep it", &pair, &mut observe)
                        .is_err()
                } else {
                    client
                        .generate_draft_meta_observed(&draft, None, &mut observe)
                        .is_err()
                };
                outcomes.push((
                    failed,
                    requests.borrow().len(),
                    costs.iter().map(CostRecord::requests).collect::<Vec<_>>(),
                ));
            }
        }
        assert!(
            outcomes
                .iter()
                .all(|outcome| outcome == &(true, 2, vec![1, 1])),
            "an invalid IPA response was accepted or discarded an observed charge: {outcomes:?}"
        );
    }

    #[test]
    fn phonetic_observer_failure_stops_before_any_later_request() {
        let mut outcomes = Vec::new();
        for stop in [1, 2] {
            let response = card_meta_response(sentence_labels_response(
                "neutral",
                "b1",
                "statement",
                Vec::new(),
            ));
            let transport =
                FakeTransport::new(vec![text_body(&response), phonetics_body(&response)]);
            let requests = transport.requests.clone();
            let client = GeminiClient::new("key", transport);
            let draft = CardDraft::new("canard", "a duck", LanguagePair::new("fr", "en"));
            let mut observed = 0;
            let result = client.generate_draft_meta_observed(&draft, None, |_| {
                observed += 1;
                if observed == stop {
                    return Err(anyhow!("observer write failed"));
                }
                Ok(())
            });
            outcomes.push((result.is_err(), observed, requests.borrow().len()));
        }
        assert_eq!(
            outcomes,
            vec![(true, 1, 1), (true, 2, 2)],
            "an observer failure allowed a later paid request or a successful card"
        );
    }

    #[test]
    fn phonetic_first_decode_failure_cannot_spend_a_second_request() {
        let transport = FakeTransport::new(vec![text_body(&json!({"invalid":"metadata"}))]);
        let requests = transport.requests.clone();
        let client = GeminiClient::new("key", transport);
        let draft = CardDraft::new("canard", "a duck", LanguagePair::new("fr", "en"));
        let mut costs = Vec::new();
        let result = client.generate_draft_meta_observed(&draft, None, |cost| {
            costs.push(cost);
            Ok(())
        });
        assert_eq!(
            (result.is_err(), requests.borrow().len(), costs.len()),
            (true, 1, 1),
            "invalid first metadata spent an IPA request or lost its observed cost"
        );
    }

    #[test]
    fn phonetic_transport_failure_preserves_only_usage_actually_returned() {
        let response = card_meta_response(sentence_labels_response(
            "neutral",
            "b1",
            "statement",
            Vec::new(),
        ));
        let transport = FakeTransport::new(vec![
            text_body(&response),
            Err(anyhow!("IPA transport failed")),
        ]);
        let requests = transport.requests.clone();
        let client = GeminiClient::new("key", transport);
        let draft = CardDraft::new("canard", "a duck", LanguagePair::new("fr", "en"));
        let mut costs = Vec::new();
        let result = client.generate_draft_meta_observed(&draft, None, |cost| {
            costs.push(cost);
            Ok(())
        });
        assert_eq!(
            (result.is_err(), requests.borrow().len(), costs.len()),
            (true, 2, 1),
            "an IPA transport failure returned success or fabricated/discarded usage"
        );
    }

    #[test]
    fn phonetic_card_paths_use_author_high_audit_medium_and_exact_costs() {
        let labels = sentence_labels_response("neutral", "b1", "statement", Vec::new());
        let metadata = card_meta_response(labels.clone());
        let correction = card_correction_response(labels);
        let transport = FakeTransport::new(vec![
            text_body(&metadata),
            phonetics_body(&metadata),
            text_body(&metadata),
            phonetics_body(&metadata),
            text_body(&correction),
            phonetics_body(&correction),
            text_body(&correction),
            phonetics_body(&correction),
            text_body(&json!("plain response")),
            text_body(&json!("intake response")),
        ]);
        let requests = transport.requests.clone();
        let urls = transport.urls.clone();
        let client = GeminiClient::new("key", transport);
        let pair = LanguagePair::new("fr", "en");
        let draft = CardDraft::new("canard", "a duck", pair.clone());
        let mut costs = Vec::new();
        let results = [
            client
                .generate_draft_meta_metered(&draft, None)
                .map(|(_, cost)| costs.push(cost))
                .is_ok(),
            client
                .generate_draft_meta_observed(&draft, None, |cost| {
                    costs.push(cost);
                    Ok(())
                })
                .is_ok(),
            client
                .correct_card_metered(&draft, "Keep this sense", &pair)
                .map(|(_, cost)| costs.push(cost))
                .is_ok(),
            client
                .correct_card_observed(&draft, "Keep this sense", &pair, |cost| {
                    costs.push(cost);
                    Ok(())
                })
                .is_ok(),
            client
                .complete(TEXT_MODEL, String::from("Freeform prompt"))
                .is_ok(),
            client.intake_text(String::from("Intake prompt")).is_ok(),
        ];
        let configs = requests
            .borrow()
            .iter()
            .map(|request| {
                serde_json::from_str::<Value>(request)
                        .expect("captured text request must decode")["generationConfig"]
                        .clone()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            (
                results,
                configs,
                urls.borrow()
                    .iter()
                    .all(|url| url.ends_with("/gemini-3.8-flash:generateContent")),
                costs
                    .iter()
                    .map(|cost| (cost.model(), cost.requests(), cost.cost().nanos()))
                    .collect::<Vec<_>>()
            ),
            (
                [true; 6],
                vec![
                    json!({"responseMimeType": "application/json", "thinkingConfig": {"thinkingLevel": "HIGH"}}),
                    json!({"responseMimeType": "application/json", "thinkingConfig": {"thinkingLevel": "MEDIUM"}}),
                    json!({"responseMimeType": "application/json", "thinkingConfig": {"thinkingLevel": "HIGH"}}),
                    json!({"responseMimeType": "application/json", "thinkingConfig": {"thinkingLevel": "MEDIUM"}}),
                    json!({"responseMimeType": "application/json", "thinkingConfig": {"thinkingLevel": "HIGH"}}),
                    json!({"responseMimeType": "application/json", "thinkingConfig": {"thinkingLevel": "MEDIUM"}}),
                    json!({"responseMimeType": "application/json", "thinkingConfig": {"thinkingLevel": "HIGH"}}),
                    json!({"responseMimeType": "application/json", "thinkingConfig": {"thinkingLevel": "MEDIUM"}}),
                    Value::Null,
                    json!({"maxOutputTokens": INTAKE_MAX_OUTPUT_TOKENS}),
                ],
                true,
                vec![
                    ("gemini-3.8-flash", 2, 320_250),
                    ("gemini-3.8-flash", 1, 262_500),
                    ("gemini-3.8-flash", 1, 57_750),
                    ("gemini-3.8-flash", 2, 320_250),
                    ("gemini-3.8-flash", 1, 262_500),
                    ("gemini-3.8-flash", 1, 57_750)
                ]
            ),
            "a text path used the wrong model, lost billed cost, or changed its thinking and format contract"
        );
    }

    #[test]
    fn reviewed_context_accepts_a_meaning_list_without_relationship_prose() {
        let senses = [
            Sense::tagged("a false report", "journalism"),
            Sense::plain("a duck"),
        ];
        let tail = "\n\n**Usage.**\nIn newspaper reports.\n\n**Limits.**\nNo special restriction.\n\n**Nuance.**\nNo additional point needs noting.";
        let context = format!("**Meaning.**\n- model paraphrase{tail}");
        assert_eq!(
            reviewed_context(&context, &senses).expect("a list-only glossary must be accepted"),
            format!("**Meaning.**\n- **[journalism] a false report**\n- a duck{tail}"),
            "a complete four-section card required unnecessary relationship prose"
        );
    }

    #[test]
    fn reviewed_context_drops_unreviewed_prose_before_the_next_section() {
        let senses = [Sense::plain("one"), Sense::plain("two")];
        let tail = "\n\n**Usage.**\nIn this situation.\n\n**Limits.**\nNo special restriction.\n\n**Nuance.**\nOne useful explanation: \"one example\" — translation.";
        let additions = [
            "relationship: old explanation.",
            "Unlabelled prose without a colon",
            "a wrapped list continuation\n\nAnother redundant paragraph.",
            "различие: одно значение.\nсходство: другое значение.",
            "違い：一方の意味。",
        ];
        assert!(
            additions.iter().all(|extra| reviewed_context(
                &format!("**Meaning.**\n- a paraphrase\n{extra}{tail}"),
                &senses
            )
            .is_ok_and(|context| context == format!("**Meaning.**\n- **one**\n- two{tail}"))),
            "redundant glossary prose survived or removing it damaged another required section"
        );
    }

    #[test]
    fn reviewed_context_removes_the_old_hindi_relationship_without_changing_usage() {
        let generated = "**अर्थ।**\n- **वित्त. - संज्ञा पैसे जमा करने तथा वित्तीय लेन-देन का आधिकारिक संस्थान।**\n- संज्ञा किसी नदी अथवा जलाशय के किनारे की ढलान वाली ज़मीन।\n- वित्त. - क्रिया पैसे को सुरक्षित रखने के लिए खाते में जमा करना।\nप्रयोग का भेद: वित्तीय संस्थान और नदी का किनारा अलग-अलग संज्ञाएँ हैं, जबकि पैसे जमा करने के संदर्भ में यह क्रिया बन जाती है।\n\n**स्वाभाविक प्रयोग।**\nनया खाता खुलवाने, कर्ज़ लेने या वित्तीय लेन-देन से जुड़ी रोज़मर्रा की बातचीत में यह स्वाभाविक रूप से आता है।\n\n**जहाँ प्रयोग अटपटा लगे।**\nयदि आप केवल सड़क किनारे लगी स्वचालित नकदी मशीन की बात कर रहे हों, तो इसके बजाय ATM कहना अधिक सटीक है।\n\n**भाव और लहजा।**\nकिसी भौतिक शाखा में जाने के लिए 'go to the bank' कहते हैं, लेकिन डिजिटल माध्यम से लेन-देन करने के लिए संज्ञा 'online banking' का प्रयोग होता है।";
        let senses = [Sense::plain("वित्तीय संस्थान"), Sense::plain("नदी का किनारा")];
        assert!(
            reviewed_context(generated, &senses)
                .is_ok_and(|context| !context.contains("प्रयोग का भेद:")
                    && context.ends_with("का प्रयोग होता है।")),
            "removing the old Hindi glossary prose failed or changed the usage sections"
        );
    }

    #[test]
    fn reviewed_context_accepts_the_rejected_bold_list_marker() {
        let generated = "**Значение.**\n**- Гл. Стойко выдерживать тяжелую физическую нагрузку либо боль.**\n- животное: Сущ. Крупное хищное лесное млекопитающее с густой шерстью.\n- Гл. Рождать потомство в естественной среде обитания.\nразграничение: существительное называет хищника, а глагольные значения разделяются по контексту физической стойкости и рождения potomstva.\n\n**Где встречается.**\nВ описаниях травм, тренировок на предел возможностей, медицинских процедур и разговорах о физических нагрузках.\n\n**Где неуместно.**\nНе подходит, когда речь идет о спокойном снисхождении к чужим капризам или взглядам; для этого используют tolerate.\n\n**Нюанс.**\nГлагол неправильный: его основные формы — bore и borne (для физического выдерживания груза или боли).";
        let senses = [
            Sense::plain("выдерживать нагрузку"),
            Sense::plain("медведь"),
        ];
        assert!(
            reviewed_context(generated, &senses)
                .is_ok_and(|context| context.starts_with(
                    "**Значение.**\n- **выдерживать нагрузку**\n- медведь\n\n**Где встречается.**"
                ) && context.ends_with("груза или боли).")),
            "a bold list item still becomes a spurious fifth section"
        );
    }

    #[test]
    fn reviewed_context_removes_relationships_independently_of_their_script() {
        let senses = [Sense::plain("one"), Sense::plain("two")];
        let relationships = [
            "التمييز: يدل الأول على المؤسسة؛ أما الثاني فيدل على ضفة النهر؟",
            "Relationship: the noun names e.g. a bank; the verb describes depositing money.",
            "違い：一方は金融機関を表し、もう一方は川岸を表す。",
            "relationship: the first sense names an animal; the second names a report",
        ];
        assert!(
            relationships.iter().all(|relationship| reviewed_context(&format!("**Meaning.**\n- one\n- two\n{relationship}\n\n**Usage.**\nCommon.\n\n**Limits.**\nRare elsewhere.\n\n**Nuance.**\nContext matters."), &senses).is_ok_and(|context| !context.contains(relationship))),
            "old glossary prose survived because of its script, capitalization, or punctuation"
        );
    }

    #[test]
    fn reviewed_context_cannot_accept_a_missing_or_empty_section() {
        let senses = [Sense::plain("one"), Sense::plain("two")];
        let contexts = [
            "**Meaning.**\n- one\nrelationship: these differ.\n\n**Usage.**\nCommon.\n\n**Nuance.**\nContext matters.",
            "**Meaning.**\n- one\nrelationship: these differ.\n\n**Usage.**\n\n**Limits.**\nRare elsewhere.\n\n**Nuance.**\nContext matters.",
        ];
        assert!(
            contexts
                .iter()
                .all(|context| reviewed_context(context, &senses).is_err()),
            "a card with a missing or empty required section passed structural validation"
        );
    }

    #[test]
    fn a_term_correction_replaces_the_old_sense_list_and_relationship() {
        let draft = CardDraft::from_candidate(
            &WordCandidate::with_senses(
                "bank",
                vec![
                    Sense::plain("financial institution"),
                    Sense::plain("river edge"),
                ],
                0,
                true,
            ),
            0,
            LanguagePair::new("en", "ru"),
        );
        let tail = "\n\n**Где встречается.**\nВ рассказах о поездках к водоёмам, во время прогулок у воды и в описаниях природы.\n\n**Где неуместно.**\nНе подходит для крутого или узкого берега реки — там лучше использовать bank.\n\n**Нюанс.**\nЧасто употребляется с предлогами on и along и обозначает границу воды и суши в целом, а не только песчаную зону отдыха как beach.";
        let generated = format!(
            "**Значение.**\n- financial institution\n- river edge\nrelationship: the original bank meanings differ.{tail}"
        );
        let revised = contextual_revision(
            &draft,
            CardRevision::new(
                "shore",
                "Полоса суши у моря",
                contextual_test_meta(&generated),
            ),
        )
        .expect("a changed-term correction must normalize");
        assert_eq!(
            revised.meta().source_context(),
            format!("**Значение.**\n- **Полоса суши у моря**{tail}"),
            "the new term inherited an old alternative or relationship, or lost the other three sections"
        );
    }

    #[test]
    fn a_term_correction_accepts_the_rejected_shore_singleton_context() {
        let draft = CardDraft::from_candidate(
            &WordCandidate::with_senses(
                "bank",
                vec![
                    Sense::plain("financial institution"),
                    Sense::plain("river edge"),
                ],
                0,
                true,
            ),
            0,
            LanguagePair::new("en", "ru"),
        );
        let generated = "**Значение.**\n- **Сущ. Полоса суши, прилегающая к крупному водоёму (озеру, морю, океану).**\n\n**Где встречается.**\nВ рассказах о поездках к водоёмам, во время прогулок у воды и в описаниях природы.\n\n**Где неуместно.**\nНе подходит для крутого или узкого берега реки — там лучше использовать bank.\n\n**Нюанс.**\nЧасто употребляется с предлогами on и along и обозначает границу воды и суши в целом, а не только песчаную зону отдыха как beach.";
        assert!(
            contextual_revision(
                &draft,
                CardRevision::new(
                    "shore",
                    "Полоса суши у моря",
                    contextual_test_meta(generated)
                )
            )
            .is_ok_and(|revision| revision
                .meta()
                .source_context()
                .starts_with("**Значение.**\n- **Полоса суши у моря**\n\n**Где встречается.**")),
            "a new singleton term still requires an obsolete relationship between the original term's meanings"
        );
    }

    #[test]
    fn reviewed_context_cannot_include_a_sixth_meaning() {
        let senses = [
            Sense::tagged("the chosen rare use", "specialist"),
            Sense::plain("the most common use"),
            Sense::plain("the next practical use"),
            Sense::tagged("a widespread informal use", "casual"),
            Sense::plain("another frequent use"),
            Sense::tagged("a lower priority alternative", "historical"),
        ];
        let generated = "**Meaning.**\n- model paraphrase\n\n**Usage.**\nA concrete situation.\n\n**Limits.**\nA supported boundary.\n\n**Nuance.**\nOne useful point.";
        assert_eq!(
            reviewed_context(generated, &senses).expect("a complete context must normalize"),
            "**Meaning.**\n- **[specialist] the chosen rare use**\n- the most common use\n- the next practical use\n- [casual] a widespread informal use\n- another frequent use\n\n**Usage.**\nA concrete situation.\n\n**Limits.**\nA supported boundary.\n\n**Nuance.**\nOne useful point.",
            "the glossary exceeded five meanings or lost the chosen use, priority, tags or guidance"
        );
    }

    #[test]
    fn metadata_recreation_cannot_promote_a_previously_selected_legacy_alternative() {
        let draft = CardDraft::new("canard", "fifth priority", LanguagePair::new("fr", "en"))
            .with_reviewed_senses(vec![
                Sense::plain("fifth priority"),
                Sense::tagged("sixth priority", "rare"),
                Sense::plain("first priority"),
                Sense::plain("second priority"),
                Sense::plain("third priority"),
                Sense::plain("fourth priority"),
            ])
            .with_sense_priorities(vec![4, 5, 0, 1, 2, 3]);
        let mut response = card_meta_response(sentence_labels_response(
            "neutral",
            "b1",
            "statement",
            Vec::new(),
        ));
        response["source_context"] = json!(
            "**Meaning.**\n- model paraphrase\n\n**Usage.**\nA concrete situation.\n\n**Limits.**\nA supported boundary.\n\n**Nuance.**\nOne useful point."
        );
        let client = GeminiClient::new(
            "key",
            FakeTransport::new(vec![text_body(&response), phonetics_body(&response)]),
        );
        let meta = client
            .generate_draft_meta_observed(&draft, None, |_| Ok(()))
            .expect("metadata recreation must preserve the restored priority");
        assert_eq!(
            meta.source_context(),
            "**Meaning.**\n- **fifth priority**\n- first priority\n- second priority\n- third priority\n- fourth priority\n\n**Usage.**\nA concrete situation.\n\n**Limits.**\nA supported boundary.\n\n**Nuance.**\nOne useful point.",
            "legacy cache order overrode the recovered priority during metadata recreation"
        );
    }

    #[test]
    fn correcting_to_a_hidden_meaning_restores_priority_in_the_card_and_full_ipa_inventory() {
        let pair = LanguagePair::new("fr", "en");
        let candidate = WordCandidate::with_senses(
            "canard",
            vec![
                Sense::plain("first priority"),
                Sense::plain("second priority"),
                Sense::plain("third priority"),
                Sense::plain("fourth priority"),
                Sense::tagged("fifth priority", "specialist"),
                Sense::tagged("sixth priority", "rare"),
            ],
            5,
            true,
        );
        let draft = CardDraft::from_candidate(&candidate, 5, pair.clone());
        let mut response = card_correction_response(sentence_labels_response(
            "neutral",
            "b1",
            "statement",
            Vec::new(),
        ));
        response["term"] = json!("canard");
        response["understanding"] = json!("fifth priority");
        response["source_context"] = json!(
            "**Meaning.**\n- model paraphrase\n\n**Usage.**\nA concrete situation.\n\n**Limits.**\nA supported boundary.\n\n**Nuance.**\nOne useful point."
        );
        let transport = FakeTransport::new(vec![text_body(&response), phonetics_body(&response)]);
        let requests = transport.requests.clone();
        let client = GeminiClient::new("key", transport);
        let revision = client
            .correct_card_observed(&draft, "Use the fifth priority", &pair, |_| Ok(()))
            .expect("a correction to an omitted reviewed sense must succeed");
        assert_eq!(
            (
                revision.meta().source_context(),
                phonetic_input(&requests.borrow()[1])["reviewed_senses"].clone()
            ),
            (
                "**Meaning.**\n- **[specialist] fifth priority**\n- first priority\n- second priority\n- third priority\n- fourth priority\n\n**Usage.**\nA concrete situation.\n\n**Limits.**\nA supported boundary.\n\n**Nuance.**\nOne useful point.",
                json!([
                    {"understanding":"fifth priority","tag":"specialist"},
                    {"understanding":"first priority","tag":null},
                    {"understanding":"second priority","tag":null},
                    {"understanding":"third priority","tag":null},
                    {"understanding":"fourth priority","tag":null},
                    {"understanding":"sixth priority","tag":"rare"},
                ]),
            ),
            "a hidden selected sense lost its tag, the rare previous choice displaced a priority alternative, or IPA lost part of the inventory"
        );
    }

    #[test]
    fn reviewed_context_replaces_model_bullets_with_the_exact_selected_first_list() {
        let senses = vec![
            Sense::tagged("a false report", "journalism"),
            Sense::plain("a duck"),
            Sense::plain("an unfounded rumour"),
        ];
        let generated = "**Meaning.**\n- a model paraphrase\n- an omitted sense\nrelationship: the report use is figurative; the bird use is concrete.\n\n**Where you'll hear it.**\nNewsrooms.\n\n**Where it's out of place.**\nUse another word at the pond.\n\n**Subtlety.**\nMind the register.";
        assert_eq!(
            reviewed_context(generated, senses.as_slice())
                .expect("reviewed meaning context must normalize"),
            "**Meaning.**\n- **[journalism] a false report**\n- a duck\n- an unfounded rumour\n\n**Where you'll hear it.**\nNewsrooms.\n\n**Where it's out of place.**\nUse another word at the pond.\n\n**Subtlety.**\nMind the register.",
            "locally normalized source context lost a reviewed meaning, its order, tag, or selected emphasis"
        );
    }

    #[test]
    fn reviewed_context_removes_bullet_continuations_and_unlabelled_prose() {
        let senses = [Sense::plain("a duck"), Sense::plain("a false report")];
        let tail = "\n\n**Where you'll hear it.**\nNewsrooms.\n\n**Where it's out of place.**\nAt the pond.\n\n**Subtlety.**\nMind the article.";
        let cases = [
            format!(
                "**Meaning.**\n- a duck with a deliberately long explanation\nthat wraps onto this unmarked continuation line.{tail}"
            ),
            format!("**Meaning.**\n- a duck\nthe senses differ in current use.{tail}"),
            format!(
                "**Meaning.**\n- a duck\nrelationship: one is an animal.\ncontrast: the other is a report.{tail}"
            ),
        ];
        assert!(
            cases.iter().all(|case| reviewed_context(case, &senses)
                .is_ok_and(|context| context
                    == format!("**Meaning.**\n- **a duck**\n- a false report{tail}"))),
            "unreviewed glossary continuations were retained or rejected instead of removed"
        );
    }

    #[test]
    fn a_correction_to_an_existing_alternative_promotes_it_without_duplication() {
        let candidate = WordCandidate::with_senses(
            "canard",
            vec![
                Sense::tagged("a duck", "animal"),
                Sense::tagged("a false report", "journalism"),
                Sense::plain("a newspaper hoax"),
            ],
            0,
            true,
        );
        let draft = CardDraft::from_candidate(&candidate, 0, LanguagePair::new("fr", "en"));
        let generated = "**Meaning.**\n- model list\nrelationship: the news uses are related while the animal use is concrete.\n\n**Where you'll hear it.**\nNewsrooms.\n\n**Where it's out of place.**\nAt the pond.\n\n**Subtlety.**\nMind the article.";
        let revision = contextual_revision(
            &draft,
            CardRevision::new(
                "canard",
                "a newspaper hoax",
                contextual_test_meta(generated),
            ),
        )
        .expect("contextual correction must normalize");
        let settled = draft.with_revision(revision.clone(), None);
        assert_eq!(
            (revision.meta().source_context(), settled.reviewed_senses(),),
            (
                "**Meaning.**\n- **a newspaper hoax**\n- [animal] a duck\n- [journalism] a false report\n\n**Where you'll hear it.**\nNewsrooms.\n\n**Where it's out of place.**\nAt the pond.\n\n**Subtlety.**\nMind the article.",
                &[
                    Sense::plain("a newspaper hoax"),
                    Sense::tagged("a duck", "animal"),
                    Sense::tagged("a false report", "journalism"),
                ] as &[Sense],
            ),
            "a correction duplicated the promoted alternative or desynchronized metadata from the settled draft"
        );
    }

    #[test]
    fn contextual_meta_request_sends_every_reviewed_sense_and_normalizes_the_reply() {
        let labels = sentence_labels_response("neutral", "b1", "statement", Vec::new());
        let mut response = card_meta_response(labels);
        response["source_context"] = json!(
            "**Meaning.**\n- paraphrased bird\nrelationship: the news use is figurative while the bird is concrete.\n\n**Where you'll hear it.**\nNewsrooms and ponds.\n\n**Where it's out of place.**\nChoose the precise noun in formal copy.\n\n**Subtlety.**\nThe article determines the reading."
        );
        let transport = FakeTransport::new(vec![text_body(&response), phonetics_body(&response)]);
        let requests = transport.requests.clone();
        let client = GeminiClient::new("key", transport);
        let candidate = WordCandidate::with_senses(
            "canard",
            vec![
                Sense::plain("a duck"),
                Sense::tagged("a false report", "journalism"),
                Sense::plain("an unfounded rumour"),
            ],
            1,
            true,
        );
        let draft = CardDraft::from_candidate(&candidate, 1, LanguagePair::new("fr", "en"));
        let mut costs = Vec::new();
        let meta = client
            .generate_draft_meta_observed(&draft, None, |cost| {
                costs.push(cost);
                Ok(())
            })
            .expect("contextual metadata must decode");
        let payload = serde_json::from_str::<Value>(&requests.borrow()[0])
            .expect("contextual metadata request must decode");
        let prompt = payload["contents"][0]["parts"][0]["text"]
            .as_str()
            .expect("contextual metadata prompt must be text");
        let reviewed = serde_json::to_string_pretty(&json!([
            {"chosen": true, "priority": 1, "understanding": "a false report", "tag": "journalism"},
            {"chosen": false, "priority": 0, "understanding": "a duck", "tag": null},
            {"chosen": false, "priority": 2, "understanding": "an unfounded rumour", "tag": null},
        ]))
        .expect("reviewed metadata expectation must encode");
        assert!(
            prompt.matches(reviewed.as_str()).count() == 1
                && prompt.contains("\"chosen\": true")
                && meta.source_context().starts_with(
                    "**Meaning.**\n- **[journalism] a false report**\n- a duck\n- an unfounded rumour\n\n**Where you'll hear it.**"
                )
                && phonetic_input(&requests.borrow()[1])["reviewed_senses"] == json!([
                    {"understanding":"a false report", "tag":"journalism"},
                    {"understanding":"a duck", "tag":null},
                    {"understanding":"an unfounded rumour", "tag":null},
                ])
                && costs.len() == 2,
            "contextual metadata request or normalized reply lost the complete reviewed-sense contract"
        );
    }

    #[test]
    fn card_metadata_rejects_the_hint_placeholder_without_rejecting_angle_brackets() {
        let labels = sentence_labels_response("neutral", "b1", "statement", Vec::new());
        let mut generated_placeholder = card_meta_response(labels.clone());
        generated_placeholder["source_hint"] =
            json!("„<near target word>“ ukazuje zisk; tady jde o užitek.");
        let mut corrected_placeholder = card_correction_response(labels.clone());
        corrected_placeholder["source_hint"] =
            json!("„<near target word>“ ukazuje zisk; tady jde o užitek.");
        let mut generated_angle_brackets = card_meta_response(labels.clone());
        generated_angle_brackets["source_hint"] = json!("Choose x < y and y > z.");
        let mut corrected_angle_brackets = card_correction_response(labels);
        corrected_angle_brackets["source_hint"] = json!("Choose x < y and y > z.");
        assert_eq!(
            (
                serde_json::from_value::<CardMetaResponse>(generated_placeholder)
                    .expect("placeholder metadata must decode before semantic validation")
                    .into_meta(None)
                    .is_err(),
                serde_json::from_value::<CardCorrectionResponse>(corrected_placeholder)
                    .expect("placeholder correction must decode before semantic validation")
                    .into_revision(preserved_selection())
                    .is_err(),
                serde_json::from_value::<CardMetaResponse>(generated_angle_brackets)
                    .expect("angle-bracket metadata must decode")
                    .into_meta(None)
                    .is_ok(),
                serde_json::from_value::<CardCorrectionResponse>(corrected_angle_brackets)
                    .expect("angle-bracket correction must decode")
                    .into_revision(preserved_selection())
                    .is_ok(),
            ),
            (true, true, true, true),
            "card metadata retained an unresolved hint placeholder or rejected legitimate angle brackets"
        );
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
        let transport = FakeTransport::new(vec![
            body(json!({
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
            })),
            phonetics_body(&json!({"pronunciation":"waʊnd", "transcription":"aɪ waʊnd ðə klɒk"})),
        ]);
        let client = GeminiClient::new("key", transport);
        let draft = CardDraft::new("wound", "noun sense", LanguagePair::new("en", "ru"));
        let (_revision, cost) = client
            .correct_card_metered(&draft, "make it a verb", &LanguagePair::new("en", "ru"))
            .expect("card correction must decode");
        assert_eq!(
            cost.cost().nanos(),
            320_250,
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
            (true, Some(1), Some(262_500)),
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
                Some("gemini-3.8-flash"),
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
                    == Some("LOW"),
                payload["generationConfig"]["maxOutputTokens"].as_u64() == Some(2048),
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
                urls.borrow()[0].ends_with("/gemini-3.8-flash:generateContent"),
                costs.first().map(CostRecord::model) == Some("gemini-3.8-flash"),
                costs.first().map(|cost| cost.cost().nanos()) == Some(262_500),
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
                urls.borrow()[0].ends_with("/gemini-3.8-flash:generateContent"),
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
                    == Some("LOW"),
                payload["generationConfig"]["mediaResolution"].as_str()
                    == Some("MEDIA_RESOLUTION_HIGH"),
                payload["generationConfig"]["temperature"].as_u64() == Some(0),
                payload["generationConfig"]["maxOutputTokens"].as_u64() == Some(2048),
                costs.len() == 1
                    && costs.first().map(CostRecord::model) == Some("gemini-3.8-flash"),
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
            (true, Some(1), Some("gemini-3.8-flash"), Some(687_000),),
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
                Some(1024),
                Some(4096),
                Some("AQID"),
                Some("AQID"),
                vec![1, 1],
                1_860_000,
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
                ThinkingLevel::Low,
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
                Some(262_500),
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
                    Some("LOW"),
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
