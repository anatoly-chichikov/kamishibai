//! Audio caching and WAV persistence.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use hound::{SampleFormat, WavSpec, WavWriter};

use crate::cache::FileCache;

/// Generate raw PCM speech bytes for one prompt.
pub trait Speaker {
    /// Return one PCM audio payload for the prompt and source text.
    fn speech(&self, prompt: &str, text: &str) -> Result<Vec<u8>>;
}

/// Cached audio generator with WAV persistence.
#[derive(Clone, Debug)]
pub struct Audio<C, S> {
    cache: C,
    prompt: String,
    speaker: S,
}

impl<C, S> Audio<C, S>
where
    C: FileCache,
    S: Speaker,
{
    /// Create one cached audio generator.
    pub fn new(cache: C, prompt: impl Into<String>, speaker: S) -> Self {
        Self {
            cache,
            prompt: prompt.into(),
            speaker,
        }
    }

    /// Return the absolute path for one cached filename.
    pub fn filepath(&self, filename: &str) -> Result<PathBuf> {
        self.cache.filepath(filename)
    }

    /// Generate one cached WAV file and report its filename and cache state.
    pub fn generate(&self, text: &str) -> Result<(String, bool)> {
        if text.trim().is_empty() {
            bail!("Cannot generate audio for empty text");
        }
        let filename = format!(
            "{}.wav",
            &format!("{:x}", md5::compute(text.as_bytes()))[..12]
        );
        if self.cache.exists(&filename) {
            return Ok((filename, true));
        }
        let data = self
            .speaker
            .speech(self.prompt.replace("{text}", text).as_str(), text)?;
        self.commit(&filename, &data)?;
        Ok((filename, false))
    }

    fn commit(&self, filename: &str, data: &[u8]) -> Result<()> {
        let staged = self.cache.stage(".wav")?;
        let result = write(&staged, data).and_then(|_| self.cache.commit(&staged, filename));
        if result.is_err() {
            let _ = fs::remove_file(&staged);
        }
        result
    }
}

fn write(path: &Path, data: &[u8]) -> Result<()> {
    let mut samples = data.chunks_exact(2);
    let spec = WavSpec {
        channels: 1,
        sample_rate: 24_000,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec)?;
    for item in &mut samples {
        writer.write_sample(i16::from_le_bytes([item[0], item[1]]))?;
    }
    if !samples.remainder().is_empty() {
        bail!("Audio payload is not 16-bit aligned");
    }
    writer.finalize()?;
    Ok(())
}
