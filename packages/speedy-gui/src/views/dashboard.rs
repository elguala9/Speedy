use crate::daemon::{DaemonBridge, DaemonState};
use egui::{Color32, RichText, Ui};
use std::time::Duration;
use std::path::PathBuf;

#[derive(Default)]
pub struct DashboardView {
    /// Live buffer behind the daemon-exe text field. Kept here (not in the
    /// bridge) so typing doesn't roundtrip through the override mutex on
    /// every frame. We sync it with the bridge override on init and on the
    /// "Applica" / "Sfoglia" / "Ripristina" actions.
    daemon_exe_buf: String,
    daemon_exe_initialized: bool,
}

impl DashboardView {
    pub fn render(
        &mut self,
        ui: &mut Ui,
        bridge: &DaemonBridge,
        state: &DaemonState,
        notify_on_error: &mut bool,
        refresh_interval: &mut Duration,
    ) {
        ui.heading("Dashboard");
        ui.add_space(6.0);

        if !state.alive {
            ui.colored_label(
                Color32::from_rgb(220, 80, 80),
                "Daemon non in esecuzione.",
            );
            if ui.button("Avvia daemon").clicked() {
                if let Err(e) = bridge.spawn_daemon() {
                    if let Ok(mut s) = bridge.state.lock() {
                        s.set_toast(format!("Spawn failed: {e}"), false);
                    }
                }
            }
            ui.add_space(8.0);
        }

        egui::Grid::new("status_grid")
            .num_columns(2)
            .spacing([20.0, 4.0])
            .show(ui, |ui| {
                ui.label("Status:");
                if state.alive {
                    ui.colored_label(Color32::from_rgb(80, 200, 80), "● alive");
                } else {
                    ui.colored_label(Color32::from_rgb(220, 80, 80), "● down");
                }
                ui.end_row();

                if let Some(s) = &state.status {
                    ui.label("PID:");
                    ui.monospace(s.pid.to_string());
                    ui.end_row();

                    ui.label("Uptime:");
                    ui.monospace(fmt_uptime(s.uptime_secs));
                    ui.end_row();

                    ui.label("Version:");
                    ui.monospace(&s.version);
                    ui.end_row();

                    ui.label("Protocol:");
                    ui.monospace(s.protocol_version.to_string());
                    ui.end_row();

                    ui.label("Workspaces:");
                    ui.monospace(s.workspace_count.to_string());
                    ui.end_row();

                    ui.label("Watchers:");
                    ui.monospace(s.watcher_count.to_string());
                    ui.end_row();
                }
            });

        if let Some(m) = &state.metrics {
            ui.add_space(8.0);
            ui.label(RichText::new("Metrics").strong());
            egui::Grid::new("metrics_grid")
                .num_columns(2)
                .spacing([20.0, 2.0])
                .show(ui, |ui| {
                    metric_row(ui, "Queries", m.queries);
                    metric_row(ui, "Indexes", m.indexes);
                    metric_row(ui, "Syncs", m.syncs);
                    metric_row(ui, "Watcher events", m.watcher_events);
                    metric_row(ui, "Exec calls", m.exec_calls);
                });
        }

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if let Ok(dir) = speedy_core::daemon_util::daemon_dir_path() {
                ui.label("Config dir:");
                let s = dir.to_string_lossy().to_string();
                if ui.link(&s).clicked() {
                    open_in_filemanager(&s);
                }
            }
        });

        self.render_daemon_exe(ui, bridge);

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button("Refresh now").clicked() {
                bridge.refresh_all();
            }
            if state.alive && ui.button("Reload workspaces").clicked() {
                bridge.reload();
            }
            if state.alive && ui.button("Restart daemon").clicked() {
                bridge.restart_daemon();
            }
            if state.alive
                && ui
                    .button(RichText::new("Stop daemon").color(Color32::from_rgb(220, 100, 100)))
                    .clicked()
            {
                bridge.stop_daemon();
            }
        });

        ui.add_space(12.0);
        ui.label(RichText::new("Preferenze").strong());
        ui.horizontal(|ui| {
            ui.checkbox(notify_on_error, "Notifiche di sistema su errore");
        });
        ui.horizontal(|ui| {
            ui.label("Auto-refresh ogni:");
            let mut secs = refresh_interval.as_secs().max(1) as u32;
            if ui.add(egui::DragValue::new(&mut secs).range(1..=60).suffix(" s")).changed() {
                *refresh_interval = Duration::from_secs(secs as u64);
            }
        });
    }

    fn render_daemon_exe(&mut self, ui: &mut Ui, bridge: &DaemonBridge) {
        ui.add_space(10.0);
        ui.label(RichText::new("Eseguibile daemon").strong());

        let override_set = bridge.daemon_exe_override().is_some();
        let resolved = bridge.resolved_daemon_exe();
        let resolved_str = resolved
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();

        // First-render sync: text field initially reflects the persisted
        // override (or the auto-detected path if no override).
        if !self.daemon_exe_initialized {
            self.daemon_exe_buf = resolved_str.clone();
            self.daemon_exe_initialized = true;
        }

        ui.horizontal(|ui| {
            ui.label(if override_set { "Personalizzato:" } else { "Auto:" });
            ui.add(
                egui::TextEdit::singleline(&mut self.daemon_exe_buf)
                    .desired_width(f32::INFINITY)
                    .hint_text("Percorso a speedy-daemon"),
            );
        });

        ui.horizontal(|ui| {
            if ui.button("Sfoglia…").clicked() {
                let mut dialog = rfd::FileDialog::new().set_title("Seleziona speedy-daemon");
                if cfg!(windows) {
                    dialog = dialog.add_filter("Eseguibile", &["exe"]);
                }
                if let Some(p) = dialog.pick_file() {
                    self.daemon_exe_buf = p.to_string_lossy().into_owned();
                    bridge.set_daemon_exe_override(Some(p));
                    if let Ok(mut s) = bridge.state.lock() {
                        s.set_toast("Path daemon aggiornato", true);
                    }
                }
            }

            let buf_trimmed = self.daemon_exe_buf.trim();
            let dirty = buf_trimmed
                != bridge
                    .daemon_exe_override()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| resolved_str.clone());
            let apply_enabled = !buf_trimmed.is_empty() && dirty;
            if ui
                .add_enabled(apply_enabled, egui::Button::new("Applica"))
                .clicked()
            {
                let p = PathBuf::from(buf_trimmed);
                let exists = p.exists();
                bridge.set_daemon_exe_override(Some(p));
                if let Ok(mut s) = bridge.state.lock() {
                    if exists {
                        s.set_toast("Path daemon aggiornato", true);
                    } else {
                        s.set_toast("Path salvato ma il file non esiste", false);
                    }
                }
            }

            if ui
                .add_enabled(
                    override_set,
                    egui::Button::new("Ripristina automatico"),
                )
                .clicked()
            {
                bridge.set_daemon_exe_override(None);
                self.daemon_exe_buf = speedy_core::daemon_util::resolve_daemon_exe()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if let Ok(mut s) = bridge.state.lock() {
                    s.set_toast("Rilevamento automatico ripristinato", true);
                }
            }

            if !resolved_str.is_empty() && ui.button("Apri cartella").clicked() {
                if let Some(parent) = resolved.as_ref().ok().and_then(|p| p.parent()) {
                    open_in_filemanager(&parent.to_string_lossy());
                }
            }
        });

        match &resolved {
            Ok(p) if !p.exists() => {
                ui.colored_label(
                    Color32::from_rgb(220, 100, 100),
                    format!("⚠ il file non esiste: {}", p.display()),
                );
            }
            Err(e) => {
                ui.colored_label(
                    Color32::from_rgb(220, 100, 100),
                    format!("⚠ daemon non trovato: {e}"),
                );
            }
            _ => {}
        }
    }
}

fn open_in_filemanager(path: &str) {
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

fn metric_row(ui: &mut Ui, label: &str, value: u64) {
    ui.label(label);
    ui.monospace(value.to_string());
    ui.end_row();
}

fn fmt_uptime(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m:02}m {s:02}s")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}
