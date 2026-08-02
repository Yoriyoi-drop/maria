//! `MariaApp` — implementasi `eframe::App`.
//!
//! Layout: toolbar (atas) → sidebar (kiri) → editor (tengah) → bottom panel
//! (bawah) → status bar (paling bawah). Semua operasi berat (compile/sim)
//! dijalankan di worker thread; hasil dipoll per-frame lewat channel.
//!
//! Catatan API: eframe/egui 0.35 menggabungkan `TopBottomPanel`/`SidePanel`
//! menjadi satu struct `Panel` (`Panel::top/bottom/left`), semua `.show`
//! menerima `&mut Ui` (bukan `Context`), dan `App::update` diganti `App::ui`.

use eframe::egui;
use std::sync::mpsc::channel;
use std::time::Duration;

use std::sync::atomic::Ordering;

use super::backend::{scan_tree, spawn_compile, spawn_sim};
use super::panels::{bottom, command_palette, editor, outline, sidebar, statusbar, toolbar};
use super::state::{DiagEntry, DiagLevel, GuiEvent, GuiState, STAGE_SIMULATOR};
use super::workspace::{restore_workspace, save_workspace};

pub struct MariaApp {
    pub state: GuiState,
    /// Restore workspace terakhir sudah dieksekusi (sekali di frame pertama —
    /// scan_tree proyek bisa berat, jangan blokir pembuatan window).
    restored: bool,
}

