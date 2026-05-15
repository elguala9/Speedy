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
        let mut files = Vec::new();
        let walker = WalkBuilder::new(&self.root)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .add_custom_ignore_filename(".speedyignore")
            .follow_links(false)
            .max_depth(None)
            .build();

        for entry in walker.flatten() {
            if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                if let Some(path) = entry.path().to_str() {
                    files.push(path.to_string());
                }
            }
        }
        files
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
}
