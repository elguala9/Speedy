use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

pub struct FileHashEntry {
    pub hash: String,
    pub mtime_secs: u64,
}

/// Shared hash registry stored at `<workspace>/.speedy/hashes.sqlite`.
///
/// Each context (ai-context, language-context, text, …) writes its own rows —
/// `(file_path, context)` is the primary key. Adding a new tool requires only
/// choosing a new context string; no schema changes needed.
pub struct HashRegistry {
    conn: Connection,
    workspace: PathBuf,
}

impl HashRegistry {
    pub fn open(workspace: &Path) -> Result<Self> {
        let speedy_dir = workspace.join(".speedy");
        std::fs::create_dir_all(&speedy_dir)
            .context("failed to create .speedy directory")?;
        let db_path = speedy_dir.join("hashes.sqlite");
        let conn = Connection::open(&db_path)
            .context("failed to open hashes.sqlite")?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS file_hashes (
                 file_path  TEXT    NOT NULL,
                 context    TEXT    NOT NULL,
                 hash       TEXT    NOT NULL,
                 mtime_secs INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 PRIMARY KEY (file_path, context)
             );
             CREATE INDEX IF NOT EXISTS idx_fh_file ON file_hashes(file_path);",
        )
        .context("failed to initialize hashes.sqlite")?;
        Ok(Self { conn, workspace: workspace.to_path_buf() })
    }

    fn to_rel(&self, abs_file: &Path) -> String {
        abs_file
            .strip_prefix(&self.workspace)
            .unwrap_or(abs_file)
            .to_string_lossy()
            .replace('\\', "/")
    }

    /// Daemon pre-check: only reads file metadata (no file content read).
    /// Returns `true` if the stored mtime matches the current mtime → safe to
    /// skip spawning the subprocess.
    pub fn mtime_unchanged(&self, abs_file: &Path, context: &str) -> bool {
        let mtime = match file_mtime_secs(abs_file) {
            Some(m) => m,
            None => return false,
        };
        let rel = self.to_rel(abs_file);
        let stored: Option<u64> = self
            .conn
            .query_row(
                "SELECT mtime_secs FROM file_hashes WHERE file_path = ?1 AND context = ?2",
                params![rel, context],
                |row| row.get(0),
            )
            .ok();
        stored == Some(mtime)
    }

    /// Full change check: mtime fast-path → hash comparison.
    /// Returns `true` if the file needs re-indexing.
    /// If mtime changed but content is identical, updates the stored mtime and
    /// returns `false` (avoids a redundant re-index on trivial metadata bumps).
    pub fn needs_reindex(&self, abs_file: &Path, context: &str) -> Result<bool> {
        let rel = self.to_rel(abs_file);
        let mtime = file_mtime_secs(abs_file).unwrap_or(0);

        let stored: Option<(String, u64)> = self
            .conn
            .query_row(
                "SELECT hash, mtime_secs FROM file_hashes WHERE file_path = ?1 AND context = ?2",
                params![rel, context],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();

        let Some((stored_hash, stored_mtime)) = stored else {
            return Ok(true); // never indexed
        };

        if mtime == stored_mtime {
            return Ok(false); // fast path — no file read needed
        }

        // mtime changed — verify content hash before triggering a re-index
        let content = match std::fs::read(abs_file) {
            Ok(b) => b,
            Err(_) => return Ok(true),
        };
        let current_hash = hash_bytes(&content);

        if current_hash == stored_hash {
            // content unchanged despite mtime bump — just refresh the stored mtime
            self.conn.execute(
                "UPDATE file_hashes SET mtime_secs = ?1, updated_at = ?2 \
                 WHERE file_path = ?3 AND context = ?4",
                params![mtime, now_secs(), rel, context],
            )?;
            return Ok(false);
        }

        Ok(true)
    }

    /// Record that a file was successfully indexed for `context`.
    /// Call this after the indexing work completes without errors.
    pub fn set_indexed(
        &self,
        abs_file: &Path,
        context: &str,
        hash: &str,
        mtime_secs: u64,
    ) -> Result<()> {
        let rel = self.to_rel(abs_file);
        self.conn.execute(
            "INSERT INTO file_hashes (file_path, context, hash, mtime_secs, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(file_path, context) DO UPDATE SET
                 hash       = excluded.hash,
                 mtime_secs = excluded.mtime_secs,
                 updated_at = excluded.updated_at",
            params![rel, context, hash, mtime_secs, now_secs()],
        )?;
        Ok(())
    }

    /// Remove all registry entries for a file (all contexts).
    /// Call when a file is deleted from disk.
    pub fn on_file_removed(&self, abs_file: &Path) -> Result<()> {
        let rel = self.to_rel(abs_file);
        self.conn.execute(
            "DELETE FROM file_hashes WHERE file_path = ?1",
            params![rel],
        )?;
        Ok(())
    }

    /// Wipe all entries for a context before a full re-index.
    pub fn delete_context(&self, context: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM file_hashes WHERE context = ?1",
            params![context],
        )?;
        Ok(())
    }

    /// Return all entries for a context (for status/diagnostics).
    pub fn get_all_for_context(&self, context: &str) -> Result<HashMap<String, FileHashEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT file_path, hash, mtime_secs FROM file_hashes WHERE context = ?1",
        )?;
        let rows = stmt.query_map(params![context], |row| {
            Ok((
                row.get::<_, String>(0)?,
                FileHashEntry {
                    hash: row.get(1)?,
                    mtime_secs: row.get::<_, u64>(2)?,
                },
            ))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (path, entry) = row?;
            map.insert(path, entry);
        }
        Ok(map)
    }

    /// Compute SHA256 of a file and return `(hex_hash, mtime_secs)`.
    pub fn hash_file(path: &Path) -> Result<(String, u64)> {
        let content = std::fs::read(path)
            .with_context(|| format!("failed to read file: {}", path.display()))?;
        let h = hash_bytes(&content);
        let mtime = file_mtime_secs(path).unwrap_or(0);
        Ok((h, mtime))
    }

    /// SHA256 of raw bytes, returned as lowercase hex (64 chars).
    pub fn hash_bytes(data: &[u8]) -> String {
        hash_bytes(data)
    }
}

