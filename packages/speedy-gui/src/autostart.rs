//! Per-platform "launch at login" toggle for the speedy-daemon.
//!
//! Windows: HKCU\Software\Microsoft\Windows\CurrentVersion\Run
//! macOS:   ~/Library/LaunchAgents/com.speedy.daemon.plist
//! Linux:   ~/.config/autostart/speedy-daemon.desktop

use anyhow::{Context, Result};
use std::path::PathBuf;

const APP_KEY: &str = "SpeedyDaemon";

/// Try to locate the `speedy-daemon` executable next to the running GUI binary.
fn daemon_exe() -> Result<PathBuf> {
    let gui_exe = std::env::current_exe().context("current_exe failed")?;
    let dir = gui_exe.parent().context("current_exe has no parent")?;
    let name = format!("speedy-daemon{}", std::env::consts::EXE_SUFFIX);
    let candidate = dir.join(&name);
    if candidate.exists() {
        return Ok(candidate);
    }
    anyhow::bail!("speedy-daemon executable not found next to {}", gui_exe.display());
}

/// Return Ok(true) if the daemon is configured to start at login.
pub fn is_enabled() -> Result<bool> {
    #[cfg(windows)]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let run = hkcu.open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run");
        match run {
            Ok(k) => Ok(k.get_value::<String, _>(APP_KEY).is_ok()),
            Err(_) => Ok(false),
        }
    }
    #[cfg(target_os = "macos")]
    {
        Ok(plist_path()?.exists())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Ok(desktop_path()?.exists())
    }
}

/// Register the daemon binary to start at login.
pub fn enable() -> Result<()> {
    let exe = daemon_exe()?;

    #[cfg(windows)]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (run, _) =
            hkcu.create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")?;
        // Quote the path so a Program Files install doesn't break on spaces.
        let value = format!("\"{}\"", exe.display());
        run.set_value(APP_KEY, &value)?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let path = plist_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.speedy.daemon</string>
  <key>ProgramArguments</key>
  <array><string>{}</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><false/>
</dict>
</plist>
"#,
            exe.display()
        );
        std::fs::write(&path, plist)?;
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let path = desktop_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let entry = format!(
            "[Desktop Entry]\nType=Application\nName=Speedy Daemon\nExec={}\nX-GNOME-Autostart-enabled=true\n",
            exe.display()
        );
        std::fs::write(&path, entry)?;
        return Ok(());
    }
}

/// Unregister the daemon auto-start hook (best-effort: missing entries are OK).
pub fn disable() -> Result<()> {
    #[cfg(windows)]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(run) = hkcu.open_subkey_with_flags(
            r"Software\Microsoft\Windows\CurrentVersion\Run",
            KEY_SET_VALUE,
        ) {
            let _ = run.delete_value(APP_KEY);
        }
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        let path = plist_path()?;
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let path = desktop_path()?;
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        return Ok(());
    }
}

#[cfg(target_os = "macos")]
fn plist_path() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("no home dir")?
        .join("Library/LaunchAgents/com.speedy.daemon.plist"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn desktop_path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("no config dir")?
        .join("autostart/speedy-daemon.desktop"))
}
