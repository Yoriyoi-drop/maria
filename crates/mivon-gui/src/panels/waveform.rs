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

use std::collections::HashMap;
use std::path::PathBuf;

use mivon_core::Symbol;
use mivon_ir::IrDesign;

use super::super::state::{word_count, GuiState, WaveformSignal};

// ── Palet ──
const GRID_COLOR: egui::Color32 = egui::Color32::from_rgb(46, 49, 56);
const WAVE_COLOR: egui::Color32 = egui::Color32::from_rgb(122, 200, 255);
const VALUE_COLOR: egui::Color32 = egui::Color32::from_rgb(165, 175, 195);
const CURSOR_COLOR: egui::Color32 = egui::Color32::from_rgb(239, 68, 68);
const BUS_LINE_COLOR: egui::Color32 = egui::Color32::from_gray(75);

const NAME_W: f32 = 190.0;
const ROW_H: f32 = 22.0;
const HEADER_H: f32 = 26.0;

/// Warna overlay run sebelumnya di mode Compare (abu redup).
const OVERLAY_COLOR: egui::Color32 = egui::Color32::from_rgb(110, 118, 129);

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

    // Index run sebelumnya untuk mode Compare — borrow field `prev_waveform`
    // (disjoint dari `signals = &state.waveform`; pemakaian terakhir di dalam
    // ScrollArea closure, jadi tidak menghalangi borrow mut setelahnya).
    let prev_by_name: HashMap<&str, &WaveformSignal> = state
        .prev_waveform
        .iter()
        .map(|s| (s.name.as_str(), s))
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
        if ui
            .button("Fit")
            .on_hover_text("Sesuaikan zoom dengan lebar panel")
            .clicked()
        {
            let avail = ui.available_width().max(240.0);
            state.wave_zoom = ((avail - NAME_W) / (t_end as f32).max(1.0)).clamp(0.5, 256.0);
        }
        // ── Mode Compare: overlay waveform run sebelumnya (baseline disimpan
        // saat F5 berikutnya) — segmen nilai berbeda di-highlight merah.
        ui.separator();
        let has_prev = !state.prev_waveform.is_empty();
        let cmp_resp = ui
            .add_enabled(
                has_prev,
                egui::Button::new(if state.wave_compare {
                    "⟲ Compare: ON"
                } else {
                    "⟲ Compare"
                }),
            )
            .on_hover_text("Overlay waveform run sebelumnya; segmen berbeda di-highlight merah");
        if cmp_resp.clicked() && has_prev {
            state.wave_compare = !state.wave_compare;
        }
        if has_prev && state.wave_compare {
            let mism = count_mismatches(&state.waveform, &state.prev_waveform);
            let color = if mism > 0 {
                egui::Color32::from_rgb(239, 68, 68)
            } else {
                egui::Color32::from_rgb(34, 197, 94)
            };
            ui.label(
                egui::RichText::new(format!("{} mismatch", mism))
                    .monospace()
                    .size(11.0)
                    .color(color),
            );
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
    // Nama sinyal yang diklik — dibuka deklarasinya SETELAH ScrollArea
    // (borrow `state` bersih; di dalam closure hanya field terpisah yang
    // dipinjam, sama seperti `wave_zoom`/`waveform`).
    let mut want_goto: Option<String> = None;

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
                    let label_resp = ui
                        .add_sized(
                            [NAME_W, ROW_H],
                            egui::Label::new(
                                egui::RichText::new(format!(
                                    "{} {}  [{}]",
                                    icon, sig.name, sig.width
                                ))
                                .monospace()
                                .size(11.0),
                            )
                            .truncate()
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_text("Klik: buka deklarasi sinyal di RTL");
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(wf_w, ROW_H), egui::Sense::click());
                    // Mode Compare: overlay run sebelumnya + stripe segmen beda.
                    if state.wave_compare {
                        match prev_by_name.get(sig.name.as_str()) {
                            Some(prev) => paint_signal_compare(ui, rect, sig, prev, scale),
                            None => paint_signal(ui, rect, sig, scale),
                        }
                    } else {
                        paint_signal(ui, rect, sig, scale);
                    }
                    let _ = resp
                        .clone()
                        .on_hover_text("Klik: buka deklarasi sinyal di RTL");
                    // Klik nama atau area trace → catat sinyal untuk resolve
                    // sumber (dieksekusi setelah ScrollArea selesai).
                    if resp.clicked() || label_resp.clicked() {
                        want_goto = Some(sig.name.clone());
                    }
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
                    ui.painter().vline(
                        p.x,
                        wf.top()..=wf.bottom(),
                        egui::Stroke::new(1.0, CURSOR_COLOR),
                    );
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
                line.push_str(&format!(
                    "   … +{} sinyal lagi",
                    visible.len() - MAX_READOUT
                ));
                break;
            }
            line.push_str(&format!("   {} = {}", sig.name, value_at(sig, t)));
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(line)
                    .monospace()
                    .size(11.0)
                    .color(CURSOR_COLOR),
            );
        });
    }

    // ── Klik sinyal → buka deklarasi di RTL. Ditempatkan PALING AKHIR:
    // `visible` (referensi ke `state.waveform`) dipakai sampai readout strip
    // selesai — memanggil `open_signal_declaration(..., state)` (borrow mut)
    // sebelum itu memicu E0502. File target = module pemilik sinyal; baris =
    // deklarasi heuristic (lihat `find_signal_decl_line`). ──
    if let Some(name) = want_goto {
        open_signal_declaration(state, &name);
    }
}

