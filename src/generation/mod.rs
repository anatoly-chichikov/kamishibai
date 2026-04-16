//! Asset generation pipeline and its runtime contracts.

pub mod artifact_cache;
mod catalog;
mod contracts;
mod pipeline;
pub mod prompts;
pub mod speech;

pub mod manga;

pub use artifact_cache::Cache;
pub use catalog::{GeneratorCatalog, SceneComposer};
pub use contracts::{
    BuildProgress, Deck, GeneratorSource, IllustrationGenerator, SceneSource, SkippedCard,
    SpeechGenerator,
};
pub use pipeline::DeckBuilder;
pub use prompts::{
    audio_prompt, manga_template, render_audio_prompt, render_scene_prompt, scene_prompt,
};
pub use speech::{Audio, Speaker};
