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

        // ── Resource monitor (CPU/RAM realtime) ──
        let res = &mut state.resource;
        res.refresh(std::time::Duration::from_millis(1000));
        if res.cpu_percent >= 0.0 {
            let cpu_color = if res.cpu_percent > 80.0 {
                egui::Color32::from_rgb(239, 68, 68)
            } else if res.cpu_percent > 50.0 {
                egui::Color32::from_rgb(234, 179, 8)
            } else {
                egui::Color32::from_rgb(34, 197, 94)
            };
            let mem_pct = if res.mem_total_gb > 0.0 {
                (res.mem_used_gb / res.mem_total_gb * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            };
            let resp = ui
                .horizontal(|ui| {
                    ui.label(egui::RichText::new("CPU").weak().size(10.0));
                    ui.add(
                        egui::ProgressBar::new(res.cpu_percent as f32 / 100.0)
                            .desired_width(60.0)
                            .fill(cpu_color)
                            .text(""),
                    );
                    ui.label(
                        egui::RichText::new(format!("{:.0}%", res.cpu_percent))
                            .monospace()
                            .size(10.0),
                    );
                    ui.separator();
                    ui.label(egui::RichText::new("RAM").weak().size(10.0));
                    ui.add(
                        egui::ProgressBar::new(mem_pct as f32 / 100.0)
                            .desired_width(60.0)
                            .fill(egui::Color32::from_rgb(59, 130, 246))
                            .text(""),
                    );
                    ui.label(
                        egui::RichText::new(format!("{:.1}G", res.mem_used_gb))
                            .monospace()
                            .size(10.0),
                    );
                })
                .response
                .on_hover_text(format!(
                    "CPU {:.0}% · RAM {:.1}G / {:.1}G · {} thread · load {:.2}",
                    res.cpu_percent, res.mem_used_gb, res.mem_total_gb, res.threads, res.load_1
                ));
            let _ = resp;
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
