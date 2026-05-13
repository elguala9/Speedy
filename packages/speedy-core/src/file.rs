use std::path::Path;

pub fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
}

pub fn is_source_file(path: &Path) -> bool {
    matches!(
        extension(path).as_deref(),
        Some(e) if matches!(e, "rs" | "py" | "js" | "ts" | "go" | "java" | "c" | "cpp" | "h" | "hpp")
    )
}

pub fn is_config_file(path: &Path) -> bool {
    matches!(
        extension(path).as_deref(),
        Some(e) if matches!(e, "toml" | "json" | "yaml" | "yml" | "ini" | "cfg")
    )
}

pub fn is_documentation(path: &Path) -> bool {
    matches!(
        extension(path).as_deref(),
        Some(e) if matches!(e, "md" | "rst" | "txt" | "pdf" | "html")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_extension_rs() {
        assert_eq!(extension(Path::new("foo.rs")), Some("rs".to_string()));
    }

    #[test]
    fn test_extension_no_ext() {
        assert_eq!(extension(Path::new("Makefile")), None);
    }

    #[test]
    fn test_is_source_file() {
        assert!(is_source_file(Path::new("main.rs")));
        assert!(is_source_file(Path::new("app.py")));
        assert!(!is_source_file(Path::new("readme.md")));
    }

    #[test]
    fn test_is_config_file() {
        assert!(is_config_file(Path::new("Cargo.toml")));
        assert!(is_config_file(Path::new("config.json")));
        assert!(!is_config_file(Path::new("main.rs")));
    }

    #[test]
    fn test_is_documentation() {
        assert!(is_documentation(Path::new("README.md")));
        assert!(is_documentation(Path::new("doc.txt")));
        assert!(!is_documentation(Path::new("main.rs")));
    }
}
