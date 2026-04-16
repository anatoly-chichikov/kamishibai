use std::path::Path;

use anyhow::Result;
use image::{DynamicImage, GrayImage};
use serde_json::Value;

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
    /// Return one rendered image for the scene and word.
    fn render(
        &self,
        scene: &Value,
        word: &str,
        progress: &mut dyn Progress,
    ) -> Result<DynamicImage>;
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

/// Render one scene JSON payload into raw image bytes.
pub trait ImageSource {
    /// Return one encoded image payload for the scene and word.
    fn image(&self, scene: &Value, word: &str) -> Result<Vec<u8>>;
}
