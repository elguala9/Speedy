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

/// Concurrently add N entries then concurrently remove them all. The final
/// file must be valid JSON with zero entries and no corruption.
#[test]
fn test_concurrent_add_then_concurrent_remove_across_processes() {
    let exe = fixture_bin();
    assert!(exe.exists(), "workspace-fixture not built");

    let daemon_dir = unique_daemon_dir("xp_rm");
    const N: usize = 6;

    // Sequential adds first (so removes have something to remove)
    for i in 0..N {
        let status = quiet_command(&exe)
            .args(["add", &format!("/rm-base-{i}")])
            .env("SPEEDY_DAEMON_DIR", &daemon_dir)
            .output()
            .expect("spawn add")
            .status;
        assert!(status.success(), "pre-add {i} failed with {status}");
    }

    // Concurrent removes
    let mut children: Vec<_> = (0..N)
        .map(|i| {
            quiet_command(&exe)
                .args(["remove", &format!("/rm-base-{i}")])
                .env("SPEEDY_DAEMON_DIR", &daemon_dir)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("spawn remove")
        })
        .collect();

    for (i, child) in children.iter_mut().enumerate() {
        let status = child.wait().expect("wait remove");
        assert!(status.success(), "remove #{i} failed with {status}");
    }

    let content = std::fs::read_to_string(daemon_dir.join("workspaces.json")).unwrap();
    let entries: Vec<serde_json::Value> = serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("corrupt JSON after concurrent removes: {e}\n{content}"));
    assert_eq!(entries.len(), 0, "all entries removed: {content}");

    let _ = std::fs::remove_dir_all(&daemon_dir);
}

/// Spawn concurrent adds and concurrent removes at the same time.
/// The final JSON must be well-formed regardless of interleaving.
#[test]
fn test_concurrent_add_and_remove_interleaved_stays_consistent() {
    let exe = fixture_bin();
    assert!(exe.exists(), "workspace-fixture not built");

    let daemon_dir = unique_daemon_dir("xp_interleave");
    const PRE: usize = 4;
    const NEW: usize = 4;

    // Pre-populate entries that will be removed concurrently
    for i in 0..PRE {
        let status = quiet_command(&exe)
            .args(["add", &format!("/interleave-pre-{i}")])
            .env("SPEEDY_DAEMON_DIR", &daemon_dir)
            .output()
            .expect("spawn pre-add")
            .status;
        assert!(status.success(), "pre-add {i} failed");
    }

    // Concurrent: PRE removes + NEW adds
    let mut handles: Vec<_> = (0..PRE)
        .map(|i| {
            quiet_command(&exe)
                .args(["remove", &format!("/interleave-pre-{i}")])
                .env("SPEEDY_DAEMON_DIR", &daemon_dir)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("spawn remove")
        })
        .collect();

    handles.extend((0..NEW).map(|i| {
        quiet_command(&exe)
            .args(["add", &format!("/interleave-new-{i}")])
            .env("SPEEDY_DAEMON_DIR", &daemon_dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn add")
    }));

    for h in &mut handles {
        let _ = h.wait();
    }

    // Final state must be valid JSON (integrity check)
    let content = std::fs::read_to_string(daemon_dir.join("workspaces.json")).unwrap();
    let entries: Vec<serde_json::Value> = serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("corrupt JSON after interleaved ops: {e}\n{content}"));
    // All NEW adds should have succeeded; PRE removes may or may not have all
    // succeeded depending on ordering. The key invariant is valid JSON.
    assert_eq!(
        entries.len(),
        NEW,
        "expected {NEW} entries (adds) after all removes finished: {content}"
    );

    let _ = std::fs::remove_dir_all(&daemon_dir);
}

/// Write malformed JSON into workspaces.json, then call `workspace-fixture list`.
/// The fixture must exit with failure (not panic / SIGSEGV) — recovery from a
/// corrupt file is a controlled error, not a crash.
#[test]
fn test_workspace_json_recovery_from_malformed() {
    let exe = fixture_bin();
    assert!(exe.exists(), "workspace-fixture not built");

    let daemon_dir = unique_daemon_dir("xp_malformed");

    // Write invalid JSON directly into the workspaces file.
    let ws_path = daemon_dir.join("workspaces.json");
    std::fs::write(&ws_path, b"{this is not valid json!!!").expect("write malformed json");

    // `list` must fail gracefully — exit with a non-zero code, not a panic/crash.
    let status = quiet_command(&exe)
        .args(["list"])
        .env("SPEEDY_DAEMON_DIR", &daemon_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("spawn fixture");

    assert!(
        !status.success(),
        "fixture should exit with failure on malformed JSON, but exited successfully"
    );

    let _ = std::fs::remove_dir_all(&daemon_dir);
}

/// Add two workspaces: one whose directory exists on disk, one whose does not.
/// After calling `prune`, the missing entry must be gone and the existing one
/// must remain.
#[test]
fn test_prune_missing_with_deleted_directory() {
    let exe = fixture_bin();
    assert!(exe.exists(), "workspace-fixture not built");

    let daemon_dir = unique_daemon_dir("xp_prune");

    // Workspace A: a real temp directory that will survive the prune.
    let real_dir = daemon_dir.join("real_ws");
    std::fs::create_dir_all(&real_dir).expect("create real_ws");
    let real_path = real_dir.to_str().expect("valid utf-8").to_string();

    // Workspace B: path that does not exist on disk.
    let ghost_path = daemon_dir.join("ghost_ws").to_str().expect("valid utf-8").to_string();

    // Add both.
    for path in [&real_path, &ghost_path] {
        let status = quiet_command(&exe)
            .args(["add", path])
            .env("SPEEDY_DAEMON_DIR", &daemon_dir)
            .output()
            .expect("spawn add")
            .status;
        assert!(status.success(), "add {path} failed with {status}");
    }

    // Run prune.
    let output = quiet_command(&exe)
        .args(["prune"])
        .env("SPEEDY_DAEMON_DIR", &daemon_dir)
        .output()
        .expect("spawn prune");
    assert!(
        output.status.success(),
        "prune failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("pruned:1"),
        "expected pruned:1, got: {stdout}"
    );

    // List remaining entries — only the real path should survive.
    let list_out = quiet_command(&exe)
        .args(["list"])
        .env("SPEEDY_DAEMON_DIR", &daemon_dir)
        .output()
        .expect("spawn list");
    assert!(list_out.status.success(), "list failed after prune");
    let listed = String::from_utf8_lossy(&list_out.stdout);
    assert!(
        listed.contains(&real_path),
        "real workspace should remain after prune, listed: {listed}"
    );
    assert!(
        !listed.contains(&ghost_path),
        "ghost workspace should have been pruned, listed: {listed}"
    );

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
