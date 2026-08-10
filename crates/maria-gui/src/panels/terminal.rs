//! Terminal tab — jalankan perintah shell (mis. CLI maria) langsung dari GUI.
//!
//! Perintah dieksekusi di worker thread (`backend::spawn_term`); stdout/stderr
//! di-stream baris per baris dan ditampilkan dengan warna berbeda (stderr
//! merah). Dukungan: Enter jalankan, ↑/↓ navigasi riwayat, tombol ✕ hapus.

use eframe::egui;

use super::super::backend::spawn_term;
use super::super::state::{GuiState, TermLine, MAX_TERM_LINES};

/// Maksimal entri riwayat perintah.
const MAX_HISTORY: usize = 100;

/// Jalankan perintah: catat ke riwayat (dedupe, ujung = terbaru), tampilkan
/// `$ cmd`, lalu spawn worker thread. Dipakai oleh Enter dan tombol ▶ — satu
/// jalur saja agar tidak drift.
fn run_cmd(state: &mut GuiState, cmd: String) {
    state.term_history.retain(|h| *h != cmd);
    state.term_history.push(cmd.clone());
    if state.term_history.len() > MAX_HISTORY {
        let overflow = state.term_history.len() - MAX_HISTORY;
        state.term_history.drain(0..overflow);
    }
    state.term_hist_idx = usize::MAX;
    state.term_lines.push(TermLine {
        text: format!("$ {}", cmd),
        is_err: false,
    });
    let cwd = state.project_root.clone();
    let tx = state.tx.clone();
    state.term_running = true;
    state.term_input.clear();
    spawn_term(tx, cmd, cwd);
}

pub fn show(ui: &mut egui::Ui, state: &mut GuiState) {
    let height = ui.available_height().max(80.0);

    // ── Output (auto-scroll ke bawah, stderr merah) ──
    egui::ScrollArea::vertical()
        .id_salt("term_scroll")
        .auto_shrink([false, false])
        .max_height(height - 30.0)
        .stick_to_bottom(true)
        .show(ui, |ui| {
            if state.term_lines.is_empty() {
                ui.label(
                    egui::RichText::new(
                        "Terminal — ketik perintah lalu Enter (mis. maria test/counter.sv)",
                    )
                    .weak()
                    .italics(),
                );
            }
            for line in &state.term_lines {
                if line.is_err {
                    ui.label(
                        egui::RichText::new(&line.text)
                            .monospace()
                            .size(11.0)
                            .color(egui::Color32::from_rgb(239, 68, 68)),
                    );
                } else {
                    ui.label(egui::RichText::new(&line.text).monospace().size(11.0));
                }
            }
        });

    // ── Input baris + tombol ──
    ui.horizontal(|ui| {
        let running = state.term_running;
        let resp = ui.add(
            egui::TextEdit::singleline(&mut state.term_input)
                .id_source("term_input")
                .hint_text(if running { "▶ proses berjalan…" } else { "$ perintah" })
                .desired_width((ui.available_width() - 70.0).max(50.0)),
        );

        // Enter → jalankan; ↑/↓ → riwayat (hanya jika input fokus)
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let up = ui.input(|i| i.key_pressed(egui::Key::ArrowUp));
        let down = ui.input(|i| i.key_pressed(egui::Key::ArrowDown));

        if resp.has_focus() {
            if enter {
                let cmd = state.term_input.trim().to_string();
                if !cmd.is_empty() {
                    run_cmd(state, cmd);
                }
            } else if up {
                if !state.term_history.is_empty() {
                    let idx = if state.term_hist_idx == usize::MAX {
                        state.term_history.len() - 1
                    } else {
                        state.term_hist_idx.saturating_sub(1)
                    };
                    state.term_hist_idx = idx;
                    state.term_input = state.term_history[idx].clone();
                }
            } else if down {
                if state.term_hist_idx == usize::MAX {
                    // sudah di ujung
                } else if state.term_hist_idx + 1 >= state.term_history.len() {
                    state.term_hist_idx = usize::MAX;
                    state.term_input.clear();
                } else {
                    state.term_hist_idx += 1;
                    state.term_input = state.term_history[state.term_hist_idx].clone();
                }
            }
        }

        if ui.button("▶").on_hover_text("Jalankan perintah").clicked() {
            let cmd = state.term_input.trim().to_string();
            if !cmd.is_empty() {
                run_cmd(state, cmd);
            }
        }
        if ui.button("✕").on_hover_text("Bersihkan output").clicked() {
            state.term_lines.clear();
            state.term_running = false;
        }
        if running {
            ui.label(egui::RichText::new("●").color(egui::Color32::from_rgb(34, 197, 94)));
        }
    });

    // Batasi panjang buffer
    if state.term_lines.len() > MAX_TERM_LINES {
        let overflow = state.term_lines.len() - MAX_TERM_LINES;
        state.term_lines.drain(0..overflow);
    }
}
