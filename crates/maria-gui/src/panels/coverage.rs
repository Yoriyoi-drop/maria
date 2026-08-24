//! Coverage tab — persentase coverage (line, toggle, branch, FSM) hasil
//! simulasi dengan bar proporsional, plus detail covergroup per baris.
//!
//! Data berasal dari `engine.coverage_stats()` + `cover_total`/`cover_hits`
//! yang diisi worker di backend.rs. Empty state ditampilkan jika belum ada
//! simulasi atau tidak ada data coverage sama sekali.

use eframe::egui;

use super::super::state::GuiState;

const BAR_W: f32 = 220.0;
const BAR_H: f32 = 14.0;

pub fn show(ui: &mut egui::Ui, state: &mut GuiState) {
    let c = &state.coverage;
    let has_data = c.line_items > 0
        || c.toggle_signals > 0
        || c.branch_total > 0
        || c.fsm_signals > 0
        || !c.covergroups.is_empty();

    if state.signals.is_empty() || !has_data {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Jalankan simulasi untuk melihat coverage")
                .weak()
                .italics(),
        );
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(
                "Coverage aktif otomatis (line/toggle/branch/FSM) — tidak perlu flag tambahan.",
            )
            .weak()
            .size(11.0),
        );
        return;
    }

    ui.label(egui::RichText::new("Coverage summary").strong().size(12.0));
    ui.add_space(4.0);

    // ── Branch coverage (persentase penuh dari engine) ──
    let branch_pct = c.branch_percent;
    pct_row(
        ui,
        "Branch",
        branch_pct,
        &format!("{}/{}", c.branch_covered, c.branch_total),
    );

    // ── Line coverage (hits / items) ──
    let line_pct = if c.line_items > 0 {
        (c.line_hits as f64 / c.line_items as f64).min(1.0) * 100.0
    } else {
        0.0
    };
    pct_row(
        ui,
        "Line",
        line_pct,
        &format!("{}/{}", c.line_hits, c.line_items),
    );

    // ── Toggle coverage (signal dengan toggle / total signal) ──
    let toggle_pct = if state.signals.is_empty() {
        0.0
    } else {
        (c.toggle_signals as f64 / state.signals.len() as f64).min(1.0) * 100.0
    };
    pct_row(
        ui,
        "Toggle",
        toggle_pct,
        &format!("{}/{} signal", c.toggle_signals, state.signals.len()),
    );

    // ── FSM coverage (state teramati) ──
    let fsm_pct = if c.fsm_signals > 0 {
        let per_signal = c.fsm_states as f64 / c.fsm_signals as f64;
        (per_signal / 16.0).min(1.0) * 100.0 // asumsi ≤16 state per FSM
    } else {
        0.0
    };
    pct_row(
        ui,
        "FSM",
        fsm_pct,
        &format!("{} state / {} signal", c.fsm_states, c.fsm_signals),
    );

    // ── Detail covergroup ──
    if !c.covergroups.is_empty() {
        ui.add_space(10.0);
        ui.separator();
        ui.label(egui::RichText::new("Covergroups").strong().size(12.0));
        ui.add_space(4.0);
        egui::Grid::new("cov_cg_grid")
            .striped(true)
            .num_columns(3)
            .min_col_width(80.0)
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Covergroup").strong().size(11.0));
                ui.label(egui::RichText::new("Hits").strong().size(11.0));
                ui.label(egui::RichText::new("Samples").strong().size(11.0));
                ui.end_row();
                for cg in &c.covergroups {
                    let pct = if cg.total > 0 {
                        cg.hits as f64 / cg.total as f64 * 100.0
                    } else {
                        0.0
                    };
                    ui.label(egui::RichText::new(&cg.name).monospace().size(12.0));
                    ui.label(
                        egui::RichText::new(cg.hits.to_string())
                            .monospace()
                            .size(12.0),
                    );
                    ui.label(
                        egui::RichText::new(format!("{} ({:.1}%)", cg.total, pct))
                            .weak()
                            .size(11.0),
                    );
                    ui.end_row();
                }
            });
    }
}

fn pct_row(ui: &mut egui::Ui, label: &str, pct: f64, detail: &str) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [80.0, 18.0],
            egui::Label::new(egui::RichText::new(label).size(11.0)),
        );
        let (rect, _) = ui.allocate_exact_size(egui::vec2(BAR_W, BAR_H), egui::Sense::hover());
        let frac = (pct / 100.0) as f32;
        let bg = ui.visuals().widgets.noninteractive.bg_fill;
        let accent = if pct >= 90.0 {
            egui::Color32::from_rgb(34, 197, 94) // hijau — sukses
        } else if pct >= 60.0 {
            egui::Color32::from_rgb(234, 179, 8) // kuning — peringatan
        } else {
            egui::Color32::from_rgb(239, 68, 68) // merah — rendah
        };
        let fill_w = (rect.width() * frac.clamp(0.0, 1.0)).max(2.0);
        ui.painter().rect_filled(rect, egui::CornerRadius::ZERO, bg);
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(fill_w, rect.height())),
            egui::CornerRadius::ZERO,
            accent,
        );
        ui.label(
            egui::RichText::new(format!("{:.1}%  {}", pct, detail))
                .monospace()
                .size(11.0),
        );
    });
}
