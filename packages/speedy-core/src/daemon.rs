use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::sync::Arc;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonInfo {
    pub project_path: String,
    pub pid: Option<u32>,
    pub status: String,
    pub auto_start: bool,
    pub installed_at: String,
}

fn daemon_json_path(root: &Path) -> PathBuf {
    root.join(".speedy").join("daemon.json")
}

pub fn read_info(root: &Path) -> Result<Option<DaemonInfo>> {
    let path = daemon_json_path(root);
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)?;
    Ok(Some(serde_json::from_str(&content)?))
}

pub fn write_info(root: &Path, info: &DaemonInfo) -> Result<()> {
    let dir = root.join(".speedy");
    std::fs::create_dir_all(&dir)?;
    let path = daemon_json_path(root);
    let content = serde_json::to_string_pretty(info)?;
    std::fs::write(&path, content)?;
    Ok(())
}

pub fn update_status(root: &Path, status: &str, pid: Option<u32>) -> Result<()> {
    if let Some(mut info) = read_info(root)? {
        info.status = status.to_string();
        info.pid = pid;
        write_info(root, &info)?;
    }
    Ok(())
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub fn save_pid(root: &Path, pid: u32) -> Result<()> {
    let info = DaemonInfo {
        project_path: root.to_string_lossy().to_string(),
        pid: Some(pid),
        status: "running".to_string(),
        auto_start: true,
        installed_at: now_rfc3339(),
    };
    write_info(root, &info)
}

pub fn mark_stopped(root: &Path) -> Result<()> {
    update_status(root, "stopped", None)
}

#[cfg(windows)]
fn service_name_for(root: &Path) -> String {
    format!("speedy-watch-{}", root.to_string_lossy().replace(|c: char| !c.is_alphanumeric(), "_"))
}

#[cfg(windows)]
pub fn run_as_windows_service(service_name: &str) -> Result<()> {
    windows_service::service_dispatcher::start(service_name, service_main_impl)?;
    Ok(())
}

#[cfg(windows)]
extern "system" fn service_main_impl(dw_argc: u32, lpsz_argv: *mut *mut u16) {
    use std::os::windows::ffi::OsStringExt;

    let args: Vec<std::ffi::OsString> = (0..dw_argc)
        .filter_map(|i| unsafe {
            let ptr = *lpsz_argv.offset(i as isize);
            if ptr.is_null() {
                return None;
            }
            let mut len = 0;
            while *ptr.offset(len) != 0 {
                len += 1;
            }
            Some(std::ffi::OsString::from_wide(
                std::slice::from_raw_parts(ptr, len as usize),
            ))
        })
        .collect();

    service_main_body(args);
}

#[cfg(windows)]
fn service_main_body(_args: Vec<std::ffi::OsString>) {
    use windows_service::service::*;
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use std::time::Duration;

    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = stop_tx.send(());
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let service_name = match std::env::args()
        .position(|a| a == "--service-name")
        .and_then(|i| std::env::args().nth(i + 1))
    {
        Some(n) => n,
        None => {
            eprintln!("speedy service: missing --service-name argument");
            return;
        }
    };

    let status_handle = match service_control_handler::register(&service_name, event_handler) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("speedy service: failed to register control handler: {e}");
            return;
        }
    };

    let set_status = |state: ServiceState, accept: ServiceControlAccept| {
        let _ = status_handle.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: Some(std::process::id()),
        });
    };

    set_status(ServiceState::Running, ServiceControlAccept::STOP);

    let workspace_path = match std::env::args()
        .position(|a| a == "-p")
        .and_then(|i| std::env::args().nth(i + 1))
    {
        Some(p) => p,
        None => {
            eprintln!("speedy service: missing -p argument");
            set_status(ServiceState::Stopped, ServiceControlAccept::empty());
            return;
        }
    };

    std::env::set_current_dir(&workspace_path).ok();
    let config = crate::config::Config::load();
    let running = Arc::new(AtomicBool::new(true));
    let run = running.clone();

    std::thread::spawn(move || {
        let _ = stop_rx.recv();
        run.store(false, Ordering::SeqCst);
    });

    let rt = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("speedy service: failed to create runtime: {e}");
            set_status(ServiceState::Stopped, ServiceControlAccept::empty());
            return;
        }
    };

    let _ = rt.block_on(crate::watcher::start_service_watcher(".", &config, running));
    set_status(ServiceState::Stopped, ServiceControlAccept::empty());
}

