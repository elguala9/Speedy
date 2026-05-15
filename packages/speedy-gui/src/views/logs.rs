use crate::daemon::DaemonBridge;
use crate::log_stream::LogStreamHandle;
use egui::{Color32, RichText, Ui};
use speedy_core::types::LogLine;
use std::path::PathBuf;

pub struct LogsView {
    pub stream: LogStreamHandle,
    pub level_filter: LevelFilter,
    pub substring: String,
    pub target_filter: String,
    pub follow_tail: bool,
    pub started: bool,
    pub workspace_filter: String,
    pub source: LogSource,
    pub available_files: Vec<PathBuf>,
    pub history_lines: Vec<LogLine>,
    pub history_error: Option<String>,
    pub history_loaded_path: Option<PathBuf>,
}

#[derive(Clone, PartialEq, Eq)]
pub enum LogSource {
    Live,
    File(PathBuf),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LevelFilter {
    pub trace: bool,
    pub debug: bool,
    pub info: bool,
    pub warn: bool,
    pub error: bool,
}

impl Default for LevelFilter {
    fn default() -> Self {
        Self { trace: false, debug: false, info: true, warn: true, error: true }
    }
}

impl LevelFilter {
    fn accepts(&self, level: &str) -> bool {
        match level {
            "trace" => self.trace,
            "debug" => self.debug,
            "info" => self.info,
            "warn" => self.warn,
            "error" => self.error,
            _ => true,
        }
    }
}

impl Default for LogsView {
    fn default() -> Self {
        Self {
            stream: LogStreamHandle::new(),
            level_filter: LevelFilter::default(),
            substring: String::new(),
            target_filter: String::new(),
            follow_tail: true,
            started: false,
            workspace_filter: String::new(),
            source: LogSource::Live,
            available_files: Vec::new(),
            history_lines: Vec::new(),
            history_error: None,
            history_loaded_path: None,
        }
    }
}

impl LogsView {
    pub fn render(&mut self, ui: &mut Ui, bridge: &DaemonBridge) {
        if !self.started {
            self.stream.start(bridge);
            self.started = true;
            self.available_files = list_log_files();
        }

        ui.heading("Log viewer");
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label("Sorgente:");
            let live_label = "Live (stream)";
            if ui
                .selectable_label(matches!(self.source, LogSource::Live), live_label)
                .clicked()
            {
                self.source = LogSource::Live;
            }
            ui.label("File:");
            let current_file = match &self.source {
                LogSource::File(p) => p
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "(seleziona)".into()),
                _ => "(seleziona)".into(),
            };
            egui::ComboBox::from_id_source("log_file_combo")
                .selected_text(current_file)
                .show_ui(ui, |ui| {
                    for p in &self.available_files {
                        let label = p
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let selected = matches!(&self.source, LogSource::File(cur) if cur == p);
                        if ui.selectable_label(selected, label).clicked() {
                            self.source = LogSource::File(p.clone());
                        }
                    }
                });
            if ui.button("↻ Rilegge elenco").clicked() {
                self.available_files = list_log_files();
            }
        });

        ui.horizontal_wrapped(|ui| {
            ui.label("Livello:");
            ui.checkbox(&mut self.level_filter.trace, "trace");
            ui.checkbox(&mut self.level_filter.debug, "debug");
            ui.checkbox(&mut self.level_filter.info, "info");
            ui.checkbox(&mut self.level_filter.warn, "warn");
            ui.checkbox(&mut self.level_filter.error, "error");
        });
        ui.horizontal(|ui| {
            ui.label("Cerca:");
            ui.add(egui::TextEdit::singleline(&mut self.substring).desired_width(220.0));
            ui.label("Target:");
            ui.add(
                egui::TextEdit::singleline(&mut self.target_filter)
                    .hint_text("ipc, watcher, sync...")
                    .desired_width(160.0),
            );
            ui.label("Workspace:");
            ui.add(
                egui::TextEdit::singleline(&mut self.workspace_filter)
                    .hint_text("substring")
                    .desired_width(220.0),
            );
        });

        ui.horizontal(|ui| {
            if matches!(self.source, LogSource::Live) {
                ui.checkbox(&mut self.follow_tail, "Follow tail");
            }
            if ui.button("Pulisci buffer").clicked() {
                if matches!(self.source, LogSource::Live) {
                    self.stream.clear();
                } else {
                    self.history_lines.clear();
                    self.history_loaded_path = None;
                }
            }
            if matches!(self.source, LogSource::Live) {
                if ui.button("Restart stream").clicked() {
                    self.stream.stop();
                    self.stream.start(bridge);
                }
            }

            match &self.source {
                LogSource::Live => {
                    let (connected, last_err, count) = match self.stream.buffer.lock() {
                        Ok(b) => (b.connected, b.last_error.clone(), b.lines.len()),
                        Err(_) => (false, None, 0),
                    };
                    if connected {
                        ui.colored_label(
                            Color32::from_rgb(80, 200, 80),
                            format!("● connected ({count})"),
                        );
                    } else {
                        ui.colored_label(Color32::from_rgb(220, 80, 80), "● disconnected");
                    }
                    if let Some(e) = last_err {
                        ui.label(
                            RichText::new(e).color(Color32::from_rgb(220, 120, 120)).weak(),
                        );
                    }
                }
                LogSource::File(_) => {
                    ui.colored_label(
                        Color32::from_rgb(180, 180, 220),
                        format!("● file ({})", self.history_lines.len()),
                    );
                    if let Some(e) = &self.history_error {
                        ui.label(
                            RichText::new(e).color(Color32::from_rgb(220, 120, 120)).weak(),
                        );
                    }
                }
            }
        });

        // Load history file lazily when the selection changes.
        if let LogSource::File(path) = self.source.clone() {
            if self.history_loaded_path.as_ref() != Some(&path) {
                match load_log_file(&path) {
                    Ok(lines) => {
                        self.history_lines = lines;
                        self.history_error = None;
                    }
                    Err(e) => {
                        self.history_lines.clear();
                        self.history_error = Some(format!("{e}"));
                    }
                }
                self.history_loaded_path = Some(path);
            }
        }

        let lines: Vec<LogLine> = match &self.source {
            LogSource::Live => match self.stream.buffer.lock() {
                Ok(b) => b.lines.iter().cloned().collect(),
                Err(_) => Vec::new(),
            },
            LogSource::File(_) => self.history_lines.clone(),
        };

        // Pre-compute the filtered set once so the export button reuses it.
        let filtered: Vec<&LogLine> = lines.iter().filter(|l| self.passes_filters(l)).collect();

        ui.horizontal(|ui| {
            if ui
                .button(format!("Esporta selezione ({})", filtered.len()))
                .clicked()
            {
                if let Some(p) = rfd::FileDialog::new()
                    .set_file_name("speedy-log-export.json")
                    .add_filter("JSON", &["json"])
                    .add_filter("JSON Lines", &["jsonl"])
                    .save_file()
                {
                    let snapshot: Vec<LogLine> = filtered.iter().map(|l| (*l).clone()).collect();
                    match write_export(&p, &snapshot) {
                        Ok(_) => {
                            if let Ok(mut s) = bridge.state.lock() {
                                s.set_toast(format!("Esportato: {}", p.display()), true);
                            }
                        }
                        Err(e) => {
                            if let Ok(mut s) = bridge.state.lock() {
                                s.set_toast(format!("Export fallito: {e}"), false);
                            }
                        }
                    }
                }
            }
        });

        ui.separator();

        let mut scroll = egui::ScrollArea::vertical().auto_shrink([false, false]);
        if matches!(self.source, LogSource::Live) && self.follow_tail {
            scroll = scroll.stick_to_bottom(true);
        }

        scroll.show(ui, |ui| {
            for line in filtered {
                let color = match line.level.as_str() {
                    "error" => Color32::from_rgb(230, 100, 100),
                    "warn" => Color32::from_rgb(230, 180, 80),
                    "info" => Color32::from_rgb(180, 220, 240),
                    "debug" => Color32::from_rgb(160, 160, 200),
                    "trace" => Color32::from_rgb(140, 140, 160),
                    _ => Color32::from_rgb(220, 220, 220),
                };

                let ts = line.ts.split('.').next().unwrap_or(&line.ts);
                let extras = if line.fields.is_empty() {
                    String::new()
                } else {
                    let mut parts = Vec::new();
                    for (k, v) in &line.fields {
                        let v_str = match v {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        parts.push(format!("{k}={v_str}"));
                    }
                    format!("  {{ {} }}", parts.join(", "))
                };

                ui.horizontal(|ui| {
                    ui.monospace(RichText::new(ts).weak());
                    ui.monospace(RichText::new(format!("[{:>5}]", line.level)).color(color));
                    ui.monospace(
                        RichText::new(&line.target).color(Color32::from_rgb(150, 200, 150)),
                    );
                    ui.monospace(&line.message);
                    if !extras.is_empty() {
                        ui.monospace(RichText::new(&extras).weak());
                    }
                });
            }
        });
    }

    fn passes_filters(&self, line: &LogLine) -> bool {
        if !self.level_filter.accepts(&line.level) {
            return false;
        }
        if !self.substring.is_empty()
            && !line.message.to_lowercase().contains(&self.substring.to_lowercase())
        {
            return false;
        }
        if !self.target_filter.is_empty()
            && !line.target.to_lowercase().contains(&self.target_filter.to_lowercase())
        {
            return false;
        }
        if !self.workspace_filter.is_empty() {
            let ws = line
                .fields
                .get("workspace")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !ws.to_lowercase().contains(&self.workspace_filter.to_lowercase()) {
                return false;
            }
        }
        true
    }
}