/// Gambar sumbu waktu: grid vertikal + label tick tiap step "nice".
fn paint_time_axis(ui: &mut egui::Ui, rect: egui::Rect, scale: f32) {
    let painter = ui.painter();
    let step = nice_step(scale);
    let mut t = 0u64;
    while t as f32 * scale <= rect.width() {
        let x = rect.left() + t as f32 * scale;
        painter.vline(
            x,
            rect.top()..=rect.bottom(),
            egui::Stroke::new(1.0, GRID_COLOR),
        );
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
    paint_signal_styled(ui, rect, sig, scale, WAVE_COLOR, Some(VALUE_COLOR));
}

/// Mode Compare: gambar run ini (normal) + segmen waktu yang nilainya
/// berbeda dengan run sebelumnya (stripe merah di belakang) + overlay run
/// sebelumnya (abu, tanpa label nilai).
fn paint_signal_compare(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    cur: &WaveformSignal,
    prev: &WaveformSignal,
    scale: f32,
) {
    // Stripe merah pada segmen yang berbeda (di belakang waveform).
    // `from_rgba_unmultiplied` bukan const fn — dibuat per panggilan.
    let diff_fill = egui::Color32::from_rgba_unmultiplied(239, 68, 68, 60);
    for (t0, t1) in diff_segments(cur, prev) {
        let x0 = rect.left() + t0 as f32 * scale;
        let x1 = rect.left() + t1 as f32 * scale;
        if x1 > x0 {
            ui.painter().rect_filled(
                egui::Rect::from_min_max(egui::pos2(x0, rect.top()), egui::pos2(x1, rect.bottom())),
                0.0,
                diff_fill,
            );
        }
    }
    // Run ini — warna normal.
    paint_signal(ui, rect, cur, scale);
    // Overlay run sebelumnya — abu redup, tanpa label nilai.
    paint_signal_styled(ui, rect, prev, scale, OVERLAY_COLOR, None);
}

/// Versi berparameter warna — dipakai `paint_signal` (normal) dan overlay run
/// sebelumnya di mode Compare (`value_color = None` → tanpa label nilai, cukup
/// garis duplikasi / garis dasar bus).
fn paint_signal_styled(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    sig: &WaveformSignal,
    scale: f32,
    wave_color: egui::Color32,
    value_color: Option<egui::Color32>,
) {
    let painter = ui.painter();
    let trace = &sig.trace;
    if trace.is_empty() {
        return;
    }
    if sig.width == 1 {
        let (top_y, bot_y) = (rect.top() + 3.0, rect.bottom() - 4.0);
        let mid_y = rect.center().y;
        let stroke = egui::Stroke::new(1.5, wave_color);
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
        // Label nilai kecil di tiap transisi (di-skip saat overlay).
        if let Some(vc) = value_color {
            for (t, v) in trace.iter() {
                let x = rect.left() + (*t as f32) * scale;
                painter.text(
                    egui::pos2(x + 4.0, rect.top() + 2.0),
                    egui::Align2::LEFT_TOP,
                    v,
                    egui::FontId::monospace(9.0),
                    vc,
                );
            }
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
            if let Some(vc) = value_color {
                if x1 - x0 >= 18.0 {
                    painter.text(
                        egui::pos2((x0 + x1) / 2.0, mid_y),
                        egui::Align2::CENTER_CENTER,
                        bin_to_hex(v),
                        egui::FontId::monospace(10.0),
                        vc,
                    );
                }
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

// ─────────────────────────── Compare antar-run ───────────────────────────

/// Jumlah total segmen nilai berbeda antar dua run (per signal, dijumlahkan) —
/// badge mismatch di kontrol Compare.
fn count_mismatches(cur: &[WaveformSignal], prev: &[WaveformSignal]) -> usize {
    cur.iter()
        .filter_map(|s| {
            prev.iter()
                .find(|p| p.name == s.name)
                .map(|p| diff_segments(s, p).len())
        })
        .sum()
}

/// Nilai mentah (string trace) pada waktu `t` — nilai terakhir ≤ t. Tanpa
/// konversi display (hex untuk bus) supaya perbandingan compare konsisten —
/// dua run dengan nilai biner yang sama dianggap identik.
fn value_raw_at(sig: &WaveformSignal, t: u64) -> Option<&str> {
    sig.trace
        .iter()
        .rev()
        .find(|(tt, _)| *tt <= t)
        .map(|(_, v)| v.as_str())
}

/// Segmen waktu `(t_start..t_end)` di mana nilai `cur` vs `prev` BERBEDA.
/// Titik waktu = gabungan trace keduanya (step-function); nilai tiap interval
/// dieval via `value_raw_at`. Interval terakhir yang masih beda ditutup di
/// `last_time + 1`. Trace kosong → tanpa segmen.
fn diff_segments(cur: &WaveformSignal, prev: &WaveformSignal) -> Vec<(u64, u64)> {
    let mut segs: Vec<(u64, u64)> = Vec::new();
    if cur.trace.is_empty() || prev.trace.is_empty() {
        return segs;
    }
    let mut times: Vec<u64> = cur
        .trace
        .iter()
        .chain(prev.trace.iter())
        .map(|(t, _)| *t)
        .collect();
    times.sort_unstable();
    times.dedup();

    let mut start: Option<u64> = None;
    for &t in &times {
        let diff = value_raw_at(cur, t) != value_raw_at(prev, t);
        match (start, diff) {
            (None, true) => start = Some(t),
            (Some(s), false) => {
                segs.push((s, t));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        segs.push((s, times.last().copied().unwrap_or(s) + 1));
    }
    segs
}

// ───────────────────────── Klik sinyal → deklarasi ─────────────────────────

/// Pisahkan nama sinyal waveform menjadi (owner scope, nama signal): "u1.q" →
/// ("u1", "q"); nama bare → (None, nama). Nama invalid (scope/signal kosong)
/// diperlakukan sebagai bare (fallback ke modul top).
fn split_scope(name: &str) -> (Option<String>, String) {
    match name.split_once('.') {
        Some((m, s)) if !m.is_empty() && !s.is_empty() => (Some(m.to_string()), s.to_string()),
        _ => (None, name.to_string()),
    }
}

/// Cari nama module dari nama instance (scan sub_instances seluruh design —
/// waveform scope bisa berupa nama instance, bukan nama module).
fn instance_to_module(design: &IrDesign, inst: &str) -> Option<String> {
    let check = |m: &mivon_ir::IrModule| {
        m.sub_instances
            .iter()
            .find(|i| i.instance_name.as_str() == inst)
            .map(|i| i.module_name.to_string())
    };
    if let Some(m) = check(&design.top) {
        return Some(m);
    }
    design.modules.values().find_map(check)
}

/// Resolve nama sinyal waveform → (file RTL module pemilik, baris deklarasi).
///
/// Alur: pisahkan scope → tentukan module pemilik (nama module langsung, nama
/// instance, atau fallback modul top) → file dari `module_files` → baris
/// deklarasi heuristic dari isi file. Baris `None` = module ditemukan tapi
/// deklarasi tidak terdeteksi.
pub fn decl_location(state: &GuiState, name: &str) -> Option<(PathBuf, Option<usize>)> {
    let design = state.design.as_ref()?;
    let ci = state.compile_info.as_ref()?;
    let (owner, sig) = split_scope(name);

    let top_name = design.top.name.to_string();
    let module_name = match &owner {
        Some(m) if *m == top_name => top_name,
        Some(m) if design.modules.contains_key(&Symbol::intern(m)) => m.clone(),
        Some(m) => instance_to_module(design, m).unwrap_or(top_name),
        None => top_name,
    };

    let path = ci.module_files.get(&module_name)?.clone();
    let content = std::fs::read_to_string(&path).ok()?;
    let line = find_signal_decl_line(&content, &sig);
    Some((path, line))
}

/// Baris (1-based) deklarasi `sig` di file — scan heuristic per-baris: baris
/// berisi kata utuh `sig` DAN diawali keyword deklarasi data
/// (logic/reg/wire/bit/input/output/inout/tri). Mengembalikan baris pertama
/// yang cocok; `None` bila tidak ada. Cukup untuk navigasi — bukan parser.
fn find_signal_decl_line(content: &str, sig: &str) -> Option<usize> {
    const DECL_KW: &[&str] = &[
        "logic", "reg", "wire", "bit", "input", "output", "inout", "tri",
    ];
    if sig.is_empty() {
        return None;
    }
    for (i, raw) in content.lines().enumerate() {
        let code = raw.split("//").next().unwrap_or(raw).trim_start();
        if code.is_empty() {
            continue;
        }
        let first = code.split_whitespace().next().unwrap_or("");
        if DECL_KW.contains(&first) && word_count(code, sig) > 0 {
            return Some(i + 1);
        }
    }
    None
}

/// Buka file module pemilik sinyal & lompat ke baris deklarasi (via
/// `pending_goto` editor, sama seperti navigasi diagnostic di Problems tab).
fn open_signal_declaration(state: &mut GuiState, name: &str) {
    match decl_location(state, name) {
        Some((path, Some(line))) => {
            if !state.open_files.iter().any(|of| of.path == path) {
                state.open_file(path.clone());
            }
            if let Some(idx) = state.open_files.iter().position(|of| of.path == path) {
                state.active_file = Some(idx);
                state.open_files[idx].pending_goto = Some(line);
            }
            state.log(format!(
                "→ Deklarasi '{}': {}:{}",
                name,
                path.display(),
                line
            ));
        }
        Some((path, None)) => {
            state.open_file(path);
            state.log(format!(
                "→ Deklarasi '{}': file module dibuka (baris tak terdeteksi)",
                name
            ));
        }
        None => {
            state.log(format!(
                "⚠ Deklarasi '{}' tidak ditemukan — compile dulu?",
                name
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_scope_bare_and_scoped() {
        assert_eq!(split_scope("u1.q"), (Some("u1".into()), "q".into()));
        assert_eq!(split_scope("clk"), (None, "clk".into()));
        // scope/signal kosong → diperlakukan sebagai nama bare.
        assert_eq!(split_scope(".."), (None, "..".into()));
        assert_eq!(split_scope(".x"), (None, ".x".into()));
    }

    #[test]
    fn decl_line_simple_signal() {
        let src = "module top;\n  logic clk;\nendmodule\n";
        assert_eq!(find_signal_decl_line(src, "clk"), Some(2));
    }

    #[test]
    fn decl_line_port_with_range() {
        let src = "module m (\n  input logic [7:0] data_in,\n  output logic [7:0] data_out\n);\nendmodule\n";
        assert_eq!(find_signal_decl_line(src, "data_in"), Some(2));
        assert_eq!(find_signal_decl_line(src, "data_out"), Some(3));
    }

    #[test]
    fn decl_line_multi_decl_same_line() {
        let src = "module m;\n  logic a, b, c;\nendmodule\n";
        assert_eq!(find_signal_decl_line(src, "c"), Some(2));
        assert_eq!(find_signal_decl_line(src, "b"), Some(2));
    }

    #[test]
    fn decl_line_picks_decl_not_assignment() {
        // `y` dideklarasikan baris 2; baris assign bukan deklarasi — tetap
        // ambil baris deklarasi pertama.
        let src = "module m;\n  logic y;\n  assign y = 1'b0;\nendmodule\n";
        assert_eq!(find_signal_decl_line(src, "y"), Some(2));
        // `z` tidak pernah dideklarasikan (hanya di-assign) → None.
        let no_decl = "module m;\n  assign z = 1'b0;\nendmodule\n";
        assert_eq!(find_signal_decl_line(no_decl, "z"), None);
    }

    #[test]
    fn decl_line_ignores_always_block_for_clk() {
        // `clk` ada di deklarasi port (baris 2) DAN di `always @(posedge clk)`
        // (baris 3, bukan deklarasi) — ambil baris deklarasi.
        let src = "module m;\n  input logic clk,\n  always @(posedge clk) begin end\nendmodule\n";
        assert_eq!(find_signal_decl_line(src, "clk"), Some(2));
    }

    // ── Compare antar-run ──

    fn wf(name: &str, trace: Vec<(u64, &str)>) -> WaveformSignal {
        WaveformSignal {
            name: name.into(),
            width: 1,
            trace: trace.into_iter().map(|(t, v)| (t, v.to_string())).collect(),
        }
    }

    #[test]
    fn diff_segments_identical_traces_empty() {
        let a = wf("q", vec![(0, "0"), (10, "1")]);
        assert!(diff_segments(&a, &a).is_empty());
    }

    #[test]
    fn diff_segments_transition_shift() {
        // a naik di t=10, b di t=20 → beda hanya [10,20).
        let a = wf("q", vec![(0, "0"), (10, "1")]);
        let b = wf("q", vec![(0, "0"), (20, "1")]);
        assert_eq!(diff_segments(&a, &b), vec![(10, 20)]);
    }

    #[test]
    fn diff_segments_two_windows() {
        // b selalu 0; a naik di t=5 dan t=15 → dua jendela beda.
        let a = wf(
            "q",
            vec![(0, "0"), (5, "1"), (10, "0"), (15, "1"), (20, "0")],
        );
        let b = wf(
            "q",
            vec![(0, "0"), (5, "0"), (10, "0"), (15, "0"), (20, "0")],
        );
        assert_eq!(diff_segments(&a, &b), vec![(5, 10), (15, 20)]);
    }

    #[test]
    fn diff_segments_untouched_signal_empty() {
        let a = wf("clk", vec![(0, "0"), (5, "1"), (10, "0")]);
        let b = wf("clk", vec![(0, "0"), (5, "1"), (10, "0")]);
        assert!(diff_segments(&a, &b).is_empty());
    }

    #[test]
    fn diff_segments_empty_trace_no_segments() {
        let a = wf("q", Vec::new());
        let b = wf("q", vec![(0, "0"), (10, "1")]);
        assert!(diff_segments(&a, &b).is_empty());
    }

    #[test]
    fn count_mismatches_sums_per_signal() {
        let a = vec![
            wf("q", vec![(0, "0"), (10, "1")]),
            wf("r", vec![(0, "0"), (10, "1")]),
        ];
        let b = vec![
            wf("q", vec![(0, "0"), (20, "1")]),
            wf("r", vec![(0, "0"), (10, "1")]),
        ];
        // q beda 1 segmen; r identik → total 1.
        assert_eq!(count_mismatches(&a, &b), 1);
    }
}
