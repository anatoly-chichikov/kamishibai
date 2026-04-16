use std::rc::Rc;

use anyhow::{Result, bail};
use image::DynamicImage;
use serde_json::Value;

use super::{BorderDetector, ImageSource, Progress, Renderer, SceneText};

/// Render one scene through Gemini and reject invalid manga images.
#[derive(Clone)]
pub struct MangaRenderer<D> {
    client: Rc<dyn ImageSource>,
    retries: usize,
    text: D,
    border: BorderDetector,
}

impl<D> MangaRenderer<D> {
    /// Create one validating manga renderer.
    pub fn new<C>(client: C, retries: usize, text: D, border: BorderDetector) -> Self
    where
        C: ImageSource + 'static,
    {
        Self {
            client: Rc::new(client),
            retries,
            text,
            border,
        }
    }
}

impl<D> std::fmt::Debug for MangaRenderer<D>
where
    D: std::fmt::Debug,
{
    /// Render one stable debug view for test diagnostics.
    fn fmt(&self, item: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        item.debug_struct("MangaRenderer")
            .field("client", &"ImageSource")
            .field("retries", &self.retries)
            .field("text", &self.text)
            .field("border", &self.border)
            .finish()
    }
}

impl<D> Renderer for MangaRenderer<D>
where
    D: SceneText,
{
    /// Return one rendered image for the scene and word.
    fn render(
        &self,
        scene: &Value,
        word: &str,
        progress: &mut dyn Progress,
    ) -> Result<DynamicImage> {
        let panels = scene
            .get("manga_panel")
            .and_then(|root| root.get("panels"))
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let mut reason = String::new();
        for attempt in 0..self.retries {
            let gray =
                image::load_from_memory(self.client.image(scene, word)?.as_slice())?.into_luma8();
            let found = self.text.detected(scene, &gray)?;
            if !found.is_empty() {
                reason = format!("OCR detected text: '{found}'");
                progress.retry("Rendering manga", attempt + 1, reason.as_str());
                continue;
            }
            let failed = self.border.borders(&gray);
            if !failed.is_empty() {
                reason = format!("White border missing on: {}", failed.join(", "));
                progress.retry("Rendering manga", attempt + 1, reason.as_str());
                continue;
            }
            if panels > 1 && !self.border.gutter(&gray) {
                reason = String::from("No white gutter found");
                progress.retry("Rendering manga", attempt + 1, reason.as_str());
                continue;
            }
            return Ok(DynamicImage::ImageLuma8(gray));
        }
        bail!(
            "Rejected after {} attempts for '{}': {}",
            self.retries,
            word,
            reason
        );
    }
}
