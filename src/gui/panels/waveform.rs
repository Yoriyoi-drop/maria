//! Waveform viewer — diagram timing terintegrasi (CLK/RESET/READY/VALID…).
//!
//! Data berasal dari `GuiState.waveform` (trace transisi per signal, hasil
//! parse VCD oleh backend). Rendering memakai `Painter` pada rect screen-space
//! yang dialokasikan di dalam `ScrollArea::both()` — egui menggeser posisi
//! otomatis saat di-scroll, jadi tidak perlu sinkronisasi offset manual.
//!
//! Interaksi: zoom via slider/tombol (±) atau tombol Fit; hover untuk cursor
//! vertikal + readout nilai semua signal pada waktu tersebut.

use eframe::egui;

use super::super::state::{GuiState, WaveformSignal};

// ── Palet ──
const GRID_COLOR: egui::Color32 = egui::Color32::from_rgb(46, 49, 56);
const WAVE_COLOR: egui::Color32 = egui::Color32::from_rgb(122, 200, 255);
const VALUE_COLOR: egui::Color32 = egui::Color32::from_rgb(165, 175, 195);
const CURSOR_COLOR: egui::Color32 = egui::Color32::from_rgb(239, 68, 68);
const BUS_LINE_COLOR: egui::Color32 = egui::Color32::from_gray(75);

const NAME_W: f32 = 190.0;
const ROW_H: f32 = 22.0;
const HEADER_H: f32 = 26.0;

