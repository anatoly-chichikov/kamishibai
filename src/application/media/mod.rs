//! Media service registry and batch orchestration.

mod pipeline;
mod ports;

pub use pipeline::Pipeline;
pub use ports::{
    AudioService, Deck, Failure, IllustrationService, MediaSource, PipelineProgress, SceneSource,
};
