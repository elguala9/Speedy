use crate::daemon::{DaemonBridge, DaemonState};
use egui::{Color32, RichText, Ui};

#[derive(Default)]
pub struct WorkspacesView {
    pub pending_remove: Option<String>,
}

impl WorkspacesView {
    pub fn render(&mut self, ui: &mut Ui, bridge: &DaemonBridge, state: &DaemonState) {
        ui.heading("Workspaces");
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            if ui.button("➕ Aggiungi workspace…").clicked() {
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
                .button("🧹 Pulisci orfani")
                .on_hover_text(
                    "Rimuove le entry il cui path non esiste più sul disco",
                )
                .clicked()
            {
                bridge.prune_missing();
            }
        });

        ui.add_space(6.0);

        if !state.alive {
            ui.colored_label(Color32::from_rgb(220, 80, 80), "Daemon non raggiungibile.");
            return;
        }

        if state.workspaces.is_empty() {
            ui.label("Nessun workspace registrato.");
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
            let alive = ws_status.map(|w| w.watcher_alive).unwrap_or(state.alive);
            let dot_color = if alive {
                Color32::from_rgb(80, 200, 80)
            } else {
                Color32::from_rgb(220, 80, 80)
            };
            ui.colored_label(dot_color, "●");
            ui.monospace(path);
            if is_under_system_temp(path) {
                ui.colored_label(Color32::from_rgb(220, 180, 80), "⚠ temp")
                    .on_hover_text(
                        "Path sotto la directory temporanea di sistema — probabilmente \
                         un residuo di test. Usalo solo se hai davvero un progetto in TEMP.",
                    );
            }
            if !std::path::Path::new(path).exists() {
                ui.colored_label(Color32::from_rgb(220, 100, 100), "⚠ mancante")
                    .on_hover_text(
                        "La cartella non esiste sul disco. Premi \"Pulisci orfani\" per rimuoverla.",
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
            } else if ui.button("Carica stato").clicked() {
                bridge.refresh_workspace_status(path.to_string());
            }
        });

        ui.horizontal(|ui| {
            if ui.button("Index").clicked() {
                bridge.reindex_workspace(path.to_string());
            }
            if ui.button("Sync").clicked() {
                bridge.sync_workspace(path.to_string());
            }
            if ui.button("Open folder").clicked() {
                open_folder(path);
            }
            if ui
                .button(RichText::new("Rimuovi").color(Color32::from_rgb(220, 120, 120)))
                .clicked()
            {
                self.pending_remove = Some(path.to_string());
            }
        });
    }

    fn confirm_remove(&mut self, ctx: &egui::Context, bridge: &DaemonBridge, target: String) {
        let mut close = false;
        let mut confirm = false;
        egui::Window::new("Conferma rimozione")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!("Rimuovere il workspace?\n\n{target}"));
                ui.label(
                    RichText::new(
                        "Il database .speedy/ sul disco resta intatto. Puoi cancellarlo a mano.",
                    )
                    .weak(),
                );
                ui.horizontal(|ui| {
                    if ui.button("Annulla").clicked() {
                        close = true;
                    }
                    if ui
                        .button(RichText::new("Rimuovi").color(Color32::from_rgb(220, 100, 100)))
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