pub fn show(ui: &mut egui::Ui, state: &mut GuiState) {
    if state.waveform.is_empty() {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Jalankan simulasi untuk melihat waveform")
                .weak()
                .italics(),
        );
        return;
    }

    // Borrow (bukan clone) — rendering hanya membaca; `wave_zoom` / `wave_hidden`
    // yang di-mutasi kontrol adalah field terpisah sehingga borrow field-level aman.
    let signals = &state.waveform;
    let t_end = signals
        .iter()
        .flat_map(|s| s.trace.iter().map(|(t, _)| *t))
        .max()
        .unwrap_or(0);
    let scale = state.wave_zoom.max(0.2);

    // Signal yang terlihat — filter `wave_hidden` (pemilih signal di bawah).
    let visible: Vec<&WaveformSignal> = signals
        .iter()
        .filter(|s| !state.wave_hidden.contains(&s.name))
        .collect();

    // ── Kontrol zoom + pemilih signal ──
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Zoom").weak().size(11.0));
        if ui.button("−").clicked() {
            state.wave_zoom = (state.wave_zoom / 1.5).max(0.5);
        }
        ui.add(
            egui::Slider::new(&mut state.wave_zoom, 0.5..=256.0)
                .logarithmic(true)
                .show_value(false),
        );
        if ui.button("+").clicked() {
            state.wave_zoom = (state.wave_zoom * 1.5).min(1024.0);
        }
        if ui.button("Fit").on_hover_text("Sesuaikan zoom dengan lebar panel").clicked() {
            let avail = ui.available_width().max(240.0);
            state.wave_zoom = ((avail - NAME_W) / (t_end as f32).max(1.0)).clamp(0.5, 256.0);
        }
        ui.separator();
        ui.label(
            egui::RichText::new(format!(
                "{} signal · T_max = {} · {:.1} px/unit · Ctrl+scroll = zoom",
                visible.len(),
                t_end,
                scale
            ))
            .weak()
            .size(11.0),
        );
        // ── Pemilih signal: centang = tampil, hapus centang = sembunyikan.
        ui.separator();
        egui::ComboBox::from_id_salt("wave_sig_picker")
            .selected_text(format!("{} / {} signal", visible.len(), signals.len()))
            .show_ui(ui, |ui| {
                for sig in signals {
                    let mut on = !state.wave_hidden.contains(&sig.name);
                    if ui.checkbox(&mut on, &sig.name).changed() {
                        if on {
                            state.wave_hidden.remove(&sig.name);
                        } else {
                            state.wave_hidden.insert(sig.name.clone());
                        }
                    }
                }
                ui.separator();
                if ui.button("Tampilkan semua").clicked() {
                    state.wave_hidden.clear();
                }
            });
    });
    ui.separator();

    if visible.is_empty() {
        ui.label(
            egui::RichText::new("Semua signal disembunyikan — centang di pemilih signal")
                .weak()
                .italics(),
        );
        return;
    }

    let wf_w = (t_end as f32 + 8.0) * scale;
    let mut wf_rect: Option<egui::Rect> = None;
    let mut readout: Option<u64> = None;

    egui::ScrollArea::both()
        .id_salt("waveform_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // ── Header: time axis ──
            ui.horizontal(|ui| {
                ui.add_sized(
                    [NAME_W, HEADER_H],
                    egui::Label::new(egui::RichText::new("Signal").strong().size(11.0)),
                );
                let (hrect, _) =
                    ui.allocate_exact_size(egui::vec2(wf_w, HEADER_H), egui::Sense::hover());
                paint_time_axis(ui, hrect, scale);
                wf_rect = Some(hrect);
            });
            ui.separator();

            // ── Baris per signal (hanya yang terlihat) ──
            for sig in &visible {
                ui.horizontal(|ui| {
                    let icon = if sig.width == 1 { "─" } else { "≡" };
                    ui.add_sized(
                        [NAME_W, ROW_H],
                        egui::Label::new(
                            egui::RichText::new(format!("{} {}  [{}]", icon, sig.name, sig.width))
                                .monospace()
                                .size(11.0),
                        )
                        .truncate(),
                    );
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(wf_w, ROW_H), egui::Sense::hover());
                    paint_signal(ui, rect, sig, scale);
                    wf_rect = Some(match wf_rect {
                        Some(r) => r.union(rect),
                        None => rect,
                    });
                });
            }

            // ── Cursor hover + Ctrl+scroll zoom (seluruh area waveform) ──
            if let Some(wf) = wf_rect {
                let resp = ui.interact(wf, egui::Id::new("waveform_hover"), egui::Sense::hover());
                if resp.hovered() {
                    // Ctrl+scroll (atau pinch trackpad) = zoom horizontal.
                    // Catatan: ScrollArea sudah membaca scroll delta sebelum
                    // closure konten berjalan, jadi area boleh ikut scroll
                    // sedikit saat zoom — diterima (tidak bisa di-consume).
                    let zd = ui.input(|i| i.zoom_delta());
                    if zd != 1.0 {
                        state.wave_zoom = (state.wave_zoom * zd).clamp(0.5, 1024.0);
                    }
                }
                if let Some(p) = resp.hover_pos() {
                    let t = ((p.x - wf.left()) / scale).max(0.0) as u64;
                    readout = Some(t);
                    ui.painter()
                        .vline(p.x, wf.top()..=wf.bottom(), egui::Stroke::new(1.0, CURSOR_COLOR));
                }
            }
        });

    // ── Readout strip: nilai signal pada waktu kursor (dibatasi — jangan
    // overflow untuk desain besar) ──
    if let Some(t) = readout {
        const MAX_READOUT: usize = 12;
        let mut line = format!("t = {}", t);
        for (i, sig) in visible.iter().enumerate() {
            if i >= MAX_READOUT {
                line.push_str(&format!("   … +{} sinyal lagi", visible.len() - MAX_READOUT));
                break;
            }
            line.push_str(&format!("   {} = {}", sig.name, value_at(sig, t)));
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(line).monospace().size(11.0).color(CURSOR_COLOR));
        });
    }
}

