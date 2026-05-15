use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub fn daemon_dir_path() -> Result<PathBuf> {
    if let Ok(custom) = std::env::var("SPEEDY_DAEMON_DIR") {
        if !custom.is_empty() {
            return Ok(PathBuf::from(custom));
        }
    }
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

pub fn default_daemon_socket_name() -> String {
    std::env::var("SPEEDY_DEFAULT_SOCKET")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "speedy-daemon".to_string())
}

/// Try to acquire an exclusive advisory lock on `<daemon_dir>/daemon.lock`.
/// Returns the locked `File` on success — keep it alive for the process
/// lifetime; the lock releases when the file handle drops.
///
/// `Err` means another daemon already holds the lock. Callers must abort,
/// not retry — `kill_existing_daemon` already ran by this point, so a still-
/// locked file proves a peer daemon is genuinely up.
pub fn acquire_daemon_lock(daemon_dir: &Path) -> Result<File> {
    let lock_path = daemon_dir.join("daemon.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&lock_path)
        .with_context(|| format!("failed to open daemon lock: {}", lock_path.display()))?;

    // We leak a RwLock<File> wrapper to satisfy fd-lock's borrow API: a
    // long-lived guard would otherwise need a self-referential struct. The
    // OS releases the lock when the underlying file handle (cloned below)
    // drops, regardless of the leaked wrapper.
    let cloned = file.try_clone()
        .context("failed to clone lock file handle")?;
    let wrapper: &'static mut fd_lock::RwLock<File> =
        Box::leak(Box::new(fd_lock::RwLock::new(cloned)));
    let guard = wrapper.try_write().map_err(|_| {
        anyhow::anyhow!(
            "another speedy-daemon already holds {}",
            lock_path.display()
        )
    })?;
    // Leak the guard too — the lock survives until process exit.
    Box::leak(Box::new(guard));

    Ok(file)
}

pub fn spawn_daemon_process(socket_name: &str) -> Result<()> {
    let exe = resolve_daemon_exe()
        .context("failed to find daemon executable")?;
    spawn_daemon_process_with(&exe, socket_name)
}

/// Spawn the daemon at a caller-specified path. Used by the GUI when the user
/// overrides the auto-detected binary location from the Dashboard.
pub fn spawn_daemon_process_with(exe: &Path, socket_name: &str) -> Result<()> {
    if !exe.exists() {
        anyhow::bail!("daemon executable not found: {}", exe.display());
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS (0x8) | CREATE_NO_WINDOW (0x08000000) | CREATE_NEW_PROCESS_GROUP (0x200):
        // fully detach so the calling shell doesn't keep pipe handles open.
        std::process::Command::new(exe)
            .arg("--daemon-socket")
            .arg(socket_name)
            .creation_flags(0x08000208)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("failed to spawn daemon process")?;
    }

    #[cfg(not(windows))]
    {
        std::process::Command::new(exe)
            .arg("--daemon-socket")
            .arg(socket_name)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("failed to spawn daemon process")?;
    }

    Ok(())
}

/// Locate the `speedy-daemon` binary using the same heuristics
/// `spawn_daemon_process` uses internally. Exposed for callers (e.g. the GUI)
/// that want to display the resolved path to the user.
pub fn resolve_daemon_exe() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let dir = exe.parent().context("no parent dir")?;

    let daemon_name = format!("speedy-daemon{}", std::env::consts::EXE_SUFFIX);

    // Same directory as the running binary (production install layout).
    let candidate = dir.join(&daemon_name);
    if candidate.exists() {
        return Ok(candidate.canonicalize()?);
    }

    // `cargo test` puts the test binary in target/debug/deps/ — the daemon
    // binary is one level up.
    if dir.file_name().and_then(|s| s.to_str()) == Some("deps") {
        if let Some(parent) = dir.parent() {
            let candidate = parent.join(&daemon_name);
            if candidate.exists() {
                return Ok(candidate.canonicalize()?);
            }
        }
    }

    // Legacy fallback for callers running from arbitrary nested locations.
    let fallback = dir.join("..").join("..").join("target").join("debug").join(&daemon_name);
    if fallback.exists() {
        return Ok(fallback.canonicalize()?);
    }

    anyhow::bail!("speedy-daemon executable not found next to {}", exe.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    // Serializes tests that mutate process-wide env vars.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_default_daemon_socket_name_returns_default() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("SPEEDY_DEFAULT_SOCKET").ok();
        std::env::remove_var("SPEEDY_DEFAULT_SOCKET");

        assert_eq!(default_daemon_socket_name(), "speedy-daemon");

        if let Some(v) = prev { std::env::set_var("SPEEDY_DEFAULT_SOCKET", v); }
    }

    #[test]
    fn test_default_daemon_socket_name_honors_env() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("SPEEDY_DEFAULT_SOCKET").ok();
        std::env::set_var("SPEEDY_DEFAULT_SOCKET", "custom-sock");

        assert_eq!(default_daemon_socket_name(), "custom-sock");

        match prev {
            Some(v) => std::env::set_var("SPEEDY_DEFAULT_SOCKET", v),
            None => std::env::remove_var("SPEEDY_DEFAULT_SOCKET"),
        }
    }

    #[test]
    fn test_default_daemon_socket_name_ignores_empty_env() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("SPEEDY_DEFAULT_SOCKET").ok();
        std::env::set_var("SPEEDY_DEFAULT_SOCKET", "");

        assert_eq!(default_daemon_socket_name(), "speedy-daemon");

        match prev {
            Some(v) => std::env::set_var("SPEEDY_DEFAULT_SOCKET", v),
            None => std::env::remove_var("SPEEDY_DEFAULT_SOCKET"),
        }
    }

    #[test]
    fn test_daemon_dir_path_uses_env_override() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("SPEEDY_DAEMON_DIR").ok();
        let custom = std::env::temp_dir().join("speedy_du_custom");
        std::env::set_var("SPEEDY_DAEMON_DIR", &custom);

        let p = daemon_dir_path().unwrap();
        assert_eq!(p, custom);

        match prev {
            Some(v) => std::env::set_var("SPEEDY_DAEMON_DIR", v),
            None => std::env::remove_var("SPEEDY_DAEMON_DIR"),
        }
    }

    #[test]
    fn test_daemon_dir_path_ignores_empty_env() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("SPEEDY_DAEMON_DIR").ok();
        std::env::set_var("SPEEDY_DAEMON_DIR", "");

        let p = daemon_dir_path().unwrap();
        // Falls back to dirs::config_dir().join("speedy")
        assert!(p.ends_with("speedy"), "expected default 'speedy' suffix, got: {p:?}");

        match prev {
            Some(v) => std::env::set_var("SPEEDY_DAEMON_DIR", v),
            None => std::env::remove_var("SPEEDY_DAEMON_DIR"),
        }
    }

    #[test]
    fn test_now_rfc3339_format() {
        let s = now_rfc3339();
        // Bare-minimum sanity: looks like an RFC3339 timestamp.
        assert!(s.contains('T'), "expected 'T' separator in {s}");
        assert!(s.len() >= 19, "timestamp too short: {s}");
    }

    #[test]
    fn test_kill_existing_daemon_no_pidfile() {
        let dir = std::env::temp_dir().join("speedy_du_kill_nopid");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Should not panic when there's no pid file.
        kill_existing_daemon(&dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_kill_existing_daemon_invalid_pidfile() {
        let dir = std::env::temp_dir().join("speedy_du_kill_invalid");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("daemon.pid"), "not-a-number").unwrap();

        kill_existing_daemon(&dir);
        // pidfile is removed even when content was junk.
        assert!(!dir.join("daemon.pid").exists(), "pidfile should be cleaned up");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_kill_existing_daemon_stale_pid_is_cleaned() {
        let dir = std::env::temp_dir().join("speedy_du_kill_stale");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 99999 is very unlikely to be a real PID. taskkill/kill will fail
        // silently and we still want the pidfile removed afterwards.
        std::fs::write(dir.join("daemon.pid"), "99999").unwrap();

        kill_existing_daemon(&dir);
        assert!(!dir.join("daemon.pid").exists(), "stale pidfile should be removed");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_acquire_daemon_lock_second_attempt_fails() {
        let dir = std::env::temp_dir().join(format!(
            "speedy_du_lock_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let first = acquire_daemon_lock(&dir);
        assert!(first.is_ok(), "first lock should succeed: {:?}", first.err());

        // Same process, lock still held by the leaked guard — second call
        // must report contention rather than silently succeed.
        let second = acquire_daemon_lock(&dir);
        assert!(second.is_err(), "second lock must fail while first is held");
        let err = second.unwrap_err().to_string();
        assert!(err.contains("already holds"), "expected contention message, got: {err}");

        // Don't remove the lock file — the leaked guard keeps it locked for
        // the rest of the test binary's life. The tempdir cleanup at next run
        // will reclaim it.
    }

    #[test]
    fn test_spawn_daemon_process_uses_built_binary() {
        // Only meaningful if speedy-daemon is built. Skip otherwise so the
        // test passes on a fresh checkout where only this crate compiled.
        let candidate = std::env::current_exe().ok()
            .and_then(|e| e.parent().map(|p| p.to_path_buf()))
            .map(|d| {
                let name = format!("speedy-daemon{}", std::env::consts::EXE_SUFFIX);
                let direct = d.join(&name);
                if direct.exists() { Some(direct) }
                else if d.file_name().and_then(|s| s.to_str()) == Some("deps") {
                    d.parent().map(|p| p.join(&name)).filter(|p| p.exists())
                } else { None }
            })
            .flatten();

        if candidate.is_none() {
            eprintln!("skipping: speedy-daemon binary not built");
            return;
        }

        // Spawn with a unique socket name so we don't clobber a running daemon.
        // We don't actually need the daemon to keep running — just verify the
        // spawn call itself succeeds.
        let socket = format!("speedy_du_spawn_{}_{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
        let result = spawn_daemon_process(&socket);
        assert!(result.is_ok(), "spawn_daemon_process should succeed: {:?}", result.err());

        // Best-effort cleanup: connect and tell it to stop.
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let client = crate::daemon_client::DaemonClient::new(&socket);
            let _ = client.stop().await;
        });
    }
}
