//! Asset generation pipeline and its runtime contracts.

pub mod artifact_cache;
mod catalog;
mod contracts;
pub(crate) mod layout;
pub mod prompts;
pub mod speech;

pub mod manga;

pub use artifact_cache::Cache;
pub use catalog::SceneComposer;
pub use contracts::SceneSource;
pub use prompts::{audio_prompt, manga_template, render_audio_prompt, visual_revision};
pub use speech::{Audio, Speaker};
