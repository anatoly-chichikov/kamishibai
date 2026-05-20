//! Host integration used by the CLI shell.

use std::process::Command;

use anyhow::Result;

/// Open one filesystem path with the host system's default handler.
pub(super) fn open_path(path: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(path).spawn()?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(path).spawn()?;
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/c", "start", "", path])
            .spawn()?;
    }
    Ok(())
}
