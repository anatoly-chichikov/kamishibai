//! Rust library surface for the canonical kamishibai runtime.

#![forbid(unsafe_code)]

mod application;
mod domain;
mod infrastructure;
mod presentation;

pub mod anki {
    pub use crate::infrastructure::anki::*;
}

pub mod assets {
    pub use crate::infrastructure::assets::*;
}

pub mod audio {
    pub use crate::infrastructure::audio::*;
}

pub mod cache {
    pub use crate::infrastructure::cache::*;
}

pub mod cli {
    pub use crate::presentation::cli::*;
}

pub mod diagnosis {
    pub use crate::presentation::diagnosis::*;
}

pub mod gemini {
    pub use crate::infrastructure::gemini::*;
}

pub mod input {
    pub use crate::domain::entry::NormalizedEntry;
    pub use crate::infrastructure::input::*;
}

pub mod media {
    pub use crate::application::media::*;
    pub use crate::infrastructure::media::{Media, SceneTranslator};
}

pub mod paths {
    pub use crate::infrastructure::paths::*;
}

pub mod profile {
    pub use crate::domain::profile::*;
}

pub mod progress {
    pub use crate::presentation::progress::*;
}

pub mod report {
    pub use crate::infrastructure::report::*;
}

pub mod scene {
    pub use crate::infrastructure::scene::*;
}

mod ocr {
    pub(crate) use crate::infrastructure::ocr::*;
}
