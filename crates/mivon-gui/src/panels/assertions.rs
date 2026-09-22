//! Assertions tab — hasil evaluasi assertion simulasi (pass/fail per lokasi).
//!
//! Data dari `SimInfo.assertions` (engine.assertion_stats: (line, col) →
//! (pass, fail)). Ringkasan + daftar per assertion. Klik baris → buka
//! assertion di RTL bila file asal terresolusi unambiguous (scan baris
//! ber-"assert" di file module) — kalau ambigu/tidak ditemukan, hanya log
//! (jangan menebak lokasi).

use eframe::egui;

use super::super::backend::resolve_assert_file;
use super::super::state::GuiState;

/// Bar progress pass-rate (warna hijau/kuning/merah).
const BAR_W: f32 = 220.0;
const BAR_H: f32 = 14.0;

pub fn show(ui: &mut egui::Ui, state: &mut GuiState) {
    if state.assertions.is_empty() {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Jalankan simulasi dengan assertion untuk melihat hasil evaluasi")
                .weak()
                .italics(),
        );
        return;
    }

    // ── Ringkasan ──
    let total_eval: u64 = state.assertions.iter().map(|a| a.pass + a.fail).sum();
    let total_pass: u64 = state.assertions.iter().map(|a| a.pass).sum();
    let total_fail: u64 = state.assertions.iter().map(|a| a.fail).sum();
    let pass_rate = if total_eval > 0 {
        total_pass as f64 / total_eval as f64 * 100.0
    } else {
        0.0
    };
    ui.label(egui::RichText::new("Assertion summary").strong().size(12.0));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let bg = ui.visuals().widgets.noninteractive.bg_fill;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(BAR_W, BAR_H), egui::Sense::hover());
        ui.painter().rect_filled(rect, egui::CornerRadius::ZERO, bg);
        let frac = (pass_rate / 100.0) as f32;
        if frac > 0.0 {
            let fill = if pass_rate >= 90.0 {
                egui::Color32::from_rgb(34, 197, 94)
            } else if pass_rate >= 60.0 {
                egui::Color32::from_rgb(234, 179, 8)
            } else {
                egui::Color32::from_rgb(239, 68, 68)
            };
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * frac, rect.height())),
                egui::CornerRadius::ZERO,
                fill,
            );
        }
        ui.label(
            egui::RichText::new(format!(
                "{:.1}% · {} pass / {} fail · {} assertion",
                pass_rate,
                total_pass,
                total_fail,
                state.assertions.len()
            ))
            .monospace()
            .size(11.0),
        );
    });
    ui.add_space(6.0);

    // ── Daftar per assertion ──
    // Clone baris + pinjam `module_files` SEBELUM ScrollArea — di dalam
    // closure `state` tidak dipinjam (aksi di-apply setelah scroll selesai).
    let rows = state.assertions.clone();
    let module_files = state
        .compile_info
        .as_ref()
        .map(|ci| ci.module_files.clone())
        .unwrap_or_default();
    let mut to_open: Option<(std::path::PathBuf, usize)> = None;
    let mut miss_log: Option<String> = None;

    egui::ScrollArea::vertical()
        .id_salt("assertions_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("assertions_grid")
                .striped(true)
                .num_columns(4)
                .min_col_width(60.0)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("Loc").strong().size(11.0));
                    ui.label(egui::RichText::new("Pass").strong().size(11.0));
                    ui.label(egui::RichText::new("Fail").strong().size(11.0));
                    ui.label(egui::RichText::new("Total").strong().size(11.0));
                    ui.end_row();

                    for a in &rows {
                        let has_fail = a.fail > 0;
                        let loc = format!("{}:{}", a.line, a.col);
                        let resp = ui
                            .add(
                                egui::Link::new(
                                    egui::RichText::new(&loc)
                                        .monospace()
                                        .size(11.0)
                                        .color(if has_fail {
                                            egui::Color32::from_rgb(239, 68, 68)
                                        } else {
                                            ui.visuals().text_color()
                                        }),
                                ),
                            )
                            .on_hover_text("Klik: buka assertion di RTL (bila file asal jelas)");
                        if resp.clicked() {
                            match resolve_assert_file(&module_files, a.line) {
                                Some(path) => to_open = Some((path, a.line)),
                                None => {
                                    miss_log = Some(format!(
                                        "⚠ Lokasi assertion {}:{} tak bisa dipastikan (ambigu/tidak ditemukan)",
                                        a.line, a.col
                                    ));
                                }
                            }
                        }
                        ui.label(
                            egui::RichText::new(a.pass.to_string())
                                .monospace()
                                .size(11.0),
                        );
                        ui.label(
                            egui::RichText::new(a.fail.to_string())
                                .monospace()
                                .size(11.0)
                                .color(if has_fail {
                                    egui::Color32::from_rgb(239, 68, 68)
                                } else {
                                    ui.visuals().text_color()
                                }),
                        );
                        ui.label(
                            egui::RichText::new((a.pass + a.fail).to_string())
                                .monospace()
                                .size(11.0)
                                .weak(),
                        );
                        ui.end_row();
                    }
                });
        });

    // Terapkan aksi navigasi setelah ScrollArea (borrow `state` bersih).
    if let Some((path, line)) = to_open {
        if !state.open_files.iter().any(|of| of.path == path) {
            state.open_file(path.clone());
        }
        if let Some(idx) = state.open_files.iter().position(|of| of.path == path) {
            state.active_file = Some(idx);
            state.open_files[idx].pending_goto = Some(line);
        }
        state.log(format!("→ Assertion: {}:{}", path.display(), line));
    }
    if let Some(msg) = miss_log {
        state.log(msg);
    }
}
