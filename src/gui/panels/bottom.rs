//! Panel bawah: Problems (diagnostics), Console (log), Signals (hasil sim).

use eframe::egui;

use super::super::state::{BottomTab, DiagLevel, GuiState};

pub fn show(ui: &mut egui::Ui, state: &mut GuiState) {
    // ── Tab selector ──
    ui.horizontal(|ui| {
        let mut clicked: Option<BottomTab> = None;
        for (tab, label) in [
            (BottomTab::Problems, "Problems"),
            (BottomTab::Console, "Console"),
            (BottomTab::Signals, "Signals"),
        ] {
            let count = match tab {
                BottomTab::Problems => state.diagnostics.len(),
                BottomTab::Console => state.console.len(),
                BottomTab::Signals => state.signals.len(),
            };
            let text = egui::RichText::new(format!("{} ({})", label, count)).size(12.0);
            if ui.selectable_label(state.bottom_tab == tab, text).clicked() {
                clicked = Some(tab);
            }
        }
        if let Some(t) = clicked {
            state.bottom_tab = t;
        }
    });
    ui.separator();

    let height = ui.available_height().max(80.0);
    egui::ScrollArea::vertical()
        .id_salt("bottom_scroll")
        .max_height(height)
        .show(ui, |ui| match state.bottom_tab {
            BottomTab::Problems => problems_tab(ui, state),
            BottomTab::Console => console_tab(ui, state),
            BottomTab::Signals => signals_tab(ui, state),
        });
}

fn problems_tab(ui: &mut egui::Ui, state: &mut GuiState) {
    if state.diagnostics.is_empty() {
        ui.label(egui::RichText::new("No problems detected").weak().italics());
        return;
    }
    for d in &state.diagnostics {
        let (icon, color) = match d.level {
            DiagLevel::Error => ("✖", egui::Color32::from_rgb(239, 68, 68)),
            DiagLevel::Warning => ("⚠", egui::Color32::from_rgb(234, 179, 8)),
            DiagLevel::Info => ("ℹ", egui::Color32::from_rgb(59, 130, 246)),
        };
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(icon).color(color));
            ui.label(
                egui::RichText::new(format!("{}:{}", d.file, d.line))
                    .weak()
                    .monospace()
                    .size(11.0),
            );
            ui.label(egui::RichText::new(&d.message).size(12.0).color(color));
        });
    }
}

fn console_tab(ui: &mut egui::Ui, state: &mut GuiState) {
    if state.console.is_empty() {
        ui.label(egui::RichText::new("Console kosong").weak().italics());
        return;
    }
    for line in &state.console {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(&line.time)
                    .weak()
                    .monospace()
                    .size(11.0),
            );
            ui.label(egui::RichText::new(&line.msg).monospace().size(12.0));
        });
    }
}

fn signals_tab(ui: &mut egui::Ui, state: &mut GuiState) {
    if state.signals.is_empty() {
        ui.label(egui::RichText::new("Jalankan simulasi untuk melihat signal").weak().italics());
        return;
    }
    egui::Grid::new("signals_grid")
        .striped(true)
        .num_columns(4)
        .min_col_width(90.0)
        .show(ui, |ui| {
            ui.label(egui::RichText::new("Signal").strong().size(11.0));
            ui.label(egui::RichText::new("Width").strong().size(11.0));
            ui.label(egui::RichText::new("Value (hex)").strong().size(11.0));
            ui.label(egui::RichText::new("Kind").strong().size(11.0));
            ui.end_row();

            for s in &state.signals {
                ui.label(egui::RichText::new(&s.name).monospace().size(12.0));
                ui.label(s.width.to_string());
                ui.label(
                    egui::RichText::new(&s.value)
                        .monospace()
                        .color(egui::Color32::from_rgb(79, 193, 255)),
                );
                ui.label(egui::RichText::new(&s.kind).weak().size(11.0));
                ui.end_row();
            }
        });
}
