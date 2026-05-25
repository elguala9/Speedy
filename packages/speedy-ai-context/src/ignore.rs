use ignore::WalkBuilder;
use std::path::Path;

pub struct FileFilter {
    root: String,
}

impl FileFilter {
    pub fn new(root: &str) -> Self {
        Self {
            root: root.to_string(),
        }
    }

    pub fn filtered_files(&self) -> Vec<String> {
        use std::sync::{Arc, Mutex};
        let files: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        WalkBuilder::new(&self.root)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .add_custom_ignore_filename(".speedyignore")
            .follow_links(false)
            .build_parallel()
            .run(|| {
                let files = Arc::clone(&files);
                Box::new(move |result| {
                    if let Ok(entry) = result {
                        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                            if let Some(path) = entry.path().to_str() {
                                files.lock().unwrap().push(path.to_string());
                            }
                        }
                    }
                    ignore::WalkState::Continue
                })
            });
        Arc::try_unwrap(files).unwrap().into_inner().unwrap()
    }

    pub fn is_binary(path: &Path) -> bool {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            return speedy_core::default_ignores::binary_extensions().contains(&ext);
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_binary_executable() {
        assert!(FileFilter::is_binary(Path::new("foo.exe")));
        assert!(FileFilter::is_binary(Path::new("bar.dll")));
        assert!(FileFilter::is_binary(Path::new("lib.so")));
    }

    #[test]
    fn test_is_binary_image() {
        assert!(FileFilter::is_binary(Path::new("photo.png")));
        assert!(FileFilter::is_binary(Path::new("image.jpg")));
        assert!(FileFilter::is_binary(Path::new("pic.jpeg")));
        assert!(FileFilter::is_binary(Path::new("icon.ico")));
        assert!(FileFilter::is_binary(Path::new("graphic.webp")));
    }

    #[test]
    fn test_is_binary_archive() {
        assert!(FileFilter::is_binary(Path::new("archive.zip")));
        assert!(FileFilter::is_binary(Path::new("bundle.tar")));
        assert!(FileFilter::is_binary(Path::new("data.gz")));
        assert!(FileFilter::is_binary(Path::new("backup.7z")));
    }

    #[test]
    fn test_is_binary_document() {
        assert!(FileFilter::is_binary(Path::new("doc.pdf")));
        assert!(FileFilter::is_binary(Path::new("report.doc")));
        assert!(FileFilter::is_binary(Path::new("sheet.xlsx")));
    }

    #[test]
    fn test_is_not_binary_source_code() {
        assert!(!FileFilter::is_binary(Path::new("main.rs")));
        assert!(!FileFilter::is_binary(Path::new("app.py")));
        assert!(!FileFilter::is_binary(Path::new("index.js")));
        assert!(!FileFilter::is_binary(Path::new("style.ts")));
        assert!(!FileFilter::is_binary(Path::new("lib.go")));
    }

    #[test]
    fn test_is_not_binary_text() {
        assert!(!FileFilter::is_binary(Path::new("readme.md")));
        assert!(!FileFilter::is_binary(Path::new("notes.txt")));
        assert!(!FileFilter::is_binary(Path::new("config.toml")));
        assert!(!FileFilter::is_binary(Path::new("data.json")));
    }

    #[test]
    fn test_is_binary_no_extension() {
        assert!(!FileFilter::is_binary(Path::new("Makefile")));
        assert!(!FileFilter::is_binary(Path::new("LICENSE")));
    }

    #[test]
    fn test_is_binary_empty_extension() {
        assert!(!FileFilter::is_binary(Path::new("file.")));
    }

    #[test]
    fn test_is_binary_media() {
        assert!(FileFilter::is_binary(Path::new("song.mp3")));
        assert!(FileFilter::is_binary(Path::new("video.mp4")));
        assert!(FileFilter::is_binary(Path::new("audio.wav")));
        assert!(FileFilter::is_binary(Path::new("track.flac")));
    }

    #[test]
    fn test_is_binary_python_bytecode() {
        assert!(FileFilter::is_binary(Path::new("module.pyc")));
        assert!(FileFilter::is_binary(Path::new("module.pyo")));
    }

    #[test]
    fn test_filtered_files_excludes_gitignored_patterns() {
        use std::fs;
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().to_str().unwrap();

        fs::write(dir.path().join("main.rs"), b"fn main() {}").unwrap();
        fs::write(dir.path().join("generated.min.js"), b"...").unwrap();
        // .speedyignore is used because git_ignore only activates inside a git repo;
        // in production, Indexer::new() copies .gitignore patterns into .speedyignore.
        fs::write(dir.path().join(".speedyignore"), b"*.min.js\n").unwrap();

        let files = FileFilter::new(root).filtered_files();

        assert!(
            files.iter().any(|f| f.ends_with("main.rs")),
            "main.rs should be included"
        );
        assert!(
            !files.iter().any(|f| f.ends_with("generated.min.js")),
            "generated.min.js should be excluded by .speedyignore (mirrors .gitignore patterns)"
        );
    }

    #[test]
    fn test_filtered_files_excludes_speedyignore_patterns() {
        use std::fs;
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().to_str().unwrap();

        fs::write(dir.path().join("keep.rs"), b"pub fn keep() {}").unwrap();
        fs::write(dir.path().join("ignore_me.log"), b"ignored log").unwrap();
        fs::write(dir.path().join(".speedyignore"), b"*.log\n").unwrap();

        let files = FileFilter::new(root).filtered_files();

        assert!(
            files.iter().any(|f| f.ends_with("keep.rs")),
            "keep.rs should be included"
        );
        assert!(
            !files.iter().any(|f| f.ends_with("ignore_me.log")),
            "ignore_me.log should be excluded by .speedyignore"
        );
    }

    #[test]
    fn test_filtered_files_excludes_directories() {
        use std::fs;
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().to_str().unwrap();

        fs::create_dir_all(dir.path().join("subdir")).unwrap();
        fs::write(dir.path().join("subdir").join("nested.rs"), b"fn f() {}").unwrap();
        fs::write(dir.path().join("top.rs"), b"fn top() {}").unwrap();

        let files = FileFilter::new(root).filtered_files();

        // Only files, not directories
        for f in &files {
            assert!(
                std::path::Path::new(f).is_file(),
                "filtered_files must return only files, got dir: {f}"
            );
        }
        assert!(files.iter().any(|f| f.ends_with("top.rs")));
        assert!(files.iter().any(|f| f.ends_with("nested.rs")));
    }
}
