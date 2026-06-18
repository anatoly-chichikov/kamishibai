//! Audio caching and WAV persistence.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::generation::artifact_cache::{Cache, VOICE_FILE};

/// Generate raw PCM speech bytes for one prompt.
pub trait Speaker {
    /// Return one PCM audio payload for the prompt and source text.
    fn speech(&self, prompt: &str, text: &str) -> Result<Vec<u8>>;
}

/// Cached audio generator with WAV persistence.
#[derive(Clone, Debug)]
pub struct Audio<S> {
    cache: Cache,
    prompt: String,
    speaker: S,
}

impl<S> Audio<S>
where
    S: Speaker,
{
    /// Create one cached audio generator.
    pub fn new(cache: Cache, prompt: impl Into<String>, speaker: S) -> Self {
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

    /// Generate the cached WAV for this card folder and report its cache state.
    ///
    /// One audio file belongs to one card folder, so the file is always `audio.wav`;
    /// the cache hit is decided by the folder, not by hashing the text.
    pub fn generate(&self, text: &str) -> Result<(String, bool)> {
        if text.trim().is_empty() {
            bail!("Cannot generate audio for empty text");
        }
        if self.cache.exists(VOICE_FILE) {
            return Ok((VOICE_FILE.to_string(), true));
        }
        let data = self
            .speaker
            .speech(self.prompt.replace("{text}", text).as_str(), text)?;
        self.commit(VOICE_FILE, &data)?;
        Ok((VOICE_FILE.to_string(), false))
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
    if !data.len().is_multiple_of(2) {
        bail!("Audio payload is not 16-bit aligned");
    }
    let data_size = u32::try_from(data.len()).context("Audio payload too large for WAV")?;
    let riff_size = data_size
        .checked_add(36)
        .context("Audio payload too large for WAV")?;
    let mut wav = Vec::with_capacity(44 + data.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_size.to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&24_000u32.to_le_bytes());
    wav.extend_from_slice(&48_000u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    wav.extend_from_slice(data);
    fs::write(path, &wav)?;
    Ok(())
}