/// Gambar sumbu waktu: grid vertikal + label tick tiap step "nice".
fn paint_time_axis(ui: &mut egui::Ui, rect: egui::Rect, scale: f32) {
    let painter = ui.painter();
    let step = nice_step(scale);
    let mut t = 0u64;
    while t as f32 * scale <= rect.width() {
        let x = rect.left() + t as f32 * scale;
        painter.vline(x, rect.top()..=rect.bottom(), egui::Stroke::new(1.0, GRID_COLOR));
        painter.text(
            egui::pos2(x + 4.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            format!("{}", t),
            egui::FontId::monospace(10.0),
            egui::Color32::from_gray(165),
        );
        t += step;
    }
}

/// Step "nice" (1/2/5 × 10^k) agar jarak antar tick ~90px.
fn nice_step(scale: f32) -> u64 {
    let target = (90.0 / scale.max(0.01)).max(1.0);
    let mag = 10f64.powf((target as f64).log10().floor());
    let mut step = mag;
    for m in [1.0, 2.0, 5.0, 10.0] {
        if mag * m >= target as f64 {
            step = mag * m;
            break;
        }
    }
    step.max(1.0) as u64
}

/// Gambar satu baris sinyal: step-line untuk 1-bit, label nilai untuk bus.
fn paint_signal(ui: &mut egui::Ui, rect: egui::Rect, sig: &WaveformSignal, scale: f32) {
    let painter = ui.painter();
    let trace = &sig.trace;
    if trace.is_empty() {
        return;
    }
    if sig.width == 1 {
        let (top_y, bot_y) = (rect.top() + 3.0, rect.bottom() - 4.0);
        let mid_y = rect.center().y;
        let stroke = egui::Stroke::new(1.5, WAVE_COLOR);
        let mut y_prev = level_y(&trace[0].1, top_y, bot_y, mid_y);
        let mut x_prev = rect.left();
        for (i, (t, v)) in trace.iter().enumerate() {
            let x = rect.left() + (*t as f32) * scale;
            let y = level_y(v, top_y, bot_y, mid_y);
            if i > 0 {
                painter.line_segment([egui::pos2(x_prev, y_prev), egui::pos2(x, y_prev)], stroke);
                if (y - y_prev).abs() > 0.5 {
                    painter.line_segment([egui::pos2(x, y_prev), egui::pos2(x, y)], stroke);
                }
            }
            x_prev = x;
            y_prev = y;
        }
        painter.line_segment(
            [egui::pos2(x_prev, y_prev), egui::pos2(rect.right(), y_prev)],
            stroke,
        );
        // Label nilai kecil di tiap transisi
        for (t, v) in trace.iter() {
            let x = rect.left() + (*t as f32) * scale;
            painter.text(
                egui::pos2(x + 4.0, rect.top() + 2.0),
                egui::Align2::LEFT_TOP,
                v,
                egui::FontId::monospace(9.0),
                VALUE_COLOR,
            );
        }
    } else {
        // Bus: label nilai (hex) per segmen + garis dasar
        let mid_y = rect.center().y;
        for (i, (t, v)) in trace.iter().enumerate() {
            let x0 = rect.left() + (*t as f32) * scale;
            let x1 = trace
                .get(i + 1)
                .map(|(t2, _)| rect.left() + (*t2 as f32) * scale)
                .unwrap_or(rect.right());
            if x1 - x0 >= 18.0 {
                painter.text(
                    egui::pos2((x0 + x1) / 2.0, mid_y),
                    egui::Align2::CENTER_CENTER,
                    bin_to_hex(v),
                    egui::FontId::monospace(10.0),
                    VALUE_COLOR,
                );
            }
            painter.line_segment(
                [
                    egui::pos2(x0, rect.bottom() - 3.0),
                    egui::pos2(x1, rect.bottom() - 3.0),
                ],
                egui::Stroke::new(1.0, BUS_LINE_COLOR),
            );
        }
    }
}

fn level_y(v: &str, top: f32, bot: f32, mid: f32) -> f32 {
    match v {
        "1" => top,
        "0" => bot,
        _ => mid, // x / z
    }
}

/// Nilai sinyal pada waktu `t` (nilai terakhir ≤ t). Hex untuk bus.
fn value_at(sig: &WaveformSignal, t: u64) -> String {
    let v = sig
        .trace
        .iter()
        .rev()
        .find(|(tt, _)| *tt <= t)
        .map(|(_, v)| v.as_str())
        .unwrap_or("?");
    if sig.width == 1 {
        v.to_string()
    } else {
        bin_to_hex(v)
    }
}

/// Konversi string biner (bisa ada x/z) ke hex, trim leading zero.
fn bin_to_hex(bin: &str) -> String {
    let mut s = String::new();
    let n = bin.len();
    let mut i = n;
    while i > 0 {
        let start = i.saturating_sub(4);
        let nib = &bin[start..i];
        let mut val = 0u8;
        let mut has_x = false;
        let mut has_z = false;
        for (j, ch) in nib.chars().enumerate() {
            match ch {
                '1' => val |= 1 << (nib.len() - 1 - j),
                'x' | 'X' => has_x = true,
                'z' | 'Z' => has_z = true,
                _ => {}
            }
        }
        s.push(if has_x {
            'x'
        } else if has_z {
            'z'
        } else {
            std::char::from_digit(val as u32, 16).unwrap_or('0')
        });
        i = start;
    }
    let trimmed = s.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}