pub fn find_all_daemons() -> Vec<DaemonInfo> {
    let workspaces = crate::workspace::list().unwrap_or_default();
    workspaces
        .iter()
        .filter_map(|ws| {
            let root = Path::new(&ws.path);
            let info = read_info(root).ok().flatten().unwrap_or(DaemonInfo {
                project_path: ws.path.clone(),
                pid: None,
                status: "unknown".to_string(),
                auto_start: true,
                installed_at: String::new(),
            });
            Some(info)
        })
        .collect()
}

pub async fn install(path: Option<String>) -> Result<()> {
    let root = path
        .map(|p| PathBuf::from(p))
        .unwrap_or_else(|| std::env::current_dir().unwrap())
        .canonicalize()?;

    let root_str = root.to_string_lossy().to_string();
    if !crate::workspace::is_registered(&root_str) {
        anyhow::bail!("No workspace registered for {}. A daemon cannot exist without a workspace. Create a workspace first.", root.display());
    }

    let exe = std::env::current_exe()?
        .canonicalize()?;

    #[cfg(windows)]
    {
        let task_name = service_name_for(&root);
        let exe_str = exe.to_string_lossy().to_string();
        let bin_path = format!(
            "\"{}\" --service-name \"{}\" --as-service -p \"{}\" watch .",
            exe_str, task_name, root_str
        );

        let status = std::process::Command::new("sc")
            .args(["create", &task_name, "binPath=", &bin_path, "start=", "auto",
                   "DisplayName=", &format!("\"Speedy Watcher - {root_str}\"")])
            .status()
            .context("failed to run sc.exe create")?;

        if !status.success() {
            anyhow::bail!("sc.exe create failed (exit: {:?})", status.code());
        }
    }

    #[cfg(target_os = "linux")]
    {
        let service_name = format!("speedy-watch-{}", root.to_string_lossy().replace('/', "_"));
        let config_dir = dirs::config_dir()
            .context("no config dir")?
            .join("systemd")
            .join("user");
        std::fs::create_dir_all(&config_dir)?;

        let service = format!(
            "[Unit]\nDescription=Speedy watcher for {}\nAfter=network.target\n\n[Service]\nExecStart={} -p \"{}\" watch\nRestart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n",
            root.display(), exe.to_string_lossy(), root.to_string_lossy(),
        );
        std::fs::write(config_dir.join(format!("{service_name}.service")), &service)?;
        let _ = std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).status();
        let _ = std::process::Command::new("systemctl").args(["--user", "enable", &service_name]).status();
    }

    let info = DaemonInfo {
        project_path: root.to_string_lossy().to_string(),
        pid: None,
        status: "installed".to_string(),
        auto_start: true,
        installed_at: now_rfc3339(),
    };
    write_info(&root, &info)?;

    println!("Daemon installed for: {}", root.display());
    Ok(())
}

pub async fn uninstall() -> Result<()> {
    let root = std::env::current_dir()?;
    let task_name = format!("speedy-watch-{}", root.to_string_lossy().replace(|c: char| !c.is_alphanumeric(), "_"));

    #[cfg(windows)]
    {
        let _ = std::process::Command::new("sc")
            .args(["stop", &task_name])
            .status();
        let _ = std::process::Command::new("sc")
            .args(["delete", &task_name])
            .status();
    }

    #[cfg(target_os = "linux")]
    {
        let service_name = format!("speedy-watch-{}", root.to_string_lossy().replace('/', "_"));
        let config_dir = dirs::config_dir()
            .context("no config dir")?
            .join("systemd")
            .join("user");
        let service_path = config_dir.join(format!("{service_name}.service"));
        let _ = std::process::Command::new("systemctl").args(["--user", "stop", &service_name]).status();
        let _ = std::process::Command::new("systemctl").args(["--user", "disable", &service_name]).status();
        let _ = std::fs::remove_file(&service_path);
        let _ = std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).status();
    }

    let json_path = daemon_json_path(&root);
    let _ = std::fs::remove_file(&json_path);

    println!("Daemon uninstalled for: {}", root.display());
    Ok(())
}

