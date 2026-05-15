use crate::daemon::DaemonBridge;
use crate::tray::{TrayAction, TrayHandle};
use crate::views::{self, Tab};
use eframe::{App, CreationContext, Frame};
use egui::{Color32, Context, RichText};
use speedy_core::types::LogLine;
use std::sync::Arc;
use std::time::{Duration, Instant};

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const SETTINGS_KEY: &str = "speedy-gui.settings";

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedSettings {
    current_tab: Tab,
    dark_mode: bool,
    socket_name: String,
    notify_on_error: bool,
}

impl Default for PersistedSettings {
    fn default() -> Self {
        Self {
            current_tab: Tab::default(),
            dark_mode: true,
            socket_name: speedy_core::daemon_util::default_daemon_socket_name(),
            notify_on_error: false,
        }
    }
}

pub struct SpeedyApp {
    bridge: DaemonBridge,
    current_tab: Tab,
    dark_mode: bool,
    pub notify_on_error: bool,
    workspaces_view: views::workspaces::WorkspacesView,
    scan_view: views::scan::ScanView,
    logs_view: views::logs::LogsView,
    last_auto_refresh: Instant,
    tray: Option<Arc<TrayHandle>>,
    last_notified_log_count: usize,
}

impl SpeedyApp {
    pub fn new(cc: &CreationContext<'_>, tray: Option<Arc<TrayHandle>>) -> Self {
        let settings: PersistedSettings = cc
            .storage
            .and_then(|s| eframe::get_value(s, SETTINGS_KEY))
            .unwrap_or_default();

        if settings.dark_mode {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
        } else {
            cc.egui_ctx.set_visuals(egui::Visuals::light());
        }

        let bridge =
            DaemonBridge::new(settings.socket_name).expect("failed to create daemon bridge");
        bridge.refresh_all();
        Self {
            bridge,
            current_tab: settings.current_tab,
            dark_mode: settings.dark_mode,
            notify_on_error: settings.notify_on_error,
            workspaces_view: Default::default(),
            scan_view: Default::default(),
            logs_view: Default::default(),
            last_auto_refresh: Instant::now(),
            tray,
            last_notified_log_count: 0,
        }
    }

    fn handle_tray_actions(&mut self, ctx: &Context) {
        let Some(tray) = self.tray.as_ref() else { return; };
        for action in tray.poll_actions() {
            match action {
                TrayAction::Show => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                TrayAction::Restart => {
                    self.bridge.restart_daemon();
                }
                TrayAction::Quit => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    fn notify_new_errors(&mut self) {
        if !self.notify_on_error {
            return;
        }
        let Ok(buf) = self.logs_view.stream.buffer.lock() else { return; };
        let count = buf.lines.len();
        if count <= self.last_notified_log_count {
            self.last_notified_log_count = count;
            return;
        }
        // Walk only the newly-appended slice — VecDeque ring buffer may have
        // dropped older entries, so clamp to the available range.
        let already = self.last_notified_log_count.min(count);
        let new_lines: Vec<LogLine> = buf.lines.iter().skip(already).cloned().collect();
        self.last_notified_log_count = count;
        drop(buf);
        for line in new_lines {
            if line.level.eq_ignore_ascii_case("error") {
                let _ = notify_rust::Notification::new()
                    .summary("Speedy: errore daemon")
                    .body(&line.message)
                    .show();
            }
        }
    }
}

impl App for SpeedyApp {
    fn update(&mut self, ctx: &Context, _frame: &mut Frame) {
        if self.last_auto_refresh.elapsed() >= REFRESH_INTERVAL {
            self.bridge.refresh_all();
            self.last_auto_refresh = Instant::now();
        }
        ctx.request_repaint_after(Duration::from_millis(500));

        self.handle_tray_actions(ctx);

        let state_snapshot = match self.bridge.state.lock() {
            Ok(s) => s.clone(),
            Err(_) => return,
        };

        if let Some(tray) = self.tray.as_ref() {
            tray.set_alive(state_snapshot.alive);
        }

        self.notify_new_errors();

        egui::TopBottomPanel::top("topbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Speedy");
                ui.add_space(12.0);
                for tab in [Tab::Dashboard, Tab::Workspaces, Tab::Scan, Tab::Logs] {
                    let selected = self.current_tab == tab;
                    if ui.selectable_label(selected, tab.label()).clicked() {
                        self.current_tab = tab;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let theme_icon = if self.dark_mode { "☀" } else { "🌙" };
                    if ui.button(theme_icon).on_hover_text("Toggle theme").clicked() {
                        self.dark_mode = !self.dark_mode;
                        if self.dark_mode {
                            ctx.set_visuals(egui::Visuals::dark());
                        } else {
                            ctx.set_visuals(egui::Visuals::light());
                        }
                    }
                    if state_snapshot.alive {
                        ui.colored_label(Color32::from_rgb(80, 200, 80), "● daemon");
                    } else if state_snapshot.probed {
                        ui.colored_label(Color32::from_rgb(220, 80, 80), "● daemon down");
                    } else {
                        ui.colored_label(Color32::from_rgb(180, 180, 80), "● probing…");
                    }
                    if state_snapshot.busy > 0 {
                        ui.spinner();
                    }
                });
            });
        });

        egui::TopBottomPanel::bottom("statusbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some((msg, ts, ok)) = &state_snapshot.toast {
                    let age = ts.elapsed();
                    if age < Duration::from_secs(6) {
                        let color = if *ok {
                            Color32::from_rgb(120, 200, 120)
                        } else {
                            Color32::from_rgb(220, 120, 120)
                        };
                        ui.colored_label(color, RichText::new(msg));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.monospace(format!("socket: {}", self.bridge.socket_name));
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.current_tab {
            Tab::Dashboard => views::dashboard::render(
                ui,
                &self.bridge,
                &state_snapshot,
                &mut self.notify_on_error,
            ),
            Tab::Workspaces => {
                self.workspaces_view.render(ui, &self.bridge, &state_snapshot)
            }
            Tab::Scan => self.scan_view.render(ui, &self.bridge, &state_snapshot),
            Tab::Logs => self.logs_view.render(ui, &self.bridge),
        });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let s = PersistedSettings {
            current_tab: self.current_tab,
            dark_mode: self.dark_mode,
            socket_name: self.bridge.socket_name.clone(),
            notify_on_error: self.notify_on_error,
        };
        eframe::set_value(storage, SETTINGS_KEY, &s);
    }
}
