//! Adapts Gemini to the scene, image, speech, and recall ports.

use anyhow::{Result, bail};

use super::cost_accounting::CostRecorder;
use crate::gemini::{GeminiClient, HttpTransport};
use crate::generation::manga::{ImageSource, RecallCard, RecallJudge, RecallReview};
use crate::generation::{SceneSource, Speaker};

#[derive(Clone)]
/// Adapts metered Gemini calls to scene, image, and speech ports.
pub(super) struct MeteredGemini {
    client: GeminiClient<HttpTransport>,
    costs: CostRecorder,
}

impl MeteredGemini {
    /// Bind a Gemini client to one artifact cost recorder.
    pub(super) fn new(client: GeminiClient<HttpTransport>, costs: CostRecorder) -> Self {
        Self { client, costs }
    }
}

impl SceneSource for MeteredGemini {
    fn scene(
        &self,
        language: &str,
        term: &str,
        sentence: &str,
        target: &str,
        attempt: u8,
    ) -> Result<serde_json::Value> {
        self.client
            .scene_observed(language, term, sentence, target, attempt, |cost| {
                self.costs.push(cost)
            })
    }
}

impl ImageSource for MeteredGemini {
    fn image(&self, prompt: &str) -> Result<Vec<u8>> {
        self.client
            .image_observed(prompt, |cost| self.costs.push(cost))
    }
}

impl Speaker for MeteredGemini {
    fn speech(&self, prompt: &str, text: &str) -> Result<Vec<u8>> {
        self.client
            .speech_observed(prompt, text, |cost| self.costs.push(cost))
    }
}

#[derive(Clone)]
/// Uses Gemini to judge whether an image preserves the recall contract.
pub(super) struct GeminiRecall {
    client: GeminiClient<HttpTransport>,
    card: RecallCard,
    costs: CostRecorder,
}

impl GeminiRecall {
    /// Bind Gemini, the recall contract, and its picture cost recorder.
    pub(super) fn new(
        client: GeminiClient<HttpTransport>,
        card: RecallCard,
        costs: CostRecorder,
    ) -> Self {
        Self {
            client,
            card,
            costs,
        }
    }
}

impl RecallJudge for GeminiRecall {
    fn review(&self, image: &[u8]) -> Result<RecallReview> {
        self.client
            .review_recall_observed(&self.card, image_mime(image)?, image, |cost| {
                self.costs.push(cost)
            })
    }
}

fn image_mime(image: &[u8]) -> Result<&'static str> {
    match image::guess_format(image)? {
        image::ImageFormat::Jpeg => Ok("image/jpeg"),
        image::ImageFormat::Png => Ok("image/png"),
        image::ImageFormat::WebP => Ok("image/webp"),
        image::ImageFormat::Gif => Ok("image/gif"),
        format => bail!("unsupported recall-review image format {format:?}"),
    }
}
