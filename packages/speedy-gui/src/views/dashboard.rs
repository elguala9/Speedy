use crate::autostart;
use crate::daemon::{DaemonBridge, DaemonState};
use egui::{Color32, RichText, Ui};

pub fn render(
    ui: &mut Ui,
    bridge: &DaemonBridge,
    state: &DaemonState,
    notify_on_error: &mut bool,
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
                #[cfg(windows)]
                {
                    let _ = std::process::Command::new("explorer").arg(&s).spawn();
                }
                #[cfg(target_os = "macos")]
                {
                    let _ = std::process::Command::new("open").arg(&s).spawn();
                }
                #[cfg(all(unix, not(target_os = "macos")))]
                {
                    let _ = std::process::Command::new("xdg-open").arg(&s).spawn();
                }
            }
        }
    });

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
        let mut enabled = autostart::is_enabled().unwrap_or(false);
        let prev = enabled;
        ui.checkbox(&mut enabled, "Avvia daemon al login utente");
        if enabled != prev {
            let result = if enabled {
                autostart::enable()
            } else {
                autostart::disable()
            };
            if let Ok(mut s) = bridge.state.lock() {
                match result {
                    Ok(()) => s.set_toast(
                        if enabled { "Autostart attivo" } else { "Autostart disattivato" },
                        true,
                    ),
                    Err(e) => s.set_toast(format!("Autostart fallito: {e}"), false),
                }
            }
        }
    });
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
