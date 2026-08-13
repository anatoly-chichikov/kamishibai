use std::path::Path;

use anyhow::Result;
use image::{DynamicImage, GrayImage};
use serde_json::Value;

use super::text_gate::{TextReview, TextReviewGate};

/// Report scene and illustration progress events.
pub trait Progress {
    /// Signal the start of one step.
    fn step(&mut self, name: &str);
    /// Signal the completion of one step.
    fn done(&mut self, name: &str, label: &str, path: Option<&Path>);
    /// Signal one retry within rendering.
    fn retry(&mut self, _name: &str, _attempt: usize, _reason: &str) {}
}

/// Translate one sentence into a scene JSON document.
pub trait Translator {
    /// Return one scene JSON document for the sentence and target language.
    fn translate(&self, sentence: &str, target: &str) -> Result<Value>;
}

/// Render one scene JSON document into an image.
pub trait Renderer {
    /// Return one rendered image for the scene.
    fn render(&self, scene: &Value, progress: &mut dyn Progress) -> Result<DynamicImage>;
}

/// Detect OCR text from one grayscale image.
pub trait ImageText {
    /// Return the detected OCR text for one image.
    fn detected(&self, image: &GrayImage) -> Result<String>;
}

/// Detect OCR text from one scene and grayscale image pair.
pub trait SceneText {
    /// Return the detected OCR text for one scene and image pair.
    fn detected(&self, scene: &Value, image: &GrayImage) -> Result<String>;
}

/// Judge literal writing in one encoded candidate image.
pub trait TextJudge {
    /// Return the route used by this judge.
    fn gate(&self) -> TextReviewGate;
    /// Return the typed literal-writing verdict for one candidate image.
    fn review(&self, encoded: &[u8], grayscale: &GrayImage) -> Result<TextReview>;
}

/// Render one compiled prose prompt into raw image bytes.
pub trait ImageSource {
    /// Return one encoded image payload for the prompt.
    fn image(&self, prompt: &str) -> Result<Vec<u8>>;
}
