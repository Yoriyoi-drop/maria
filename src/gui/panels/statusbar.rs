//! Status bar bawah: status compile/simulasi, jumlah module/signal, jam.

use eframe::egui;

use super::super::state::{DiagLevel, GuiState};

pub fn show(ui: &mut egui::Ui, state: &mut GuiState) {
    ui.horizontal(|ui| {
        // ── Status kiri ──
        if state.is_running {
            ui.label(
                egui::RichText::new("● Running")
                    .color(egui::Color32::from_rgb(34, 197, 94))
                    .strong(),
            );
        } else if let Some(info) = &state.compile_info {
            if info.success {
                ui.label(
                    egui::RichText::new("● Compiled")
                        .color(egui::Color32::from_rgb(34, 197, 94)),
                );
            }
        } else {
            ui.label(
                egui::RichText::new("○ Idle")
                    .weak()
                    .italics(),
            );
        }

        ui.separator();

        // ── Info proyek ──
        if !state.project_name.is_empty() {
            let mods = state.compile_info.as_ref().map(|i| i.modules.len()).unwrap_or(0);
            ui.label(
                egui::RichText::new(format!("{} · {} modules", state.project_name, mods))
                    .weak()
                    .size(11.0),
            );
            ui.separator();
        }

        // ── Simulasi ──
        if state.sim_time_ms > 0.0 {
            ui.label(
                egui::RichText::new(format!("sim {:.1}ms @ t={}", state.sim_time_ms, state.cycles))
                    .weak()
                    .size(11.0),
            );
            ui.separator();
        }

        // ── Spacer ke kanan ──
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Jam
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let h = (now / 3600) % 24;
            let m = (now / 60) % 60;
            let s = now % 60;
            ui.label(
                egui::RichText::new(format!("{:02}:{:02}:{:02}", h, m, s))
                    .weak()
                    .monospace()
                    .size(11.0),
            );
            ui.separator();

            // Error / warning count
            let errs = state
                .diagnostics
                .iter()
                .filter(|d| d.level == DiagLevel::Error)
                .count();
            let warns = state
                .diagnostics
                .iter()
                .filter(|d| d.level == DiagLevel::Warning)
                .count();
            if errs > 0 {
                ui.label(
                    egui::RichText::new(format!("{} errors", errs))
                        .color(egui::Color32::from_rgb(239, 68, 68)),
                );
            }
            if warns > 0 {
                ui.label(
                    egui::RichText::new(format!("{} warnings", warns))
                        .color(egui::Color32::from_rgb(234, 179, 8)),
                );
            }

            // Jumlah signal hasil sim
            if !state.signals.is_empty() {
                ui.label(
                    egui::RichText::new(format!("{} signals", state.signals.len()))
                        .weak()
                        .size(11.0),
                );
                ui.separator();
            }

            ui.label(egui::RichText::new("SystemVerilog").weak().size(11.0));
        });
    });
}
