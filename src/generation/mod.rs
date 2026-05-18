//! Asset generation pipeline and its runtime contracts.

pub mod artifact_cache;
mod catalog;
mod contracts;
pub mod prompts;
pub mod speech;

pub mod manga;

pub use artifact_cache::Cache;
pub use catalog::SceneComposer;
pub use contracts::SceneSource;
pub use prompts::{
    audio_prompt, manga_template, render_audio_prompt, render_scene_prompt, scene_prompt,
};
pub use speech::{Audio, Speaker};
