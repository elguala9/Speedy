use sha2::{Digest, Sha256};
use std::path::Path;
use tokio::fs;

pub fn hash_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    format!("{result:x}")
}

pub async fn hash_file(path: &Path) -> anyhow::Result<String> {
    use anyhow::Context;
    let content = fs::read(path).await
        .context(format!("failed to read file for hashing: {}", path.display()))?;
    Ok(hash_bytes(&content))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_bytes_deterministic() {
        let h1 = hash_bytes(b"hello world");
        let h2 = hash_bytes(b"hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_bytes_different_inputs() {
        let h1 = hash_bytes(b"hello");
        let h2 = hash_bytes(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_hash_bytes_empty() {
        let h = hash_bytes(b"");
        assert_eq!(h.len(), 64);
    }

    #[test]
    fn test_hash_bytes_known() {
        let h = hash_bytes(b"hello");
        assert_eq!(h, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
    }

    #[test]
    fn test_hash_consistency() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let dir = std::env::temp_dir().join("speedy_test_hash");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("test.txt");
        std::fs::write(&file, b"hello world").unwrap();

        let h1 = rt.block_on(hash_file(&file)).unwrap();
        let h2 = rt.block_on(hash_file(&file)).unwrap();
        assert_eq!(h1, h2);

        std::fs::write(&file, b"hello world!").unwrap();
        let h3 = rt.block_on(hash_file(&file)).unwrap();
        assert_ne!(h1, h3);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
