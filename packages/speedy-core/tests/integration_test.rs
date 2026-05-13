use std::path::Path;
use std::process::Command;

const BINARY: &str = if cfg!(windows) { "speedy.exe" } else { "speedy" };

fn speedy_binary() -> String {
    let mut path = std::env::current_dir().unwrap();
    loop {
        let candidate = path.join("target").join("debug").join(BINARY);
        if candidate.exists() {
            return candidate.to_string_lossy().to_string();
        }
        if !path.pop() {
            panic!("Could not find speedy binary in target/debug/");
        }
    }
}

fn run_speedy(args: &[&str], cwd: &Path) -> Result<String, String> {
    let output = Command::new(speedy_binary())
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| format!("failed to execute: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() {
        Ok(stdout)
    } else {
        Err(format!("exit {}: [stdout] {stdout} [stderr] {stderr}", output.status))
    }
}

#[test]
fn test_help_contains_usage() {
    let dir = std::env::temp_dir().join("speedy_int_help");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let out = run_speedy(&["--help"], &dir).unwrap();
    assert!(out.contains("Usage") || out.contains("speedy"), "help should contain usage info");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_version_output() {
    let dir = std::env::temp_dir().join("speedy_int_version");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let out = run_speedy(&["--version"], &dir).unwrap();
    assert!(!out.is_empty(), "version output should not be empty");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_no_command_shows_error() {
    let dir = std::env::temp_dir().join("speedy_int_nocmd");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let result = run_speedy(&[], &dir);
    assert!(result.is_err(), "no command should return an error");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_workspace_list_empty() {
    let dir = std::env::temp_dir().join("speedy_int_wslist");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let result = run_speedy(&["--workspaces"], &dir);
    assert!(result.is_ok(), "workspaces flag failed: {:?}", result.err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_daemons_list_empty() {
    let dir = std::env::temp_dir().join("speedy_int_daemonlist");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let result = run_speedy(&["--daemons"], &dir);
    assert!(result.is_ok(), "daemons flag failed: {:?}", result.err());
    let _ = std::fs::remove_dir_all(&dir);
}
