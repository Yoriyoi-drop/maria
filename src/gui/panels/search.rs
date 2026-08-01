//! Sidebar Search — cari module (klik → buka file) dan signal dari hasil compile.
//!
//! Module berasal dari `CompileInfo.module_files` (module_index → path file);
//! signal dari `IrDesign.top.signals`. Filter substring case-insensitive.

use eframe::egui;

use super::super::state::GuiState;

pub fn show(ui: &mut egui::Ui, state: &mut GuiState) {
    if state.compile_info.is_none() && state.design.is_none() {
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Compile dulu untuk mencari").weak().italics());
        return;
    }

    ui.add(
        egui::TextEdit::singleline(&mut state.search_filter)
            .hint_text("Cari module / signal…")
            .desired_width(f32::INFINITY),
    );
    ui.separator();

    let q = state.search_filter.to_lowercase();
    // Borrow immutably (tanpa clone IrDesign per frame)
    let info = state.compile_info.as_ref();
    let design = state.design.as_ref();
    let mut to_open: Option<std::path::PathBuf> = None;
    let mut found_any = false;

    egui::ScrollArea::vertical()
        .id_salt("search_scroll")
        .show(ui, |ui| {
            // ── Modules ──
            if let Some(info) = info {
                let modules: Vec<&String> = info
                    .modules
                    .iter()
                    .filter(|m| q.is_empty() || m.to_lowercase().contains(&q))
                    .collect();
                if !modules.is_empty() {
                    found_any = true;
                    ui.label(egui::RichText::new("Modules").strong().size(11.0));
                    for m in modules {
                        let resp = ui.selectable_label(
                            false,
                            egui::RichText::new(format!("▸ {}", m)).monospace().size(12.0),
                        );
                        if resp.clicked() {
                            if let Some(file) = info.module_files.get(m) {
                                to_open = Some(file.clone());
                            }
                        }
                    }
                    ui.add_space(6.0);
                }
            }

            // ── Signals ──
            if let Some(design) = design {
                let matched: Vec<&crate::ir::SignalInfo> = design
                    .top
                    .signals
                    .iter()
                    .filter(|s| {
                        q.is_empty() || s.name.as_str().to_lowercase().contains(&q)
                    })
                    .collect();
                if !matched.is_empty() {
                    found_any = true;
                    ui.label(egui::RichText::new("Signals").strong().size(11.0));
                    for sig in matched {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("·").weak().size(10.0));
                            ui.label(
                                egui::RichText::new(sig.name.as_str())
                                    .monospace()
                                    .size(12.0),
                            );
                            ui.label(
                                egui::RichText::new(format!("[{}]", sig.width))
                                    .weak()
                                    .size(10.0),
                            );
                        });
                    }
                }
            }

            if !q.is_empty() && !found_any {
                ui.label(egui::RichText::new("Tidak ada hasil").weak().italics());
            }
        });

    if let Some(path) = to_open {
        state.open_file(path);
    }
}
