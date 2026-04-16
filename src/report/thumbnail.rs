use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use image::codecs::jpeg::JpegEncoder;

/// Resize one image to the target thumbnail size.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Thumbnail {
    pixels: u32,
}

impl Thumbnail {
    /// Create one thumbnail compressor.
    pub fn new(pixels: u32) -> Self {
        Self { pixels }
    }

    /// Return the compressed JPEG thumbnail path.
    pub fn compressed(&self, source: &Path, directory: &Path) -> Result<PathBuf> {
        let image = image::open(source)?.thumbnail(self.pixels, self.pixels);
        let result = directory.join(format!(
            "thumb_{}",
            source
                .file_name()
                .ok_or_else(|| anyhow!("Image path '{}' has no filename", source.display()))?
                .to_string_lossy()
        ));
        let writer = fs::File::create(&result)?;
        let mut encoder = JpegEncoder::new_with_quality(writer, 60);
        encoder.encode_image(&image)?;
        Ok(result)
    }
}
