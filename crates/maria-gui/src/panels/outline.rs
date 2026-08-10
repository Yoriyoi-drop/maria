//! Outline panel kanan — navigasi simbol design ter-compile.
//!
//! Menampilkan instances (nama : module) dan signals (nama, width, kind) dari
//! modul top dengan kolom filter. Data dibaca langsung dari `IrDesign` — tanpa
//! clone per frame (borrow field-level dari `state`).

use eframe::egui;

use super::super::state::GuiState;

pub fn show(ui: &mut egui::Ui, state: &mut GuiState) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Outline").strong().size(12.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button(egui::RichText::new("✕").size(10.0))
                .on_hover_text("Tutup outline")
                .clicked()
            {
                state.show_outline = false;
            }
        });
    });

    let Some(design) = state.design.as_ref() else {
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Compile dulu untuk melihat outline").weak().italics());
        return;
    };

    ui.add(
        egui::TextEdit::singleline(&mut state.outline_filter)
            .hint_text("Filter…")
            .desired_width(f32::INFINITY),
    );
    ui.separator();

    let q = state.outline_filter.to_lowercase();
    egui::ScrollArea::vertical()
        .id_salt("outline_scroll")
        .show(ui, |ui| {
            // ── Instances ──
            if !design.top.sub_instances.is_empty() {
                ui.label(egui::RichText::new("Instances").strong().size(11.0));
                for inst in &design.top.sub_instances {
                    let label = format!("{} : {}", inst.instance_name, inst.module_name);
                    if q.is_empty() || label.to_lowercase().contains(&q) {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("▸").weak().size(10.0));
                            ui.label(egui::RichText::new(&label).monospace().size(12.0));
                        });
                    }
                }
                ui.add_space(6.0);
            }

            // ── Signals ──
            if !design.top.signals.is_empty() {
                ui.label(egui::RichText::new("Signals").strong().size(11.0));
            }
            for sig in &design.top.signals {
                let name = sig.name.to_string();
                if q.is_empty() || name.to_lowercase().contains(&q) {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("·").weak().size(10.0));
                        ui.label(egui::RichText::new(&name).monospace().size(12.0));
                        ui.label(
                            egui::RichText::new(format!("[{}]", sig.width))
                                .weak()
                                .size(10.0),
                        );
                        ui.label(
                            egui::RichText::new(format!("{:?}", sig.kind))
                                .weak()
                                .size(10.0),
                        );
                    });
                }
            }
        });
}
