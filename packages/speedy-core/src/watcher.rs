use crate::config::Config;
use crate::indexer::Indexer;
use anyhow::Result;
use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::Mutex;

pub async fn start_watcher(path: &str, config: &Config) -> Result<()> {
    let running = Arc::new(AtomicBool::new(true));
    let run = running.clone();
    ctrlc::set_handler(move || {
        println!("\nShutting down watcher...");
        run.store(false, Ordering::SeqCst);
    }).ok();
    run_watcher_loop(path, config, running).await
}

pub async fn start_service_watcher(path: &str, config: &Config, running: Arc<AtomicBool>) -> Result<()> {
    run_watcher_loop(path, config, running).await
}

async fn run_watcher_loop(path: &str, config: &Config, running: Arc<AtomicBool>) -> Result<()> {
    let indexer = Arc::new(Mutex::new(Indexer::new(config).await?));

    let pid = std::process::id();
    let cwd = std::env::current_dir()?;
    let _ = crate::daemon::save_pid(&cwd, pid);
    eprintln!("Watcher started (PID: {pid})");

    let (tx, rx) = std::sync::mpsc::channel();
    let mut debouncer = new_debouncer(Duration::from_millis(500), tx)?;

    debouncer
        .watcher()
        .watch(Path::new(path), RecursiveMode::Recursive)?;

    println!("Watching {path} for changes...");

    let result: Result<()> = (|| {
        while running.load(Ordering::SeqCst) {
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(result) => {
                    match result {
                        Ok(events) => {
                            let idx = indexer.clone();
                            tokio::spawn(async move {
                                for event in events {
                                    if let Err(e) = handle_event(&idx, &event.path).await {
                                        eprintln!("Error handling event: {e}");
                                    }
                                }
                            });
                        }
                        Err(e) => {
                            eprintln!("Watch error: {e}");
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        Ok(())
    })();

    let _ = crate::daemon::mark_stopped(&cwd);
    println!("Watcher stopped.");
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_skip_extensions_known_binary() {
        let skip_exts = [
            "exe", "dll", "so", "dylib", "bin", "obj", "o", "a", "lib",
            "png", "jpg", "jpeg", "gif", "bmp", "ico", "webp",
            "mp3", "mp4", "avi", "mov", "mkv", "wav", "flac", "ogg",
            "zip", "tar", "gz", "bz2", "7z", "rar",
        ];
        for ext in &skip_exts {
            let fname = format!("test.{ext}");
            let p = std::path::Path::new(&fname);
            assert!(p.extension().and_then(|e| e.to_str()).is_some(), "{ext} should have extension");
        }
    }

    #[test]
    fn test_text_extensions_not_filtered() {
        let ok_exts = ["rs", "py", "js", "ts", "md", "txt", "toml", "json", "yaml"];
        let skip = ["exe", "dll", "so", "dylib", "bin", "obj", "o", "a", "lib",
            "png", "jpg", "jpeg", "gif", "bmp", "ico", "webp",
            "mp3", "mp4", "avi", "mov", "mkv", "wav", "flac", "ogg",
            "zip", "tar", "gz", "bz2", "7z", "rar"];
        for ext in &ok_exts {
            let fname = format!("test.{ext}");
            let p = std::path::Path::new(&fname);
            let is_binary = p.extension().and_then(|e| e.to_str()).map_or(false, |e| skip.contains(&e));
            assert!(!is_binary, "{ext} should not be treated as binary");
        }
    }
}

async fn handle_event(indexer: &Arc<Mutex<Indexer>>, path: &Path) -> Result<()> {
    let path_str = path.to_string_lossy().to_string();

    if !path.exists() {
        let idx = indexer.lock().await;
        idx.db.remove_chunks_for_file(&path_str).await?;
        println!("Removed from index: {path_str}");
        return Ok(());
    }

    if !path.is_file() {
        return Ok(());
    }

    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let skip_exts = [
            "exe", "dll", "so", "dylib", "bin", "obj", "o", "a", "lib",
            "png", "jpg", "jpeg", "gif", "bmp", "ico", "webp",
            "mp3", "mp4", "avi", "mov", "mkv", "wav", "flac", "ogg",
            "zip", "tar", "gz", "bz2", "7z", "rar",
        ];
        if skip_exts.contains(&ext) {
            return Ok(());
        }
    }

    {
        let idx = indexer.lock().await;
        if let Ok(Some(existing_hash)) = idx.db.get_last_hash(&path_str).await {
            if let Ok(current_hash) = crate::hash::hash_file(path).await {
                if current_hash == existing_hash {
                    return Ok(());
                }
            }
        }
    }

    let idx = indexer.lock().await;
    match idx.index_file(&path_str).await {
        Ok(chunks) => println!("Indexed {path_str}: {chunks} chunks"),
        Err(e) => eprintln!("Failed to index {path_str}: {e}"),
    }

    Ok(())
}
