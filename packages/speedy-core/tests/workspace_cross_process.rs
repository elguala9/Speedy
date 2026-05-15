//! Cross-process integration test for the `workspaces.json` file lock.
//!
//! The in-process test (`workspace::tests::test_concurrent_add_no_corruption`)
//! only proves the intra-process `Mutex` works. This one fires up N actual
//! `workspace-fixture` subprocesses against the same isolated daemon dir to
//! prove `fd_lock::RwLock` serializes them across processes.

use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn fixture_bin() -> PathBuf {
    let suffix = std::env::consts::EXE_SUFFIX;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .join("target").join("debug");
    root.join(format!("workspace-fixture{suffix}"))
}

fn quiet_command(exe: &Path) -> Command {
    let cmd = Command::new(exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut c = cmd;
        c.creation_flags(CREATE_NO_WINDOW);
        return c;
    }
    #[cfg(not(windows))]
    cmd
}

fn unique_daemon_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let d = std::env::temp_dir().join(format!(
        "speedy_xp_ws_{label}_{}_{nanos}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn test_concurrent_add_across_processes_no_corruption() {
    let exe = fixture_bin();
    assert!(
        exe.exists(),
        "workspace-fixture binary missing at {}. Build with: cargo build -p testexe --bin workspace-fixture",
        exe.display()
    );

    let daemon_dir = unique_daemon_dir("xp_add");

    // Fan out N processes, each adding a unique path. Spawn first, wait all.
    const N: usize = 8;
    let mut children: Vec<_> = (0..N)
        .map(|i| {
            quiet_command(&exe)
                .args(["add", &format!("/cross-process-ws-{i}")])
                .env("SPEEDY_DAEMON_DIR", &daemon_dir)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("spawn fixture")
        })
        .collect();

    for (i, child) in children.iter_mut().enumerate() {
        let status = child.wait().expect("wait fixture");
        assert!(
            status.success(),
            "fixture #{i} failed with status {status}"
        );
    }

    // Verify the persisted file is well-formed JSON with exactly N entries —
    // no lost writes, no torn JSON, no duplicates.
    let ws_path = daemon_dir.join("workspaces.json");
    let content = std::fs::read_to_string(&ws_path)
        .expect("workspaces.json missing after concurrent adds");
    let entries: Vec<serde_json::Value> = serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("workspaces.json is not valid JSON ({e}): {content}"));
    assert_eq!(
        entries.len(),
        N,
        "expected {N} entries, got {}: {content}",
        entries.len()
    );

    let mut paths: Vec<String> = entries
        .iter()
        .filter_map(|e| e.get("path").and_then(|p| p.as_str()).map(String::from))
        .collect();
    paths.sort();
    paths.dedup();
    assert_eq!(paths.len(), N, "duplicate paths after concurrent adds: {paths:?}");

    let _ = std::fs::remove_dir_all(&daemon_dir);
}

/// Mix readers (`list`) and writers (`add`) across processes. The reader
/// always reads valid JSON — if the lock leaked, a reader could occasionally
/// observe a half-written file and panic on parse.
#[test]
fn test_mixed_read_write_across_processes_stays_consistent() {
    let exe = fixture_bin();
    assert!(exe.exists(), "workspace-fixture not built");

    let daemon_dir = unique_daemon_dir("xp_mix");

    const WRITERS: usize = 4;
    const READERS: usize = 4;

    let mut handles: Vec<_> = (0..WRITERS)
        .map(|i| {
            quiet_command(&exe)
                .args(["add", &format!("/mixed-rw-{i}")])
                .env("SPEEDY_DAEMON_DIR", &daemon_dir)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("spawn writer")
        })
        .collect();

    handles.extend((0..READERS).map(|_| {
        quiet_command(&exe)
            .args(["list"])
            .env("SPEEDY_DAEMON_DIR", &daemon_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn reader")
    }));

    for h in &mut handles {
        let status = h.wait().expect("wait child");
        assert!(status.success(), "child failed: {status}");
    }

    // Final state must be valid JSON with exactly WRITERS entries.
    let content = std::fs::read_to_string(daemon_dir.join("workspaces.json")).unwrap();
    let entries: Vec<serde_json::Value> = serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("torn workspaces.json: {e}\n{content}"));
    assert_eq!(entries.len(), WRITERS, "{content}");

    let _ = std::fs::remove_dir_all(&daemon_dir);
}
