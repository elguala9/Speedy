use crate::daemon::{DaemonBridge, DaemonState};
use egui::{Color32, RichText, Ui};
use std::collections::HashMap;

#[derive(Clone)]
struct WorkspaceFeatures {
    speedy_indexer: bool,
    language_context: bool,
    text_context: bool,
}

impl Default for WorkspaceFeatures {
    fn default() -> Self {
        // Opt-in by default — matches speedy-language-context::features::Features.
        Self { speedy_indexer: false, language_context: false, text_context: false }
    }
}

fn load_features(workspace_path: &str) -> WorkspaceFeatures {
    // Single source of truth for the on-disk `[features]` schema.
    let f = speedy_core::contexts::load_features(Some(workspace_path));
    WorkspaceFeatures {
        speedy_indexer: f.speedy_indexer,
        language_context: f.language_context,
        text_context: f.text_context,
    }
}

fn save_features(workspace_path: &str, f: &WorkspaceFeatures) {
    // Each toggle is persisted via the shared helper, which merges into the
    // existing `.speedy/config.toml` and preserves other sections.
    let _ = speedy_core::contexts::set_feature(
        Some(workspace_path),
        "speedy_indexer",
        f.speedy_indexer,
    );
    let _ = speedy_core::contexts::set_feature(
        Some(workspace_path),
        "language_context",
        f.language_context,
    );
    let _ = speedy_core::contexts::set_feature(Some(workspace_path), "text_context", f.text_context);
}

#[derive(Default)]
pub struct WorkspacesView {
    pub pending_remove: Option<String>,
    features_cache: HashMap<String, WorkspaceFeatures>,
}