impl MariaApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = channel::<GuiEvent>();
        setup_theme(&cc.egui_ctx);
        Self {
            state: GuiState::new(tx, rx),
            restored: false,
        }
    }

    /// Poll event dari worker thread dan terapkan ke state.
    fn poll_events(&mut self) {
        while let Ok(ev) = self.state.rx.try_recv() {
            match ev {
                GuiEvent::CompileDone(result) => match result {
                    Ok((info, design)) => {
                        // Lint warning (unused signal, blocking assignment di
                        // blok sequential) dari scan source — tampilkan di
                        // Problems tab dengan tombol Quick Fix (💡). Di-clone
                        // dulu karena `info` dipindah ke compile_info.
                        let lint = info.lint.clone();
                        self.state.design = Some(design);
                        self.state.compile_info = Some(info);
                        // Graf dependensi berubah → buang cache layout lama
                        // (di-rebuild dengan kunci baru saat tab dibuka).
                        self.state.dep_graph = None;
                        let m = self.state.compile_info.as_ref().map(|i| i.modules.len()).unwrap_or(0);
                        self.state.log(format!(
                            "✅ Compile + elaborate selesai ({} module, {:.2}ms)",
                            m,
                            self.state.compile_info.as_ref().map(|i| i.total_time_ms).unwrap_or(0.0)
                        ));
                        if !lint.is_empty() {
                            self.state.log(format!("🔍 Lint: {} warning(s)", lint.len()));
                            self.state.diagnostics.extend(lint);
                        }
                    }
                    Err(diags) => {
                        // Diagnostics sudah punya file/line (dari source snippet) —
                        // langsung masuk Problems tab + Mini Map editor.
                        for d in &diags {
                            let loc = if d.file.is_empty() {
                                String::new()
                            } else {
                                format!("{}:{}", d.file, d.line)
                            };
                            self.state.log(format!("❌ [{}] {}", loc, d.message));
                        }
                        self.state.diagnostics.extend(diags);
                    }
                },
                GuiEvent::TermOutput(text, is_err) => {
                    self.state.term_lines.push(crate::gui::state::TermLine {
                        text,
                        is_err,
                    });
                    if self.state.term_lines.len() > crate::gui::state::MAX_TERM_LINES {
                        let overflow =
                            self.state.term_lines.len() - crate::gui::state::MAX_TERM_LINES;
                        self.state.term_lines.drain(0..overflow);
                    }
                }
                GuiEvent::TermExit(code) => {
                    self.state.term_running = false;
                    let status = if code == 0 {
                        "✅ selesai".to_string()
                    } else {
                        format!("❌ exit code {}", code)
                    };
                    self.state.term_lines.push(crate::gui::state::TermLine {
                        text: status,
                        is_err: code != 0,
                    });
                }
                GuiEvent::SimDone(result) => {
                    self.state.is_running = false;
                    match result {
                        Ok(info) => {
                            // Pipeline: tandai tahap Simulator selesai (✓ +
                            // durasi) — panel Pipeline mencerminkan full
                            // lifecycle, bukan hanya compile. Cocokkan tahap
                            // berdasarkan NAMA (bukan last_mut) agar tidak
                            // rapuh bila urutan tahap pipeline diubah; nama
                            // memakai konstanta bersama STAGE_SIMULATOR.
                            if let Some(ci) = self.state.compile_info.as_mut() {
                                if let Some(stage) =
                                    ci.pipeline.iter_mut().find(|s| s.name == STAGE_SIMULATOR)
                                {
                                    stage.status = "ok".into();
                                    stage.ms = info.sim_time_ms as u64;
                                }
                            }
                            self.state.signals = info.signals;
                            self.state.cycles = info.cycles;
                            self.state.sim_time_ms = info.sim_time_ms;
                            self.state.delta_cycles = info.delta_cycles;
                            self.state.events_processed = info.events_processed;
                            self.state.processes_evaluated = info.processes_evaluated;
                            self.state.nba_commits = info.nba_commits;
                            self.state.sensitive_triggers = info.sensitive_triggers;
                            self.state.events_per_delta = info.events_per_delta;
                            self.state.coverage = info.coverage;
                            self.state.log(format!(
                                "✅ Simulasi selesai — t={} ({} signal, {:.2}ms)",
                                info.cycles,
                                self.state.signals.len(),
                                info.sim_time_ms
                            ));
                        }
                        Err(e) => {
                            // Stop oleh user BUKAN error — jangan masuk Problems tab.
                            if !e.starts_with("Simulasi dihentikan") {
                                self.state.diagnostics.push(DiagEntry {
                                    file: String::new(),
                                    line: 0,
                                    message: format!("Simulation error: {}", e),
                                    level: DiagLevel::Error,
                                    fix: None,
                                });
                                self.state.log(format!("❌ Simulasi gagal: {}", e));
                            } else {
                                self.state.log(format!("⏹ {}", e));
                            }
                        }
                    }
                }
            }
        }
    }

    /// Handle keyboard shortcuts.
    ///
    /// Ctrl+S dan Ctrl+O selalu aktif (termasuk saat editor fokus — tombol
    /// global seperti save/open dialog tidak membajak pengetikan). Shortcut
    /// lain (F5/F7/Ctrl+B/Ctrl+`) di-skip jika editor sedang menerima input
    /// keyboard (mis. mengetik di CodeEditor) agar tidak mengganggu editing.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::Key;
        let editor_typing = ctx.egui_wants_keyboard_input();
        ctx.input(|i| {
            let cmd = i.modifiers.command;

            // Global: selalu aktif
            if cmd && i.key_pressed(Key::S) {
                if self.state.save_active_file() {
                    self.state.log("💾 File disimpan");
                }
            }
            if cmd && i.key_pressed(Key::O) {
                trigger_open_project(&mut self.state);
            }
            if cmd && i.modifiers.shift && i.key_pressed(Key::P) {
                let opening = !self.state.palette_open;
                self.state.palette_open = opening;
                if opening {
                    self.state.palette_just_opened = true;
                    self.state.palette_filter.clear();
                    self.state.palette_selected = 0;
                }
            }
            if cmd && i.modifiers.shift && i.key_pressed(Key::O) {
                self.state.show_outline = !self.state.show_outline;
            }

            if editor_typing {
                return;
            }

            if i.key_pressed(Key::F5) {
                if self.state.is_running {
                    trigger_stop(&mut self.state);
                } else {
                    trigger_run(&mut self.state);
                }
            }
            if i.key_pressed(Key::F7) {
                trigger_compile(&mut self.state);
            }
            if cmd && i.key_pressed(Key::B) {
                self.state.show_sidebar = !self.state.show_sidebar;
            }
            if cmd && i.key_pressed(Key::Backtick) {
                self.state.show_bottom = !self.state.show_bottom;
            }
        });
    }
}

/// Trigger: buka folder proyek (dialog native via rfd).
pub fn trigger_open_project(state: &mut GuiState) {
    let Some(dir) = rfd::FileDialog::new().pick_folder() else {
        return;
    };
    let name = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Project".to_string());
    let files = scan_tree(&dir);
    state.project_name = name;
    state.project_root = Some(dir);
    state.files = files;
    state.compile_info = None;
    state.signals.clear();
    state.diagnostics.clear();
    state.log("📂 Proyek dibuka");
    // Proyek ini jadi "last workspace" — dipulihkan saat app dibuka kembali.
    save_workspace(state);
}

