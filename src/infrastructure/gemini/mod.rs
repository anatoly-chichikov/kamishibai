//! Direct REST client for Gemini text, image, and TTS generation.

mod client;
mod codec;
mod protocol;

pub use client::{GeminiClient, HttpTransport, Transport, TransportResponse};

use anyhow::Result;

use crate::application::media::SceneSource;
use crate::infrastructure::audio::Speaker;

impl<T> SceneSource for GeminiClient<T>
where
    T: Transport,
{
    /// Return one translated scene JSON document.
    fn scene(&self, language: &str, sentence: &str, target: &str) -> Result<serde_json::Value> {
        GeminiClient::<T>::scene(self, language, sentence, target)
    }
}

impl<T> Speaker for GeminiClient<T>
where
    T: Transport,
{
    /// Return one PCM audio payload for the prompt and source text.
    fn speech(&self, prompt: &str, text: &str) -> Result<Vec<u8>> {
        GeminiClient::<T>::speech(self, prompt, text)
    }
}
