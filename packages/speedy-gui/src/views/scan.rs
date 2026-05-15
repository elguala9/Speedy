use crate::daemon::{DaemonBridge, DaemonState};
use egui::{Color32, RichText, Ui};
use std::collections::HashSet;

pub struct ScanView {
    pub root: String,
    pub max_depth: usize,
    pub selected: HashSet<String>,
}

impl Default for ScanView {
    fn default() -> Self {
        let home = dirs::home_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string());
        Self {
            root: home,
            max_depth: 8,
            selected: HashSet::new(),
        }
    }
}

impl ScanView {
    pub fn render(&mut self, ui: &mut Ui, bridge: &DaemonBridge, state: &DaemonState) {
        ui.heading("Scan workspace orfani");
        ui.add_space(6.0);
        ui.label("Cerca cartelle che contengono un .speedy/ ma non sono registrate nel daemon.");
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.label("Root:");
            ui.add(egui::TextEdit::singleline(&mut self.root).desired_width(420.0));
            if ui.button("…").clicked() {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    self.root = folder.to_string_lossy().to_string();
                }
            }
            ui.label("Max depth:");
            ui.add(egui::DragValue::new(&mut self.max_depth).range(1..=20));
            if ui.button("Scansiona").clicked() {
                self.selected.clear();
                bridge.scan(self.root.clone(), self.max_depth);
            }
        });

        ui.add_space(8.0);

        if state.scan_results.is_empty() {
            ui.label(RichText::new("Nessun risultato (ancora). Lancia una scansione.").weak());
            return;
        }

        ui.horizontal(|ui| {
            let unregistered: Vec<&str> = state
                .scan_results
                .iter()
                .filter(|r| !r.registered)
                .map(|r| r.path.as_str())
                .collect();
            if ui
                .button(format!(
                    "Registra selezionati ({})",
                    self.selected.iter().filter(|p| unregistered.contains(&p.as_str())).count()
                ))
                .clicked()
            {
                for p in &self.selected {
                    if unregistered.contains(&p.as_str()) {
                        bridge.add_workspace(p.clone());
                    }
                }
                self.selected.clear();
            }
            if ui.button("Seleziona tutti i non registrati").clicked() {
                self.selected = unregistered.iter().map(|s| s.to_string()).collect();
            }
            if ui.button("Pulisci selezione").clicked() {
                self.selected.clear();
            }
        });

        ui.add_space(6.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("scan_grid")
                    .num_columns(5)
                    .striped(true)
                    .spacing([12.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("Sel").strong());
                        ui.label(RichText::new("Path").strong());
                        ui.label(RichText::new("Reg.").strong());
                        ui.label(RichText::new("DB size").strong());
                        ui.label(RichText::new("Last mod").strong());
                        ui.end_row();

                        let results = state.scan_results.clone();
                        for r in results {
                            let mut sel = self.selected.contains(&r.path);
                            let was = sel;
                            ui.add_enabled(!r.registered, egui::Checkbox::new(&mut sel, ""));
                            if sel != was {
                                if sel {
                                    self.selected.insert(r.path.clone());
                                } else {
                                    self.selected.remove(&r.path);
                                }
                            }
                            ui.monospace(&r.path);
                            if r.registered {
                                ui.colored_label(Color32::from_rgb(80, 200, 80), "sì");
                            } else {
                                ui.colored_label(Color32::from_rgb(220, 180, 80), "no");
                            }
                            ui.monospace(fmt_size(r.db_size_bytes));
                            ui.monospace(r.last_modified.as_deref().unwrap_or("—"));
                            ui.end_row();
                        }
                    });
            });
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