fn logs_dir() -> Option<PathBuf> {
    speedy_core::daemon_util::daemon_dir_path()
        .ok()
        .map(|p| p.join("logs"))
}

fn list_log_files() -> Vec<PathBuf> {
    let Some(dir) = logs_dir() else { return Vec::new(); };
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new(); };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("daemon.log"))
                .unwrap_or(false)
        })
        .collect();
    // Most recent first (lexicographic order matches date suffix order).
    files.sort();
    files.reverse();
    files
}

fn load_log_file(path: &std::path::Path) -> std::io::Result<Vec<LogLine>> {
    let contents = std::fs::read_to_string(path)?;
    let mut out = Vec::new();
    for line in contents.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(l) = parse_log_line(line) {
            out.push(l);
        }
    }
    Ok(out)
}

fn parse_log_line(raw: &str) -> serde_json::Result<LogLine> {
    // The daemon emits one JSON object per line; tracing's JSON formatter uses
    // a slightly different shape (timestamp, level, target, fields.message…)
    // than our IPC `LogLine`. Try the IPC shape first, then fall back to a
    // best-effort projection of the tracing shape.
    if let Ok(line) = serde_json::from_str::<LogLine>(raw) {
        return Ok(line);
    }
    let v: serde_json::Value = serde_json::from_str(raw)?;
    let ts = v
        .get("timestamp")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let level = v
        .get("level")
        .and_then(|x| x.as_str())
        .unwrap_or("info")
        .to_ascii_lowercase();
    let target = v
        .get("target")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let (message, fields) = match v.get("fields") {
        Some(serde_json::Value::Object(map)) => {
            let mut map = map.clone();
            let msg = map
                .remove("message")
                .and_then(|m| m.as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            (msg, map.into_iter().collect())
        }
        _ => (String::new(), serde_json::Map::new()),
    };
    Ok(LogLine { ts, level, target, message, fields })
}

fn write_export(path: &std::path::Path, lines: &[LogLine]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::File::create(path)?;
    let is_jsonl = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("jsonl"))
        .unwrap_or(false);
    if is_jsonl {
        for line in lines {
            serde_json::to_writer(&mut file, line)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            file.write_all(b"\n")?;
        }
    } else {
        serde_json::to_writer_pretty(&mut file, lines)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    }
    Ok(())
}