/// Trigger: compile + elaborate semua file .sv/.svh di pohon proyek.
pub fn trigger_compile(state: &mut GuiState) {
    if state.project_root.is_none() {
        return;
    }
    let paths = state.collect_sv_files();
    if paths.is_empty() {
        state.log("⚠ Tidak ada file .sv/.svh");
        return;
    }
    state.log(format!("🔨 Compile {} file...", paths.len()));
    state.diagnostics.clear();
    let tx = state.tx.clone();
    // Project root dipakai backend untuk mencari database MICD (`.maria/
    // database`) — tanpa root, compile berjalan non-incremental.
    let project_root = state.project_root.clone();
    spawn_compile(tx, paths, project_root);
}

/// Trigger: jalankan simulasi pada design ter-compile.
pub fn trigger_run(state: &mut GuiState) {
    if state.is_running {
        return;
    }
    let Some(design) = state.design.clone() else {
        state.log("⚠ Compile dulu sebelum run");
        return;
    };
    state.cancel_flag.store(false, Ordering::Relaxed);
    state.log(format!("▶ Simulasi (T={})...", state.max_time));
    state.is_running = true;
    let max_time = state.max_time;
    let tx = state.tx.clone();
    let cancel = state.cancel_flag.clone();
    spawn_sim(tx, design, max_time, cancel);
}

/// Trigger: hentikan simulasi yang sedang berjalan (Stop sungguhan — worker
/// thread memeriksa flag ini di run loop dan berhenti lebih awal).
pub fn trigger_stop(state: &mut GuiState) {
    if !state.is_running {
        return;
    }
    state.cancel_flag.store(true, Ordering::Relaxed);
    state.is_running = false;
    // Pesan final ("⏹ Simulasi dihentikan") dikirim worker via SimDone.
}

fn setup_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    // Palet: abu-abu gelap tenang, kontras lembut
    visuals.panel_fill = egui::Color32::from_rgb(26, 27, 30);
    visuals.window_fill = egui::Color32::from_rgb(24, 25, 28);
    visuals.extreme_bg_color = egui::Color32::from_rgb(18, 19, 22);
    visuals.faint_bg_color = egui::Color32::from_rgb(30, 31, 35);
    visuals.selection.bg_fill = egui::Color32::from_rgb(59, 130, 246);
    visuals.selection.stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(79, 193, 255));
    visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(30, 31, 35);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(42, 43, 48);
    visuals.widgets.active.bg_fill = egui::Color32::from_rgb(59, 130, 246);
    visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(26, 27, 30);
    ctx.set_visuals(visuals);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
    });
}

impl eframe::App for MariaApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // ── Restore workspace terakhir (sekali, di frame pertama) ──
        if !self.restored {
            self.restored = true;
            restore_workspace(&mut self.state);
        }
        // ── Simpan workspace saat window akan ditutup (frame terakhir) ──
        if ctx.input(|i| i.viewport().close_requested()) {
            save_workspace(&self.state);
        }

        self.handle_shortcuts(&ctx);
        self.poll_events();

        // ── Toolbar (atas) ──
        egui::Panel::top(egui::Id::new("toolbar"))
            .exact_size(36.0)
            .show(ui, |ui| {
                toolbar::show(ui, &mut self.state);
            });

        // ── Status bar (paling bawah) ──
        egui::Panel::bottom(egui::Id::new("statusbar"))
            .exact_size(24.0)
            .show(ui, |ui| {
                statusbar::show(ui, &mut self.state);
            });

        // ── Bottom panel ──
        if self.state.show_bottom {
            egui::Panel::bottom(egui::Id::new("bottom_panel"))
                .resizable(true)
                .default_size(220.0)
                .show(ui, |ui| {
                    bottom::show(ui, &mut self.state);
                });
        }

        // ── Sidebar (kiri) ──
        if self.state.show_sidebar {
            egui::Panel::left(egui::Id::new("sidebar"))
                .resizable(true)
                .default_size(260.0)
                .size_range(180.0..=420.0)
                .show(ui, |ui| {
                    sidebar::show(ui, &mut self.state);
                });
        }

        // ── Outline (kanan) ──
        if self.state.show_outline {
            egui::Panel::right(egui::Id::new("outline_panel"))
                .resizable(true)
                .default_size(230.0)
                .size_range(160.0..=400.0)
                .show(ui, |ui| {
                    outline::show(ui, &mut self.state);
                });
        }

        // ── Editor (tengah) ──
        egui::CentralPanel::default().show(ui, |ui| {
            editor::show(ui, &mut self.state);
        });

        // ── Command Palette (overlay, di atas semua panel) ──
        if self.state.palette_open {
            command_palette::show(ui, &mut self.state);
        }

        // Repaint terus jika worker sibuk (simulasi)
        if self.state.is_running {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        // Repaint berkala untuk resource monitor (CPU/RAM realtime di status bar)
        ctx.request_repaint_after(Duration::from_millis(1000));
    }
}
