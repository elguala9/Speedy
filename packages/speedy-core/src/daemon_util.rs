use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub fn daemon_dir_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("no config directory found")?
        .join("speedy");
    Ok(dir)
}

pub fn kill_existing_daemon(daemon_dir: &Path) {
    let pid_path = daemon_dir.join("daemon.pid");
    if let Ok(pid_str) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            #[cfg(windows)]
            {
                let _ = std::process::Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/F"])
                    .status();
            }
            #[cfg(not(windows))]
            {
                let _ = std::process::Command::new("kill")
                    .args(["-9", &pid.to_string()])
                    .status();
            }
        }
    }
    let _ = std::fs::remove_file(&pid_path);
}

pub fn spawn_daemon_process(port: u16) -> Result<()> {
    let exe = find_daemon_exe()
        .context("failed to find daemon executable")?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new(&exe)
            .arg("--daemon-port")
            .arg(port.to_string())
            .creation_flags(0x08000000)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("failed to spawn daemon process")?;
    }

    #[cfg(not(windows))]
    {
        std::process::Command::new(&exe)
            .arg("--daemon-port")
            .arg(port.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("failed to spawn daemon process")?;
    }

    Ok(())
}

fn find_daemon_exe() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let dir = exe.parent().context("no parent dir")?;

    // Cerca speedy-daemon.exe nella stessa directory del CLI
    let daemon_name = format!("speedy-daemon{}", std::env::consts::EXE_SUFFIX);
    let candidate = dir.join(&daemon_name);
    if candidate.exists() {
        return Ok(candidate.canonicalize()?);
    }

    // Fallback: cerca nella directory target
    let fallback = dir.join("..").join("..").join("target").join("debug").join(&daemon_name);
    if fallback.exists() {
        return Ok(fallback.canonicalize()?);
    }

    anyhow::bail!("speedy-daemon executable not found next to {}", exe.display());
}