// --- module-level helpers (not pub) ---

pub(crate) fn hash_bytes(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    format!("{digest:x}")
}

fn file_mtime_secs(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_workspace() -> tempfile::TempDir { tempfile::tempdir().unwrap() }

    #[test]
    fn test_new_file_needs_reindex() {
        let ws = tmp_workspace();
        let file = ws.path().join("foo.rs");
        std::fs::write(&file, b"fn main() {}").unwrap();
        let reg = HashRegistry::open(ws.path()).unwrap();
        assert!(reg.needs_reindex(&file, "test").unwrap());
    }

    #[test]
    fn test_set_then_no_reindex() {
        let ws = tmp_workspace();
        let file = ws.path().join("bar.rs");
        std::fs::write(&file, b"fn foo() {}").unwrap();
        let reg = HashRegistry::open(ws.path()).unwrap();
        let (hash, mtime) = HashRegistry::hash_file(&file).unwrap();
        reg.set_indexed(&file, "test", &hash, mtime).unwrap();
        assert!(!reg.needs_reindex(&file, "test").unwrap());
    }

    #[test]
    fn test_content_change_triggers_reindex() {
        let ws = tmp_workspace();
        let file = ws.path().join("baz.rs");
        std::fs::write(&file, b"v1").unwrap();
        let reg = HashRegistry::open(ws.path()).unwrap();
        let (hash_v1, mtime) = HashRegistry::hash_file(&file).unwrap();

        // Store with a stale mtime (1s in the past) so the mtime fast-path
        // never triggers and the hash comparison path is always exercised.
        reg.set_indexed(&file, "test", &hash_v1, mtime.saturating_sub(1)).unwrap();

        // Write v2 — hash changes, stored hash is still v1's
        std::fs::write(&file, b"v2").unwrap();
        assert!(reg.needs_reindex(&file, "test").unwrap());
    }

    #[test]
    fn test_mtime_unchanged() {
        let ws = tmp_workspace();
        let file = ws.path().join("x.rs");
        std::fs::write(&file, b"hello").unwrap();
        let reg = HashRegistry::open(ws.path()).unwrap();
        assert!(!reg.mtime_unchanged(&file, "test")); // no entry yet
        let (hash, mtime) = HashRegistry::hash_file(&file).unwrap();
        reg.set_indexed(&file, "test", &hash, mtime).unwrap();
        assert!(reg.mtime_unchanged(&file, "test")); // entry exists with same mtime
    }

    #[test]
    fn test_delete_context() {
        let ws = tmp_workspace();
        let file = ws.path().join("y.rs");
        std::fs::write(&file, b"data").unwrap();
        let reg = HashRegistry::open(ws.path()).unwrap();
        let (hash, mtime) = HashRegistry::hash_file(&file).unwrap();
        reg.set_indexed(&file, "ctx-a", &hash, mtime).unwrap();
        reg.set_indexed(&file, "ctx-b", &hash, mtime).unwrap();
        reg.delete_context("ctx-a").unwrap();
        assert!(reg.needs_reindex(&file, "ctx-a").unwrap());
        assert!(!reg.needs_reindex(&file, "ctx-b").unwrap());
    }

    #[test]
    fn test_contexts_are_independent() {
        let ws = tmp_workspace();
        let file = ws.path().join("z.rs");
        std::fs::write(&file, b"shared").unwrap();
        let reg = HashRegistry::open(ws.path()).unwrap();
        let (hash, mtime) = HashRegistry::hash_file(&file).unwrap();
        reg.set_indexed(&file, "ctx-1", &hash, mtime).unwrap();
        // ctx-2 never indexed — should still need reindex
        assert!(!reg.needs_reindex(&file, "ctx-1").unwrap());
        assert!(reg.needs_reindex(&file, "ctx-2").unwrap());
    }

    #[test]
    fn test_hash_bytes_known_value() {
        let h = hash_bytes(b"hello");
        assert_eq!(h, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
    }
}
