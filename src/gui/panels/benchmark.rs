//! Benchmark tab — statistik performa simulasi (time steps, delta, events,
//! proses, NBA, sensitivitas, throughput) dengan bar proporsional.

use eframe::egui;

use super::super::state::GuiState;

const BAR_W: f32 = 260.0;
const BAR_H: f32 = 14.0;

pub fn show(ui: &mut egui::Ui, state: &mut GuiState) {
    if state.cycles == 0 && state.delta_cycles == 0 {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Jalankan simulasi untuk melihat benchmark")
                .weak()
                .italics(),
        );
        return;
    }

    let rows: Vec<(&str, f64, &str)> = vec![
        ("Time steps", state.cycles as f64, "steps"),
        ("Delta cycles", state.delta_cycles as f64, "delta"),
        ("Events processed", state.events_processed as f64, "events"),
        ("Processes evaluated", state.processes_evaluated as f64, "eval"),
        ("NBA commits", state.nba_commits as f64, "commit"),
        ("Sensitivity triggers", state.sensitive_triggers as f64, "trigger"),
        ("Events / delta", state.events_per_delta, "avg"),
    ];
    let max_v = rows
        .iter()
        .map(|r| r.1)
        .fold(1.0f64, f64::max);

    ui.label(egui::RichText::new("Simulation metrics").strong().size(12.0));
    ui.add_space(4.0);
    for (label, v, unit) in &rows {
        metric_row(ui, label, *v, max_v, unit);
    }

    ui.add_space(8.0);
    ui.separator();
    ui.label(egui::RichText::new("Timing").strong().size(12.0));
    ui.add_space(4.0);

    // Waktu simulasi vs jumlah time steps → throughput (time step / detik)
    let sim_ms = state.sim_time_ms;
    let steps_per_s = if sim_ms > 0.0 {
        state.cycles as f64 / (sim_ms / 1000.0)
    } else {
        0.0
    };
    let time_rows: Vec<(&str, f64, &str)> = vec![
        ("Sim wall-clock", sim_ms, "ms"),
        ("Throughput", steps_per_s, "step/s"),
    ];
    let tmax = time_rows.iter().map(|r| r.1).fold(1.0f64, f64::max);
    for (label, v, unit) in &time_rows {
        metric_row(ui, label, *v, tmax, unit);
    }

    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(format!(
            "Simulasi {} time steps selesai dalam {:.2} ms ({:.0} signal).",
            state.cycles,
            sim_ms,
            state.signals.len()
        ))
        .weak()
        .size(11.0),
    );
}

fn metric_row(ui: &mut egui::Ui, label: &str, v: f64, max_v: f64, unit: &str) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [170.0, 18.0],
            egui::Label::new(egui::RichText::new(label).size(11.0)),
        );
        let (rect, _) = ui.allocate_exact_size(egui::vec2(BAR_W, BAR_H), egui::Sense::hover());
        let frac = (v / max_v.max(1e-9)) as f32;
        let bg = ui.visuals().widgets.noninteractive.bg_fill;
        let accent = ui.visuals().selection.bg_fill;
        let fill_w = (rect.width() * frac.clamp(0.0, 1.0)).max(2.0);
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::ZERO,
            bg,
        );
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(fill_w, rect.height())),
            egui::CornerRadius::ZERO,
            accent,
        );
        ui.label(
            egui::RichText::new(format!("{:.1} {}", v, unit))
                .monospace()
                .size(11.0),
        );
    });
}
