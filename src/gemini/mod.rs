//! Direct REST client for Gemini text, image, and TTS generation.

mod client;
mod codec;
mod prompts;
mod protocol;

pub use client::{GeminiClient, HttpTransport, Transport, TransportResponse};

use anyhow::Result;

use crate::generation::SceneSource;
use crate::generation::Speaker;
use crate::session::{
    BulkCorrection, CardCorrection, CardDraft, CardMeta, CardMetaGeneration, CardRevision,
    LanguagePair, RawInputBatch, Understanding, Understood, WordCandidate,
};

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

impl<T> Understanding for GeminiClient<T>
where
    T: Transport,
{
    /// Return one reviewed candidate list from the raw user blob.
    fn understand(&self, raw: &RawInputBatch, my: &str) -> Result<Understood> {
        GeminiClient::<T>::understand(self, raw, my)
    }
}

impl<T> BulkCorrection for GeminiClient<T>
where
    T: Transport,
{
    /// Return the candidate list after one bulk user correction.
    fn correct_bulk(
        &self,
        candidates: &[WordCandidate],
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<Vec<WordCandidate>> {
        GeminiClient::<T>::correct_bulk(self, candidates, comment, pair)
    }
}

impl<T> CardMetaGeneration for GeminiClient<T>
where
    T: Transport,
{
    /// Return one rich card meta for a term plus the understanding context.
    fn generate_card_meta(
        &self,
        term: &str,
        understanding: &str,
        pair: &LanguagePair,
    ) -> Result<CardMeta> {
        GeminiClient::<T>::generate_card_meta(self, term, understanding, pair)
    }
}

impl<T> CardCorrection for GeminiClient<T>
where
    T: Transport,
{
    /// Return one card revision after a per-card user correction.
    fn correct_card(
        &self,
        draft: &CardDraft,
        comment: &str,
        pair: &LanguagePair,
    ) -> Result<CardRevision> {
        GeminiClient::<T>::correct_card(self, draft, comment, pair)
    }
}
