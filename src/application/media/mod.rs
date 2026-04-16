//! Media service registry and batch orchestration.

mod pipeline;
mod ports;

pub use pipeline::Pipeline;
pub use ports::{
    AudioService, AudioSource, Deck, Failure, IllustrationService, IllustrationSource,
    PipelineProgress, Profiles, SceneSource,
};
