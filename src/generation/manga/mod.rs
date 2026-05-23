//! Manga scene composition, OCR validation, and illustration persistence.

mod border;
mod contracts;
mod illustration;
pub mod ocr_bundle;
mod redirect;
mod render;
mod text;

use anyhow::Result;
use serde_json::Value;

use crate::gemini::{GeminiClient, Transport};

pub use border::BorderDetector;
pub use contracts::{ImageSource, ImageText, Progress, Renderer, SceneText, Translator};
pub use illustration::Illustration;
pub use render::MangaRenderer;
pub use text::{TextDetector, TextDetectors};

impl<T> ImageSource for GeminiClient<T>
where
    T: Transport,
{
    /// Return one encoded image payload for the scene.
    fn image(&self, scene: &Value) -> Result<Vec<u8>> {
        GeminiClient::<T>::image(self, scene)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::fs;

    use anyhow::Result;

    use super::TextDetector;
    #[cfg(unix)]
    use super::redirect::{Redirect, discarded, locked, quiet};
    use super::text::resolved;

    #[cfg(unix)]
    struct NoisyDrop;

    #[cfg(unix)]
    impl Drop for NoisyDrop {
        /// Write to stdout and stderr when the test value is dropped.
        fn drop(&mut self) {
            let _ = rustix::io::write(std::io::stdout(), "άλφα\n".as_bytes());
            let _ = rustix::io::write(std::io::stderr(), "βήτα\n".as_bytes());
        }
    }

    /// Redirect routes noisy process output into the sink file.
    #[cfg(unix)]
    #[test]
    fn redirect_routes_stdout_and_stderr_into_the_sink_file() -> Result<()> {
        let sink = tempfile::NamedTempFile::new()?;
        locked(|| {
            let item = Redirect::new(sink.reopen()?)?;
            rustix::io::write(std::io::stdout(), "άλφα\n".as_bytes())?;
            rustix::io::write(std::io::stderr(), "βήτα\n".as_bytes())?;
            item.restore()
        })?;
        let text = fs::read_to_string(sink.path())?;
        assert_eq!(
            (text.contains("άλφα\n"), text.contains("βήτα\n")),
            (true, true),
            "redirect no longer routes stdout and stderr into the sink file"
        );
        Ok(())
    }

    /// Quiet redirection discards noisy process output inside the closure.
    #[cfg(unix)]
    #[test]
    fn quiet_redirection_discards_stdout_and_stderr_inside_the_closure() -> Result<()> {
        let sink = tempfile::NamedTempFile::new()?;
        locked(|| {
            let item = Redirect::new(sink.reopen()?)?;
            quiet(|| {
                rustix::io::write(std::io::stdout(), "άλφα\n".as_bytes())?;
                rustix::io::write(std::io::stderr(), "βήτα\n".as_bytes())?;
                Ok(())
            })?;
            item.restore()
        })?;
        let text = fs::read_to_string(sink.path())?;
        assert_eq!(
            (text.contains("άλφα\n"), text.contains("βήτα\n")),
            (false, false),
            "quiet redirection no longer discards stdout and stderr inside the closure"
        );
        Ok(())
    }

    /// Discarded drops mute stdout and stderr during value destruction.
    #[cfg(unix)]
    #[test]
    fn discarded_drops_mute_stdout_and_stderr_during_value_destruction() -> Result<()> {
        let sink = tempfile::NamedTempFile::new()?;
        locked(|| {
            let item = Redirect::new(sink.reopen()?)?;
            discarded(NoisyDrop)?;
            item.restore()
        })?;
        let text = fs::read_to_string(sink.path())?;
        assert_eq!(
            (text.contains("άλφα\n"), text.contains("βήτα\n")),
            (false, false),
            "discarded drops no longer mute stdout and stderr during value destruction"
        );
        Ok(())
    }

    /// Explicit inventories keep supported OCR tokens in order.
    #[test]
    fn explicit_inventories_keep_supported_ocr_tokens_in_order() {
        assert_eq!(
            resolved(
                String::from("eng+ell"),
                &[
                    String::from("eng"),
                    String::from("ell"),
                    String::from("osd")
                ]
            ),
            String::from("eng+ell"),
            "explicit inventories no longer keep supported ocr tokens in order"
        );
    }

    /// Explicit inventories drop unsupported OCR tokens.
    #[test]
    fn explicit_inventories_drop_unsupported_ocr_tokens() {
        assert_eq!(
            resolved(
                String::from("eng+ell"),
                &[String::from("eng"), String::from("osd")]
            ),
            String::from("eng"),
            "explicit inventories no longer drop unsupported ocr tokens"
        );
    }

    /// The default detector still resolves to English.
    #[test]
    fn default_detector_still_resolves_to_english() {
        assert_eq!(
            TextDetector::new(60).selection(),
            "eng",
            "default detector no longer resolves to English"
        );
    }
}