impl WorkspacesView {
    pub fn render(&mut self, ui: &mut Ui, bridge: &DaemonBridge, state: &DaemonState) {
        ui.heading("Workspaces");
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            if ui.button("➕ Add workspace…").clicked() {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    bridge.add_workspace(folder.to_string_lossy().to_string());
                }
            }
            if ui.button("Refresh").clicked() {
                bridge.refresh_all();
                for p in &state.workspaces {
                    bridge.refresh_workspace_status(p.clone());
                }
            }
            if ui
                .button("🧹 Prune orphans")
                .on_hover_text(
                    "Removes entries whose path no longer exists on disk",
                )
                .clicked()
            {
                bridge.prune_missing();
            }
        });

        ui.add_space(6.0);

        // No daemon is the normal standalone case (`standalone == !alive`):
        // `refresh_all` still loads the workspace list via `speedy-cli`, and
        // add/remove/sync/reindex all route through the CLI. So we keep
        // rendering the list instead of bailing out — only show a hint.
        if !state.alive && state.probed {
            ui.label(
                RichText::new(
                    "Standalone mode: no daemon running. Workspace operations run via speedy-cli.",
                )
                .weak(),
            );
            ui.add_space(4.0);
        }

        if state.workspaces.is_empty() {
            ui.label("No registered workspaces.");
            return;
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for path in state.workspaces.clone() {
                    self.row(ui, bridge, state, &path);
                    ui.separator();
                }
            });

        if let Some(target) = self.pending_remove.clone() {
            self.confirm_remove(ui.ctx(), bridge, target);
        }
    }

    fn row(&mut self, ui: &mut Ui, bridge: &DaemonBridge, state: &DaemonState, path: &str) {
        ui.horizontal(|ui| {
            let ws_status = state.workspace_status.get(path);
            let dot_color = if state.standalone {
                // No daemon → no watcher to report on; show a neutral dot
                // rather than implying the workspace is broken.
                Color32::from_rgb(150, 150, 150)
            } else {
                let alive = ws_status.map(|w| w.watcher_alive).unwrap_or(state.alive);
                if alive {
                    Color32::from_rgb(80, 200, 80)
                } else {
                    Color32::from_rgb(220, 80, 80)
                }
            };
            ui.colored_label(dot_color, "●");
            ui.monospace(path);
            if is_under_system_temp(path) {
                ui.colored_label(Color32::from_rgb(220, 180, 80), "⚠ temp")
                    .on_hover_text(
                        "Path under the system temp directory — probably a test \
                         leftover. Use it only if you really have a project in TEMP.",
                    );
            }
            if !std::path::Path::new(path).exists() {
                ui.colored_label(Color32::from_rgb(220, 100, 100), "⚠ missing")
                    .on_hover_text(
                        "The folder does not exist on disk. Press \"Prune orphans\" to remove it.",
                    );
            }
        });

        ui.horizontal_wrapped(|ui| {
            if let Some(ws) = state.workspace_status.get(path) {
                ui.label(RichText::new(format!("DB: {}", fmt_size(ws.index_size_bytes))).weak());
                if let Some(t) = ws.last_event_at {
                    ui.label(RichText::new(format!("event: {}", fmt_ago(t))).weak());
                }
                if let Some(t) = ws.last_sync_at {
                    ui.label(RichText::new(format!("sync: {}", fmt_ago(t))).weak());
                }
            } else if ui.button("Load status").clicked() {
                bridge.refresh_workspace_status(path.to_string());
            }
        });

        let is_indexing = state.indexing.contains(path);
        let is_syncing = state.syncing.contains(path);

        if is_indexing {
            ui.horizontal(|ui| {
                if let Some(&(processed, total)) = state.index_progress.get(path) {
                    if total > 0 {
                        let fraction = processed as f32 / total as f32;
                        ui.add(
                            egui::ProgressBar::new(fraction)
                                .text(format!("{processed} / {total} file"))
                                .desired_width(220.0),
                        );
                    } else {
                        ui.spinner();
                        ui.label(
                            RichText::new("Indexing…").color(Color32::from_rgb(180, 180, 80)),
                        );
                    }
                } else {
                    ui.spinner();
                    ui.label(
                        RichText::new("Indexing…").color(Color32::from_rgb(180, 180, 80)),
                    );
                }
            });
        }

        if is_syncing {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Syncing…").color(Color32::from_rgb(180, 180, 80)));
            });
        }

        ui.horizontal(|ui| {
            if ui
                .add_enabled(!is_indexing, egui::Button::new("Index"))
                .on_hover_text(
                    "Re-index this workspace now (runs only the enabled \
                     contexts; enable at least one feature below first).",
                )
                .clicked()
            {
                bridge.reindex_workspace(path.to_string());
            }
            if ui
                .add_enabled(!is_syncing, egui::Button::new("Sync"))
                .on_hover_text("Sync this workspace now")
                .clicked()
            {
                bridge.sync_workspace(path.to_string());
            }
            if ui.button("Open folder").clicked() {
                open_folder(path);
            }
            if ui
                .button(RichText::new("Remove").color(Color32::from_rgb(220, 120, 120)))
                .clicked()
            {
                self.pending_remove = Some(path.to_string());
            }
        });

        if !self.features_cache.contains_key(path) {
            self.features_cache.insert(path.to_string(), load_features(path));
        }
        let features = self.features_cache.get_mut(path).unwrap();
        let mut features_changed = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Features:").weak());
            if ui
                .checkbox(&mut features.speedy_indexer, "AI Context")
                .on_hover_text("AI semantic / vector index (speedy-ai-context)")
                .changed()
            {
                features_changed = true;
            }
            if ui
                .checkbox(&mut features.language_context, "Language Context")
                .on_hover_text("Code intelligence (speedy-language-context)")
                .changed()
            {
                features_changed = true;
            }
            if ui
                .checkbox(&mut features.text_context, "Text Context")
                .on_hover_text("Text-symbol index (speedy-text-context)")
                .changed()
            {
                features_changed = true;
            }
        });
        if features_changed {
            let snapshot = features.clone();
            save_features(path, &snapshot);
            if let Ok(mut s) = bridge.state.lock() {
                s.set_toast("Features updated", true);
            }
        }
    }

    fn confirm_remove(&mut self, ctx: &egui::Context, bridge: &DaemonBridge, target: String) {
        let mut close = false;
        let mut confirm = false;
        egui::Window::new("Confirm removal")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!("Remove the workspace?\n\n{target}"));
                ui.label(
                    RichText::new(
                        "The .speedy/ database on disk stays intact. You can delete it manually.",
                    )
                    .weak(),
                );
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    if ui
                        .button(RichText::new("Remove").color(Color32::from_rgb(220, 100, 100)))
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });
        if confirm {
            bridge.remove_workspace(target);
            self.pending_remove = None;
        } else if close {
            self.pending_remove = None;
        }
    }
}

fn fmt_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn fmt_ago(unix_secs: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let diff = now.saturating_sub(unix_secs);
    if diff < 60 {
        format!("{diff}s ago")
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

/// True when `path` sits inside the OS temp directory. Used to flag workspace
/// rows that almost certainly come from test runs (the actual user's projects
/// are not in `%TEMP%` / `/tmp`).
fn is_under_system_temp(path: &str) -> bool {
    let Ok(temp) = std::env::temp_dir().canonicalize() else {
        return false;
    };
    let p = std::path::Path::new(path);
    let candidate = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    candidate.starts_with(&temp)
}

fn open_folder(path: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(path).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}
