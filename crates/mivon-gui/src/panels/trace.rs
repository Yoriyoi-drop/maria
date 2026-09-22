//! Trace tab — daftar event (transisi nilai) dari trace waveform hasil sim,
//! urut waktu, dengan filter sinyal & batas waktu. Data = fakta dari trace
//! asli, bukan dugaan sebab-akibat (sesuai desain gui.md §2.D). Klik event →
//! buka deklarasi sinyal di RTL (resolve sama seperti Waveform).

use eframe::egui;

use super::super::backend::flatten_events;
use super::super::state::{GuiState, TraceEvent};
use super::waveform::decl_location;

/// Maksimal baris event yang dirender (anti-bloat untuk desain besar).
const MAX_ROWS: usize = 1000;

pub fn show(ui: &mut egui::Ui, state: &mut GuiState) {
    if state.waveform.is_empty() {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Jalankan simulasi untuk melihat trace event")
                .weak()
                .italics(),
        );
        return;
    }

    let events = flatten_events(&state.waveform);

    // ── Filter: substring nama sinyal + batas waktu (0 = semua) ──
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Filter").weak().size(11.0));
        ui.add(
            egui::TextEdit::singleline(&mut state.trace_filter)
                .hint_text("nama sinyal…")
                .desired_width(180.0),
        );
        ui.label(egui::RichText::new("t≤").weak().size(11.0));
        ui.add(egui::DragValue::new(&mut state.trace_t_max).speed(10));
        ui.label(egui::RichText::new("(0 = semua)").weak().size(10.0));
        ui.separator();
        let q = state.trace_filter.to_lowercase();
        let tmax = state.trace_t_max;
        let matches = |e: &TraceEvent| {
            (q.is_empty() || e.name.to_lowercase().contains(&q)) && (tmax == 0 || e.t <= tmax)
        };
        let count = events.iter().filter(|e| matches(e)).count();
        ui.label(
            egui::RichText::new(format!("{} event", count))
                .weak()
                .monospace()
                .size(11.0),
        );
    });
    ui.separator();

    let q = state.trace_filter.to_lowercase();
    let tmax = state.trace_t_max;
    let matches = |e: &TraceEvent| {
        (q.is_empty() || e.name.to_lowercase().contains(&q)) && (tmax == 0 || e.t <= tmax)
    };
    let filtered: Vec<&TraceEvent> = events.iter().filter(|e| matches(e)).collect();
    if filtered.is_empty() {
        ui.label(
            egui::RichText::new("Tidak ada event yang cocok dengan filter")
                .weak()
                .italics(),
        );
        return;
    }

    let mut to_open: Option<(std::path::PathBuf, Option<usize>)> = None;
    let mut miss: Option<String> = None;
    egui::ScrollArea::vertical()
        .id_salt("trace_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("trace_grid")
                .striped(true)
                .num_columns(4)
                .min_col_width(70.0)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("t").strong().size(11.0));
                    ui.label(egui::RichText::new("Signal").strong().size(11.0));
                    ui.label(egui::RichText::new("Value").strong().size(11.0));
                    ui.label(egui::RichText::new("W").strong().size(11.0));
                    ui.end_row();

                    let shown = filtered.len().min(MAX_ROWS);
                    for e in &filtered[..shown] {
                        let resp = ui
                            .add(egui::Link::new(
                                egui::RichText::new(e.t.to_string()).monospace().size(11.0),
                            ))
                            .on_hover_text("Klik: buka deklarasi sinyal di RTL");
                        if resp.clicked() {
                            match decl_location(state, &e.name) {
                                Some((path, line)) => to_open = Some((path, line)),
                                None => {
                                    miss = Some(format!(
                                        "⚠ Deklarasi '{}' tidak ditemukan (compile dulu?)",
                                        e.name
                                    ));
                                }
                            }
                        }
                        ui.label(egui::RichText::new(&e.name).monospace().size(11.0));
                        ui.label(egui::RichText::new(&e.value).monospace().size(11.0));
                        ui.label(egui::RichText::new(e.width.to_string()).weak().size(11.0));
                        ui.end_row();
                    }
                    if filtered.len() > MAX_ROWS {
                        ui.label(
                            egui::RichText::new(format!(
                                "… {} event lagi (maks {} ditampilkan)",
                                filtered.len() - MAX_ROWS,
                                MAX_ROWS
                            ))
                            .weak()
                            .italics(),
                        );
                        ui.end_row();
                    }
                });
        });

    // Interaksi di-apply setelah ScrollArea (borrow `state` bersih).
    if let Some((path, line)) = to_open {
        if !state.open_files.iter().any(|of| of.path == path) {
            state.open_file(path.clone());
        }
        if let Some(idx) = state.open_files.iter().position(|of| of.path == path) {
            state.active_file = Some(idx);
            if let Some(l) = line {
                state.open_files[idx].pending_goto = Some(l);
            }
        }
    }
    if let Some(msg) = miss {
        state.log(msg);
    }
}
