//! Launch the daemon at user login (opt-in, managed from the GUI).
//!
//! The installer no longer registers any autostart entry — Speedy works via
//! git hooks by default. The user opts in from the Dashboard, which writes an
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value pointing at
//! `speedy-daemon.exe`.
//!
//! A dedicated `Run` value (not the legacy Startup-folder shortcut the old
//! installer created) keeps the two mechanisms separate: an installer upgrade
//! cleans up its own legacy `.lnk` without ever touching the user's
//! GUI-managed autostart.
//!
//! Windows-only; on other platforms the functions are no-ops.

#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const RUN_VALUE: &str = "Speedy Daemon";

/// Whether daemon-at-login is currently enabled.
#[cfg(windows)]
pub fn is_enabled() -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("reg")
        .args(["query", RUN_KEY, "/v", RUN_VALUE])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Register `exe` to launch at login. The path is stored quoted so that an
/// install location containing spaces (e.g. `C:\Program Files\Speedy`) is
/// parsed correctly by the shell at logon.
#[cfg(windows)]
pub fn enable(exe: &std::path::Path) -> anyhow::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let quoted = format!("\"{}\"", exe.display());
    let out = std::process::Command::new("reg")
        .args(["add", RUN_KEY, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &quoted, "/f"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if !out.status.success() {
        anyhow::bail!("reg add failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Remove the daemon-at-login entry. Idempotent: a missing value is success.
#[cfg(windows)]
pub fn disable() -> anyhow::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("reg")
        .args(["delete", RUN_KEY, "/v", RUN_VALUE, "/f"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    // `reg delete` exits non-zero when the value does not exist; treat the
    // "already gone" case as success so the toggle is idempotent.
    if !out.status.success() && is_enabled() {
        anyhow::bail!("reg delete failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn is_enabled() -> bool {
    false
}

#[cfg(not(windows))]
pub fn enable(_exe: &std::path::Path) -> anyhow::Result<()> {
    anyhow::bail!("launch at login is only supported on Windows")
}

#[cfg(not(windows))]
pub fn disable() -> anyhow::Result<()> {
    Ok(())
}
