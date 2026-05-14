use anyhow::{Context, Result};
use fd_lock::RwLock as FileLock;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    pub path: String,
    pub created_at: String,
}

fn workspace_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn workspaces_path() -> Result<PathBuf> {
    let dir = crate::daemon_util::daemon_dir_path()?;
    std::fs::create_dir_all(&dir)
        .context("failed to create speedy config directory")?;
    Ok(dir.join("workspaces.json"))
}

fn lock_file_path() -> Result<PathBuf> {
    Ok(workspaces_path()?.with_extension("lock"))
}

/// Run `f` while holding an exclusive cross-process lock on the workspaces
/// file. The intra-process `Mutex` is taken first so threads contend cheaply
/// before reaching for the OS lock.
fn with_file_lock<R>(f: impl FnOnce() -> Result<R>) -> Result<R> {
    let _proc_lock = workspace_lock().lock().unwrap_or_else(|e| e.into_inner());
    let path = lock_file_path()?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&path)
        .with_context(|| format!("failed to open workspaces lock: {}", path.display()))?;
    let mut lock = FileLock::new(file);
    let _guard = lock.write().context("failed to acquire workspaces lock")?;
    f()
}

fn list_unlocked() -> Result<Vec<WorkspaceEntry>> {
    let path = workspaces_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .context(format!("failed to read workspaces file: {}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    Ok(serde_json::from_str(&content)
        .context("failed to parse workspaces file")?)
}

fn save_unlocked(workspaces: &[WorkspaceEntry]) -> Result<()> {
    let path = workspaces_path()?;
    let content = serde_json::to_string_pretty(workspaces)
        .context("failed to serialize workspaces")?;
    std::fs::write(&path, content)
        .context(format!("failed to write workspaces file: {}", path.display()))?;
    Ok(())
}

pub fn list() -> Result<Vec<WorkspaceEntry>> {
    with_file_lock(list_unlocked)
}

pub fn add(path: &str) -> Result<()> {
    with_file_lock(|| {
        let mut workspaces = list_unlocked()?;
        if workspaces.iter().any(|w| w.path == path) {
            anyhow::bail!("Workspace already exists: {}", path);
        }
        workspaces.push(WorkspaceEntry {
            path: path.to_string(),
            created_at: crate::daemon_util::now_rfc3339(),
        });
        save_unlocked(&workspaces)
    })
}

pub fn remove(path: &str) -> Result<()> {
    with_file_lock(|| {
        let mut workspaces = list_unlocked()?;
        let before = workspaces.len();
        workspaces.retain(|w| w.path != path);
        if workspaces.len() == before {
            anyhow::bail!("Workspace not found: {}", path);
        }
        save_unlocked(&workspaces)
    })
}

pub fn is_registered(path: &str) -> bool {
    with_file_lock(|| Ok(list_unlocked()?.iter().any(|e| e.path == path)))
        .unwrap_or(false)
}

/// Drop entries whose `path` no longer exists on disk. Returns the number of
/// entries that were pruned.
pub fn prune_missing() -> Result<usize> {
    with_file_lock(|| {
        let workspaces = list_unlocked()?;
        let before = workspaces.len();
        let kept: Vec<WorkspaceEntry> = workspaces
            .into_iter()
            .filter(|w| Path::new(&w.path).exists())
            .collect();
        let pruned = before - kept.len();
        if pruned > 0 {
            save_unlocked(&kept)?;
        }
        Ok(pruned)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn backup_and_clear() -> Option<Vec<WorkspaceEntry>> {
        let current = list().ok();
        let path = workspaces_path().unwrap();
        let _ = std::fs::remove_file(&path);
        current
    }

    fn restore(backup: Option<Vec<WorkspaceEntry>>) {
        let path = workspaces_path().unwrap();
        if let Some(ws) = backup {
            save_unlocked(&ws).unwrap();
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }

    #[test]
    fn test_list_empty_when_no_file() {
        let _lock = LOCK.lock().unwrap();
        let backup = backup_and_clear();
        let ws = list().unwrap();
        assert!(ws.is_empty());
        restore(backup);
    }

    #[test]
    fn test_add_and_list() {
        let _lock = LOCK.lock().unwrap();
        let backup = backup_and_clear();
        add("C:\\test-path").unwrap();
        let ws = list().unwrap();
        assert_eq!(ws.len(), 1);
        assert!(!ws[0].created_at.is_empty());
        restore(backup);
    }

    #[test]
    fn test_add_duplicate_errors() {
        let _lock = LOCK.lock().unwrap();
        let backup = backup_and_clear();
        add("C:\\dup-path").unwrap();
        let err = add("C:\\dup-path").unwrap_err();
        assert!(err.to_string().contains("already exists"));
        restore(backup);
    }

    #[test]
    fn test_remove() {
        let _lock = LOCK.lock().unwrap();
        let backup = backup_and_clear();
        add("C:\\rem-path").unwrap();
        remove("C:\\rem-path").unwrap();
        let ws = list().unwrap();
        assert!(ws.is_empty());
        restore(backup);
    }

    #[test]
    fn test_remove_nonexistent_errors() {
        let _lock = LOCK.lock().unwrap();
        let backup = backup_and_clear();
        let err = remove("C:\\nope").unwrap_err();
        assert!(err.to_string().contains("not found"));
        restore(backup);
    }

    #[test]
    fn test_is_registered() {
        let _lock = LOCK.lock().unwrap();
        let backup = backup_and_clear();
        assert!(!is_registered("C:\\reg-path"));
        add("C:\\reg-path").unwrap();
        assert!(is_registered("C:\\reg-path"));
        assert!(!is_registered("C:\\other-path"));
        restore(backup);
    }

    /// Spawn N OS threads that each `add` a distinct path concurrently. The
    /// intra-process `Mutex` + cross-process `fd-lock` should serialize them
    /// so the resulting `workspaces.json` is valid JSON and contains exactly N
    /// entries with no corruption / missing writes.
    #[test]
    fn test_concurrent_add_no_corruption() {
        let _lock = LOCK.lock().unwrap();
        let backup = backup_and_clear();

        const N: usize = 8;
        let mut handles = Vec::with_capacity(N);
        for i in 0..N {
            handles.push(std::thread::spawn(move || {
                add(&format!("C:\\concurrent-ws-{i}"))
            }));
        }
        for h in handles {
            h.join().unwrap().unwrap();
        }

        let ws = list().unwrap();
        assert_eq!(ws.len(), N, "expected {N} workspaces after concurrent adds, got {}", ws.len());
        // All entries are distinct (no merging artefacts).
        let mut paths: Vec<String> = ws.iter().map(|w| w.path.clone()).collect();
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), N, "duplicates appeared after concurrent add");

        restore(backup);
    }

    #[test]
    fn test_add_multiple() {
        let _lock = LOCK.lock().unwrap();
        let backup = backup_and_clear();
        add("C:\\a").unwrap();
        add("C:\\b").unwrap();
        add("C:\\c").unwrap();
        let ws = list().unwrap();
        assert_eq!(ws.len(), 3);
        restore(backup);
    }
}
