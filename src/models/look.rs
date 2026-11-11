//! Sockets for the picture: the style that words a scene and the gentle world that retells a refused one.

use std::fmt;
use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::Value;

use crate::generation::SceneMode;

/// Words a complete scene as the request for a drawing model.
///
/// Implement this to plug another picture style without editing the core. The core
/// still judges the returned picture; callers include its revision in their cache identity.
/// Experimental embedding API; compatibility is not guaranteed.
pub trait PictureStyle: Send + Sync {
    /// Name this style and its revision.
    fn name(&self) -> &str;

    /// Write the drawing request for one complete materialized scene.
    fn prompt(&self, scene: &Value) -> Result<String>;
}

/// Retells a scene the drawing model refused, in a world safe for any audience.
///
/// Implement this to plug a gentle world. With no world plugged, a refusal is an
/// ordinary failed attempt and no retelling is ever requested.
/// Experimental embedding API; compatibility is not guaranteed.
pub trait SoftWorld: Send + Sync {
    /// Name this world and its revision.
    fn name(&self) -> &str;

    /// Return the lines appended to both scene requests of a retelling.
    fn brief(&self) -> &str;

    /// Refuse a retold scene that left this world, given the drawing request the plugged style wrote for it.
    fn admit(&self, scene: &Value, prompt: &str) -> Result<()>;
}

/// Whether a paid picture request with an unknown outcome may be sent again without the person.
/// Experimental embedding API; compatibility is not guaranteed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Resend {
    /// Send again within the attempt ceiling.
    Automatic,
    /// Wait for an explicit retry before sending again.
    Manual,
}

/// The built-in ink manga style written for the built-in drawing model.
/// Experimental embedding API; compatibility is not guaranteed.
#[derive(Clone, Copy, Debug, Default)]
pub struct MangaInk;

impl PictureStyle for MangaInk {
    fn name(&self) -> &str {
        "manga-ink"
    }

    fn prompt(&self, scene: &Value) -> Result<String> {
        crate::generation::manga::compile_image_prompt(scene)
    }
}

/// Everything an application plugs in for pictures: a style, an optional gentle world, and its resend rule.
/// Experimental embedding API; compatibility is not guaranteed.
#[derive(Clone)]
pub struct Look {
    style: Arc<dyn PictureStyle>,
    world: Option<Arc<dyn SoftWorld>>,
    resend: Resend,
}

impl Look {
    /// Bind a picture style, an optional gentle world, and a resend rule.
    /// Experimental embedding API; compatibility is not guaranteed.
    #[must_use]
    pub fn new(
        style: Arc<dyn PictureStyle>,
        world: Option<Arc<dyn SoftWorld>>,
        resend: Resend,
    ) -> Self {
        Self {
            style,
            world,
            resend,
        }
    }

    /// Write the drawing request for a complete scene in the plugged style.
    /// Experimental embedding API; compatibility is not guaranteed.
    pub fn prompt(&self, scene: &Value) -> Result<String> {
        self.style.prompt(scene)
    }

    /// Whether a gentle world is plugged.
    /// Experimental embedding API; compatibility is not guaranteed.
    #[must_use]
    pub fn gentle(&self) -> bool {
        self.world.is_some()
    }

    /// Whether an interrupted paid picture request waits for an explicit retry.
    /// Experimental embedding API; compatibility is not guaranteed.
    #[must_use]
    pub fn manual(&self) -> bool {
        self.resend == Resend::Manual
    }

    /// Name the plugged style and world for diagnostics and provenance.
    /// The core does not add this name to cache identities; callers include
    /// their plugged parts' revisions in the identity passed to `models::workflow`.
    /// Experimental embedding API; compatibility is not guaranteed.
    #[must_use]
    pub fn name(&self) -> String {
        match &self.world {
            Some(world) => format!("{}+{}", self.style.name(), world.name()),
            None => String::from(self.style.name()),
        }
    }

    /// Keep the original scene request or append the plugged world's brief.
    pub(crate) fn worded(&self, mode: SceneMode, prompt: String) -> Result<String> {
        match (mode, &self.world) {
            (SceneMode::Stock, _) => Ok(prompt),
            (SceneMode::Soft, Some(world)) => Ok(format!("{prompt}\n\n{}", world.brief())),
            (SceneMode::Soft, None) => {
                bail!("a gentle retelling was requested without a plugged world")
            }
        }
    }

    /// Let the plugged world refuse a retold scene, wording it in the plugged style first.
    pub(crate) fn admit(&self, scene: &Value) -> Result<()> {
        self.admit_prompt(scene, self.style.prompt(scene)?.as_str())
    }

    /// Let the plugged world refuse a retold scene whose drawing request is already written.
    pub(crate) fn admit_prompt(&self, scene: &Value, prompt: &str) -> Result<()> {
        match &self.world {
            Some(world) => world.admit(scene, prompt),
            None => bail!("a gentle scene arrived without a plugged world"),
        }
    }

    /// Renew the picture after an explicit retry: open a new request series and leave the gentle world.
    /// Experimental embedding API; compatibility is not guaranteed.
    pub fn retry(&self, cache: &crate::generation::artifact_cache::Cache) -> Result<()> {
        crate::generation::restart_picture_request_series(cache)?;
        if self.manual() {
            crate::generation::manga::retry_image_request(
                &cache
                    .path()
                    .join(crate::generation::artifact_cache::IMAGE_ATTEMPTS_DIRECTORY),
            )?;
        }
        let soft = cache
            .path()
            .join(crate::generation::artifact_cache::SOFT_FILE);
        if soft.exists() {
            let scene = cache
                .path()
                .join(crate::generation::artifact_cache::SCENE_FILE);
            if scene.exists() {
                let value: Value = serde_json::from_slice(&std::fs::read(&scene)?)?;
                if value
                    .pointer("/manga_panel/meta/layout_selection/scene_mode")
                    .and_then(Value::as_str)
                    == Some("soft")
                {
                    std::fs::remove_file(scene)?;
                }
            }
            std::fs::remove_file(soft)?;
        }
        Ok(())
    }
}

impl Default for Look {
    fn default() -> Self {
        Self::new(Arc::new(MangaInk), None, Resend::Automatic)
    }
}

impl fmt::Debug for Look {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Look")
            .field("name", &self.name())
            .field("resend", &self.resend)
            .finish()
    }
}