pub async fn status_cmd() -> Result<()> {
    let root = std::env::current_dir()?;

    match read_info(&root)? {
        Some(info) => {
            println!("Daemon: INSTALLED");
            println!("  Project: {}", info.project_path);
            println!("  Status:  {}", info.status);
            if let Some(pid) = info.pid {
                println!("  PID:     {pid}");
                #[cfg(windows)]
                {
                    let running = std::process::Command::new("tasklist")
                        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
                        .output()
                        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
                        .unwrap_or(false);
                    if !running {
                        println!("  (process not running, updating status)");
                        mark_stopped(&root)?;
                    }
                }
            }
            #[cfg(windows)]
            {
                let task_name = service_name_for(&root);
                let output = std::process::Command::new("sc")
                    .args(["query", &task_name])
                    .output()
                    .ok();
                if let Some(out) = output {
                    let text = String::from_utf8_lossy(&out.stdout);
                    if let Some(line) = text.lines().find(|l| l.trim().starts_with("STATE")) {
                        let state = line.split(':').nth(1).unwrap_or("").trim();
                        println!("  Service:  {state}");
                    }
                }
            }
            println!("  Auto-start: {}", info.auto_start);
            if !info.installed_at.is_empty() {
                println!("  Installed: {}", info.installed_at);
            }
        }
        None => {
            println!("Daemon: NOT INSTALLED for {}", root.display());
        }
    }
    Ok(())
}

pub async fn stop_daemon(root: &Path) -> Result<()> {
    if read_info(root)?.is_none() {
        anyhow::bail!("No daemon found for: {}", root.display());
    }

    #[cfg(windows)]
    {
        let task_name = service_name_for(root);
        let _ = std::process::Command::new("sc")
            .args(["stop", &task_name])
            .status();
    }

    #[cfg(not(windows))]
    {
        let info = read_info(root)?.unwrap();
        if let Some(pid) = info.pid {
            #[cfg(windows)]
            {
                let _ = std::process::Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/F"])
                    .output();
            }
            #[cfg(not(windows))]
            {
                let _ = std::process::Command::new("kill")
                    .args(["-9", &pid.to_string()])
                    .output();
            }
            println!("Process {} terminated", pid);
        }
    }

    mark_stopped(root)?;
    println!("Daemon stopped for: {}", root.display());
    Ok(())
}

pub async fn restart_daemon(root: &Path) -> Result<()> {
    let _ = stop_daemon(root).await;

    #[cfg(windows)]
    {
        let task_name = service_name_for(root);
        let _ = std::process::Command::new("sc")
            .args(["start", &task_name])
            .status();
    }

    #[cfg(not(windows))]
    {
        let exe = std::env::current_exe()?.canonicalize()?;
        let root_str = root.to_string_lossy().to_string();
        let _child = std::process::Command::new(&exe)
            .args(["-p", &root_str, "watch", "."])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
    }

    println!("Daemon restarted for: {}", root.display());
    Ok(())
}

