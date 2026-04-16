//! Plain and terminal progress output for the pipeline and CLI.

mod contracts;
mod plain;
mod rich;
mod selector;
mod terminal;

pub use contracts::{AlignedStatus, AppProgress, Console, Live, Output, Spinner, Status};
pub use plain::PlainProgress;
pub use rich::RichProgress;
pub use selector::{ProgressSelector, SelectedProgress};
pub use terminal::{StdoutOutput, TerminalConsole, TerminalStatus};

/// Return whether terminal progress should render on stdout.
pub fn uses_stdout() -> bool {
    cfg!(any(target_os = "macos", target_os = "ios"))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::terminal::{INTERVAL, frame, line, rendered};
    use super::uses_stdout;

    /// Terminal rendering converts rich progress markup into ANSI and OSC8 sequences.
    #[test]
    fn terminal_rendering_converts_rich_progress_markup_into_ansi_and_osc8_sequences() {
        assert_eq!(
            rendered("[bold]whims[/bold]  [green]✔[/green] ([link=file:///tmp/x.wav]x.wav[/link])"),
            "\u{1b}[1mwhims\u{1b}[0m  \u{1b}[32m✔\u{1b}[0m (\u{1b}]8;;file:///tmp/x.wav\u{1b}\\x.wav\u{1b}]8;;\u{1b}\\)",
            "terminal rendering no longer converts rich progress markup into ansi and osc8 sequences"
        );
    }

    /// Terminal spinner frames keep the circular half-step sequence.
    #[test]
    fn terminal_spinner_frames_keep_the_circular_half_step_sequence() {
        assert_eq!(
            (frame(0), frame(1), frame(2), frame(3), frame(4)),
            ("◐", "◓", "◑", "◒", "◐"),
            "terminal spinner frames no longer keep the circular half step sequence"
        );
    }

    /// Terminal spinner lines keep the same left indent as completed steps.
    #[test]
    fn terminal_spinner_lines_keep_the_same_left_indent_as_completed_steps() {
        assert_eq!(
            line("◐", "Rendering manga..."),
            String::from("\r\x1b[2K  ◐ Rendering manga..."),
            "terminal spinner lines no longer keep the same left indent as completed steps"
        );
    }

    /// Terminal spinner waits two hundred milliseconds between frames.
    #[test]
    fn terminal_spinner_waits_two_hundred_milliseconds_between_frames() {
        assert_eq!(
            INTERVAL,
            Duration::from_millis(200),
            "terminal spinner no longer waits two hundred milliseconds between frames"
        );
    }

    /// Terminal progress keeps stdout on Apple to avoid OCR stderr suppression.
    #[test]
    fn terminal_progress_keeps_stdout_on_apple_to_avoid_ocr_stderr_suppression() {
        assert_eq!(
            uses_stdout(),
            cfg!(any(target_os = "macos", target_os = "ios")),
            "terminal progress no longer keeps stdout on apple to avoid ocr stderr suppression"
        );
    }
}
