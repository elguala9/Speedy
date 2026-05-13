use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    pub path: String,
    pub created_at: String,
}

fn workspaces_path() -> Result<PathBuf> {
    let config = dirs::config_dir()
        .context("no config directory found")?
        .join("speedy");
    std::fs::create_dir_all(&config)
        .context("failed to create speedy config directory")?;
    Ok(config.join("workspaces.json"))
}

pub fn list() -> Result<Vec<WorkspaceEntry>> {
    let path = workspaces_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .context(format!("failed to read workspaces file: {}", path.display()))?;
    Ok(serde_json::from_str(&content)
        .context("failed to parse workspaces file")?)
}

fn save(workspaces: &[WorkspaceEntry]) -> Result<()> {
    let path = workspaces_path()?;
    let content = serde_json::to_string_pretty(workspaces)
        .context("failed to serialize workspaces")?;
    std::fs::write(&path, content)
        .context(format!("failed to write workspaces file: {}", path.display()))?;
    Ok(())
}

pub fn add(path: &str) -> Result<()> {
    let mut workspaces = list()?;
    if workspaces.iter().any(|w| w.path == path) {
        anyhow::bail!("Workspace already exists: {}", path);
    }
    workspaces.push(WorkspaceEntry {
        path: path.to_string(),
        created_at: crate::daemon::now_rfc3339(),
    });
    save(&workspaces)
}

pub fn remove(path: &str) -> Result<()> {
    let mut workspaces = list()?;
    let before = workspaces.len();
    workspaces.retain(|w| w.path != path);
    if workspaces.len() == before {
        anyhow::bail!("Workspace not found: {}", path);
    }
    save(&workspaces)
}

pub fn is_registered(path: &str) -> bool {
    list().ok().map_or(false, |w| w.iter().any(|e| e.path == path))
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
            save(&ws).unwrap();
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