pub async fn delete_daemon(root: &Path) -> Result<()> {
    let _ = stop_daemon(root).await;

    #[cfg(windows)]
    {
        let task_name = service_name_for(root);
        let _ = std::process::Command::new("sc")
            .args(["delete", &task_name])
            .status();
    }

    #[cfg(target_os = "linux")]
    {
        let service_name = format!("speedy-watch-{}", root.to_string_lossy().replace('/', "_"));
        let config_dir = dirs::config_dir()
            .context("no config dir")?
            .join("systemd")
            .join("user");
        let service_path = config_dir.join(format!("{service_name}.service"));
        let _ = std::process::Command::new("systemctl").args(["--user", "stop", &service_name]).status();
        let _ = std::process::Command::new("systemctl").args(["--user", "disable", &service_name]).status();
        let _ = std::fs::remove_file(&service_path);
        let _ = std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).status();
    }

    let json_path = daemon_json_path(root);
    let _ = std::fs::remove_file(&json_path);

    println!("Daemon deleted for: {}", root.display());
    Ok(())
}

pub async fn create_daemon(root: &Path) -> Result<()> {
    let root_str = root.to_string_lossy().to_string();

    if !crate::workspace::is_registered(&root_str) {
        anyhow::bail!("No workspace registered for: {}. Create a workspace first.", root.display());
    }

    install(Some(root_str)).await?;
    restart_daemon(root).await?;

    println!("Daemon created for: {}", root.display());
    Ok(())
}

pub async fn force_scan(root: &Path) -> Result<()> {
    let original = std::env::current_dir()?;
    std::env::set_current_dir(root)?;
    let config = crate::config::Config::from_env();
    let indexer = crate::indexer::Indexer::new(&config).await?;
    let stats = indexer.sync_all().await?;
    std::env::set_current_dir(original)?;
    println!(
        "Force scan complete: {} files, {} chunks for {}",
        stats.files, stats.chunks, root.display()
    );
    Ok(())
}

pub async fn list_all_daemons() -> Result<()> {
    let workspaces = crate::workspace::list()?;
    if workspaces.is_empty() {
        println!("No daemons found.");
        return Ok(());
    }

    for ws in &workspaces {
        let root = Path::new(&ws.path);
        println!("[{}]", ws.path);
        match read_info(root)? {
            Some(info) => {
                println!("  Status:    {}", info.status);
                if let Some(pid) = info.pid {
                    println!("  PID:       {}", pid);
                }
                println!("  Installed: {}", info.installed_at);
            }
            None => {
                println!("  Status: not installed");
            }
        }
        println!();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rfc3339_format() {
        let s = now_rfc3339();
        assert!(s.contains('T'), "expected RFC 3339 format, got: {s}");
        assert!(s.contains('Z') || s.contains('+'), "expected timezone in: {s}");
    }

    #[test]
    fn test_write_read_info_roundtrip() {
        let dir = std::env::temp_dir().join("speedy_test_daemon_info");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let info = DaemonInfo {
            project_path: dir.to_string_lossy().to_string(),
            pid: Some(12345),
            status: "running".to_string(),
            auto_start: true,
            installed_at: "2025-01-01T00:00:00Z".to_string(),
        };
        write_info(&dir, &info).unwrap();
        let read_back = read_info(&dir).unwrap().expect("should have info");
        assert_eq!(read_back.project_path, info.project_path);
        assert_eq!(read_back.pid, Some(12345));
        assert_eq!(read_back.status, "running");
        assert_eq!(read_back.auto_start, true);

        mark_stopped(&dir).unwrap();
        let stopped = read_info(&dir).unwrap().expect("should exist");
        assert_eq!(stopped.status, "stopped");
        assert_eq!(stopped.pid, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_info_nonexistent() {
        let dir = std::env::temp_dir().join("speedy_test_daemon_nonexist");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(read_info(&dir).unwrap().is_none());
    }

    #[test]
    fn test_update_status_writes_through() {
        let dir = std::env::temp_dir().join("speedy_test_daemon_update");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let info = DaemonInfo {
            project_path: dir.to_string_lossy().to_string(),
            pid: Some(42),
            status: "initial".to_string(),
            auto_start: true,
            installed_at: "2025-06-15T12:00:00Z".to_string(),
        };
        write_info(&dir, &info).unwrap();
        update_status(&dir, "updated", Some(99)).unwrap();

        let result = read_info(&dir).unwrap().unwrap();
        assert_eq!(result.status, "updated");
        assert_eq!(result.pid, Some(99));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
