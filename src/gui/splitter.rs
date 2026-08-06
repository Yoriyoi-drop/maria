//! Komponen splitter/resizer untuk panel bawah (IDE).
//!
//! Satu tanggung jawab: handle drag horizontal di border atas Bottom Panel
//! yang mengubah tinggi panel secara real-time dengan batasan min/max.
//!
//! - Drag-to-resize: pointer dikunci ke handle (egui `Sense::drag()`), tinggi
//!   dihitung dari pergerakan vertikal pointer sejak drag dimulai (anchor di
//!   memory egui via `insert_temp` — bertahan lintas frame, tidak di-persist
//!   ke disk).
//! - Kursor: `CursorIcon::ResizeVertical` (setara `ns-resize`/`row-resize`)
//!   saat hover maupun saat drag aktif.
//! - Constraint: `BOTTOM_MIN_HEIGHT` (tab bar + sedikit konten tetap terlihat)
//!   dan maksimal `BOTTOM_MAX_FRACTION` × tinggi window.
//!
//! Pemakaian: `panels/bottom.rs` memanggil `show_resizer` di paling atas isi
//! panel; tinggi baru disimpan di `GuiState::bottom_height` dan panel dipasang
//! dengan `exact_size` pada frame berikutnya (latensi 1 frame, tak terasa).

use eframe::egui;
use egui::CursorIcon;

/// Tinggi minimal panel bawah (px) — tab bar + sedikit konten tetap terlihat,
/// panel tidak bisa hilang total.
pub const BOTTOM_MIN_HEIGHT: f32 = 100.0;

/// Fraksi maksimal tinggi window yang boleh dipakai panel bawah — editor
/// utama tidak boleh tertutup seluruhnya.
pub const BOTTOM_MAX_FRACTION: f32 = 0.8;

/// Tinggi strip handle (px).
const HANDLE_HEIGHT: f32 = 6.0;

/// Anchor drag: tinggi panel & posisi Y pointer saat drag dimulai. Disimpan
/// di memory egui per-`Id` — `insert_temp` artinya bertahan lintas frame
/// (selama drag) tapi tidak dipersistensikan ke disk.
#[derive(Clone, Copy)]
struct DragAnchor {
    start_height: f32,
    start_y: f32,
}

/// Batas tinggi panel bawah `(min, max)` terhadap tinggi window saat ini.
/// `max` disesuaikan dengan ukuran window — panel tidak pernah melebihi
/// `BOTTOM_MAX_FRACTION` × tinggi layar.
pub fn bottom_bounds(ui: &egui::Ui) -> (f32, f32) {
    let max_h = (ui.ctx().viewport_rect().height() * BOTTOM_MAX_FRACTION).max(BOTTOM_MIN_HEIGHT);
    (BOTTOM_MIN_HEIGHT, max_h)
}

/// Tampilkan handle resize di border atas Bottom Panel.
///
/// - `id` — identitas widget (state drag disimpan per-id).
/// - `height` — tinggi panel saat ini (px).
/// - `bounds` — `(min, max)` tinggi yang diizinkan (lihat `bottom_bounds`).
///
/// Mengembalikan tinggi baru setelah drag diterapkan + di-clamp ke `bounds`.
/// Caller menyimpan hasilnya di `GuiState::bottom_height`; panel dipasang
/// dengan `exact_size` pada frame berikutnya.
///
/// Saat drag aktif, pointer di-kunci ke handle ini (`dragged_id` di memory
/// egui) sehingga widget lain — termasuk editor — tidak menerima drag;
/// dengan begitu seleksi teks tidak terjadi secara tidak sengaja saat
/// menggeser panel.
pub fn show_resizer(ui: &mut egui::Ui, id: egui::Id, height: f32, bounds: (f32, f32)) -> f32 {
    let (min_h, max_h) = bounds;
    let mut out = height.clamp(min_h, max_h);

    // Strip handle — Sense::drag membuat pointer di-kunci ke widget ini saat
    // drag dimulai (mencegah seleksi teks di editor di bawahnya).
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), HANDLE_HEIGHT),
        egui::Sense::drag(),
    );

    // ── Kursor ns-resize (row-resize) saat hover/drag ──
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
    }

    // ── Visual: strip + garis aksen + grip dots di tengah ──
    let visuals = ui.visuals();
    let fill = if resp.dragged() {
        egui::Color32::from_rgb(59, 130, 246) // aksen biru saat aktif
    } else if resp.hovered() {
        egui::Color32::from_rgb(42, 43, 48)
    } else {
        visuals.panel_fill
    };
    let line_color = if resp.dragged() {
        egui::Color32::WHITE
    } else if resp.hovered() {
        egui::Color32::from_rgb(139, 197, 255)
    } else {
        egui::Color32::from_rgb(75, 77, 84)
    };

    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, fill);
    let cy = rect.center().y;
    // Garis aksen 1px di tengah strip.
    painter.rect_filled(
        egui::Rect::from_min_max(egui::pos2(rect.left(), cy - 0.5), egui::pos2(rect.right(), cy + 0.5)),
        0.0,
        line_color,
    );
    // Grip dots kecil (⋯) sebagai affordance visual "tarik di sini".
    let cx = rect.center().x;
    for dx in [-7.0, 0.0, 7.0] {
        painter.circle_filled(egui::pos2(cx + dx, cy), 1.5, line_color);
    }
    let _ = resp.clone().on_hover_text("Tarik untuk mengubah tinggi panel");

    // ── Drag → hitung tinggi baru (anchor-based, bukan delta kumulatif) ──
    if resp.drag_started() {
        if let Some(p) = resp.interact_pointer_pos() {
            ui.ctx().data_mut(|d| {
                d.insert_temp(
                    id,
                    DragAnchor {
                        start_height: out,
                        start_y: p.y,
                    },
                );
            });
        }
    }
    if resp.dragged() {
        // Real-time: pastikan repaint terus berjalan selama drag.
        ui.ctx().request_repaint();
        if let (Some(anchor), Some(p)) = (
            ui.ctx().data_mut(|d| d.get_temp::<DragAnchor>(id)),
            resp.interact_pointer_pos(),
        ) {
            out = (anchor.start_height + (p.y - anchor.start_y)).clamp(min_h, max_h);
        }
    }
    if resp.drag_stopped() {
        ui.ctx().data_mut(|d| {
            d.remove::<DragAnchor>(id);
        });
    }

    out
}
