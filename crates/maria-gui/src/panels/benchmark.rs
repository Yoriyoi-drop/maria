//! Benchmark tab — statistik performa simulasi (time steps, delta, events,
//! proses, NBA, sensitivitas, throughput) dengan bar proporsional.

use eframe::egui;

use super::super::state::GuiState;

const BAR_W: f32 = 260.0;
const BAR_H: f32 = 14.0;

pub fn show(ui: &mut egui::Ui, state: &mut GuiState) {
    // ── Resource Monitor realtime (selalu tampil — observatory, tanpa perlu
    // simulasi): bar CPU/RAM/Thread + grafik history. ──
    resource_section(ui, state);

    ui.add_space(8.0);
    ui.separator();

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

/// Section Resource Monitor: bar realtime CPU/RAM/Threads + grafik history
/// (CPU% dan RAM GB selama 3 menit terakhir).
fn resource_section(ui: &mut egui::Ui, state: &mut GuiState) {
    // Refresh + push riwayat hanya saat sampel BARU (1Hz) — aman dipanggil
    // bersamaan dengan status bar (refresh di-throttle internal).
    {
        let res = &mut state.resource;
        if res.refresh(std::time::Duration::from_millis(1000)) && res.cpu_percent >= 0.0 {
            state.resource_hist.push(
                res.cpu_percent as f32,
                res.mem_used_gb as f32,
                res.threads as f32,
            );
        }
    }

    ui.label(egui::RichText::new("Resource Monitor (realtime)").strong().size(12.0));
    ui.add_space(4.0);

    let res = &state.resource;
    if res.cpu_percent < 0.0 {
        ui.label(
            egui::RichText::new("— menunggu sampel pertama —")
                .weak()
                .italics()
                .size(11.0),
        );
        return;
    }
    let mem_total = res.mem_total_gb.max(0.1);
    let mem_pct = (res.mem_used_gb / mem_total).clamp(0.0, 1.0) as f32;

    resource_bar(ui, "CPU", (res.cpu_percent as f32) / 100.0, res.cpu_color(),
        format!("{:.0}%", res.cpu_percent));
    resource_bar(ui, "RAM", mem_pct, egui::Color32::from_rgb(59, 130, 246),
        format!("{:.1}G / {:.1}G", res.mem_used_gb, mem_total));
    ui.horizontal(|ui| {
        let th = res.threads as f32;
        resource_bar(ui, "Threads", th / th.max(16.0), egui::Color32::from_rgb(168, 85, 247),
            format!("{}", res.threads));
        ui.label(
            egui::RichText::new(format!("load {:.2}", res.load_1))
                .weak()
                .size(11.0),
        );
    });

    ui.add_space(6.0);

    // ── Grafik history (CPU% & RAM GB) ──
    let h = &state.resource_hist;
    if h.cpu.len() >= 2 {
        history_chart(
            ui,
            &[("CPU %", h.cpu.as_slice(), egui::Color32::from_rgb(59, 130, 246))],
            100.0,
            90.0,
            "CPU % (history)",
        );
        ui.add_space(4.0);
        history_chart(
            ui,
            &[("RAM GB", h.mem.as_slice(), egui::Color32::from_rgb(34, 197, 94))],
            mem_total as f32,
            90.0,
            "RAM GB (history)",
        );
    } else {
        ui.label(
            egui::RichText::new("grafik history tersedia setelah ~2 sampel (1Hz)")
                .weak()
                .italics()
                .size(11.0),
        );
    }
}

/// Satu baris label + progress bar resource.
fn resource_bar(ui: &mut egui::Ui, label: &str, frac: f32, color: egui::Color32, text: String) {
    ui.horizontal(|ui| {
        ui.add_sized([150.0, 18.0], egui::Label::new(label.to_owned()));
        ui.add(
            egui::ProgressBar::new(frac.clamp(0.0, 1.0))
                .desired_width(180.0)
                .fill(color)
                .text(text),
        );
    });
}

/// Grafik garis history sederhana (tanpa dependensi chart): area fill + garis
/// polyline, gridline horizontal, label sumbu Y. `y_max` = skala sumbu Y.
fn history_chart(
    ui: &mut egui::Ui,
    series: &[(&str, &[f32], egui::Color32)],
    y_max: f32,
    height: f32,
    title: &str,
) {
    ui.label(egui::RichText::new(title).weak().size(10.0));
    let width = ui.available_width().min(560.0).max(200.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius::same(4),
        ui.visuals().extreme_bg_color,
    );

    let grid_color = ui
        .visuals()
        .widgets
        .noninteractive
        .bg_stroke
        .color
        .gamma_multiply(0.45);
    for i in 0..=4usize {
        let y = rect.top() + rect.height() * (i as f32 / 4.0);
        ui.painter().line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(1.0, grid_color),
        );
    }
    ui.painter().text(
        egui::pos2(rect.right() - 4.0, rect.top() + 2.0),
        egui::Align2::RIGHT_TOP,
        format!("{:.1}", y_max),
        egui::FontId::monospace(9.0),
        grid_color,
    );

    for (name, vals, color) in series {
        if vals.len() < 2 {
            continue;
        }
        let n = vals.len() - 1;
        let pts: Vec<egui::Pos2> = vals
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let x = rect.left() + rect.width() * (i as f32 / n as f32);
                let frac = (v / y_max.max(1e-6)).clamp(0.0, 1.0);
                egui::pos2(x, rect.bottom() - rect.height() * frac)
            })
            .collect();

        // Area fill di bawah kurva — PathShape non-convex aman untuk kurva
        // yang naik-turun (beda dengan convex_polygon).
        let mut path = pts.clone();
        path.push(egui::pos2(rect.right(), rect.bottom()));
        path.push(egui::pos2(rect.left(), rect.bottom()));
        ui.painter().add(egui::Shape::Path(egui::epaint::PathShape {
            points: path,
            closed: true,
            fill: egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 26),
            stroke: egui::epaint::PathStroke::NONE,
        }));
        ui.painter().add(egui::Shape::line(pts, egui::Stroke::new(1.6, *color)));
        ui.painter().text(
            egui::pos2(rect.left() + 4.0, rect.top() + 2.0),
            egui::Align2::LEFT_TOP,
            *name,
            egui::FontId::monospace(9.0),
            *color,
        );
    }
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
