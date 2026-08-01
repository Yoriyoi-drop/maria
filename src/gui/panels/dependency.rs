//! Dependency tab — graf instansiasi antar module (CPU → AXI → Cache → …).
//!
//! Berbeda dari Architecture (pohon instance hierarkis): Dependency menampilkan
//! view module-level — setiap module dan module-module yang DI-INSTANSIASInya,
//! dengan jumlah instance. Klik module → buka file RTL via module_files.

use eframe::egui;

use super::super::state::GuiState;

pub fn show(ui: &mut egui::Ui, state: &mut GuiState) {
    let Some(info) = &state.compile_info else {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Compile dulu untuk melihat dependency")
                .weak()
                .italics(),
        );
        return;
    };
    if info.deps.is_empty() {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Tidak ada module yang menginstansiasi module lain")
                .weak()
                .italics(),
        );
        return;
    }

    let module_files = info.module_files.clone();
    let deps = info.deps.clone();
    let mut to_open: Option<std::path::PathBuf> = None;

    egui::ScrollArea::vertical()
        .id_salt("dep_scroll")
        .show(ui, |ui| {
            // Default: root module (baris pertama, sudah disortir ke atas di
            // backend) terbuka — mirip depth < 2 di Architecture viewer.
            for (i, row) in deps.iter().enumerate() {
                let has_children = !row.children.is_empty();
                let is_open = state
                    .dep_open
                    .get(&row.module)
                    .copied()
                    .unwrap_or(i == 0);

                ui.horizontal(|ui| {
                    if has_children {
                        let arrow = if is_open { "▾" } else { "▸" };
                        if ui
                            .button(egui::RichText::new(arrow).size(10.0))
                            .clicked()
                        {
                            state.dep_open.insert(row.module.clone(), !is_open);
                        }
                    } else {
                        ui.label(egui::RichText::new("·").weak().size(10.0));
                    }

                    let label = format!("{}  ({} dep)", row.module, row.children.len());
                    if ui
                        .selectable_label(
                            false,
                            egui::RichText::new(label).monospace().size(12.0),
                        )
                        .on_hover_text("Klik untuk membuka file module")
                        .clicked()
                    {
                        if let Some(file) = module_files.get(&row.module) {
                            to_open = Some(file.clone());
                        }
                    }
                });

                if is_open {
                    for (child, count) in &row.children {
                        ui.horizontal(|ui| {
                            ui.add_space(24.0);
                            ui.label(egui::RichText::new("└─").weak().size(11.0));
                            if ui
                                .selectable_label(
                                    false,
                                    egui::RichText::new(child).monospace().size(11.0),
                                )
                                .on_hover_text("Klik untuk membuka file module")
                                .clicked()
                            {
                                if let Some(file) = module_files.get(child) {
                                    to_open = Some(file.clone());
                                }
                            }
                            ui.label(
                                egui::RichText::new(format!("×{}", count))
                                    .weak()
                                    .size(10.0),
                            );
                        });
                    }
                }
            }
        });

    if let Some(path) = to_open {
        state.open_file(path);
    }
}
