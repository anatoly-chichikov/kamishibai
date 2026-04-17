use std::path::Path;

use anyhow::Result;
use image::DynamicImage;

/// Resize one image to the target thumbnail size in memory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Thumbnail {
    pixels: u32,
}

impl Thumbnail {
    /// Create one thumbnail compressor.
    pub fn new(pixels: u32) -> Self {
        Self { pixels }
    }

    /// Return the decoded and downscaled image ready for embedding.
    pub fn scaled(&self, source: &Path) -> Result<DynamicImage> {
        Ok(image::open(source)?.thumbnail(self.pixels, self.pixels))
    }
}
