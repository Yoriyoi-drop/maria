//! Toolbar atas: brand, project name, tombol open/compile/run/stop.

use eframe::egui;

use super::super::state::GuiState;

pub fn show(ui: &mut egui::Ui, state: &mut GuiState) {
    ui.horizontal(|ui| {
        // ── Brand ──
        ui.label(
            egui::RichText::new("Maria")
                .strong()
                .size(15.0)
                .color(ui.visuals().selection.bg_fill),
        );
        ui.separator();

        // ── Nama proyek ──
        if state.project_name.is_empty() {
            ui.label(egui::RichText::new("No project").weak().italics());
        } else {
            ui.label(egui::RichText::new(&state.project_name).strong());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // ── Max time ──
            ui.label("T=");
            ui.add(egui::DragValue::new(&mut state.max_time).range(1..=100_000_000).speed(100));

            // ── Run / Stop ──
            if state.is_running {
                if ui
                    .button(egui::RichText::new("⏹ Stop").color(egui::Color32::from_rgb(239, 68, 68)))
                    .on_hover_text("Stop (F5)")
                    .clicked()
                {
                    super::super::app::trigger_stop(state);
                }
            } else {
                if ui
                    .button(egui::RichText::new("▶ Run").color(egui::Color32::from_rgb(34, 197, 94)))
                    .on_hover_text("Run simulation (F5)")
                    .clicked()
                {
                    // Aksi di-handle di app.rs via state flag
                    super::super::app::trigger_run(state);
                }
            }

            // ── Compile ──
            if ui
                .button("Compile")
                .on_hover_text("Compile + elaborate (F7)")
                .clicked()
            {
                super::super::app::trigger_compile(state);
            }

            // ── Open project ──
            if ui
                .button("Open Project")
                .on_hover_text("Buka folder proyek (Ctrl+O)")
                .clicked()
            {
                super::super::app::trigger_open_project(state);
            }
        });
    });
}
