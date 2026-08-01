//! Command Palette (Ctrl+Shift+P) — akses cepat semua aksi utama.
//!
//! Window modal di tengah-atas dengan filter teks, navigasi keyboard
//! (↑/↓, Enter jalankan, Esc tutup), dan klik mouse.

use eframe::egui;

use super::super::app;
use super::super::state::{BottomTab, GuiState, SidebarTab};

#[derive(Debug, Clone)]
pub enum PaletteAction {
    OpenProject,
    Compile,
    Run,
    Stop,
    Save,
    ToggleSidebar,
    ToggleBottom,
    ShowProblems,
    ShowConsole,
    ShowSignals,
    ShowWaveform,
    ShowBenchmark,
    ShowCoverage,
    ShowTerminal,
    ShowArchitecture,
    ShowDependency,
    ToggleOutline,
    ShowSearch,
    ClearConsole,
}

const ACTIONS: &[(&str, &str, PaletteAction)] = &[
    ("Compile + Elaborate", "F7", PaletteAction::Compile),
    ("Run Simulation", "F5", PaletteAction::Run),
    ("Stop Simulation", "F5", PaletteAction::Stop),
    ("Open Project…", "Ctrl+O", PaletteAction::OpenProject),
    ("Save File", "Ctrl+S", PaletteAction::Save),
    ("Toggle Sidebar", "Ctrl+B", PaletteAction::ToggleSidebar),
    ("Toggle Bottom Panel", "Ctrl+`", PaletteAction::ToggleBottom),
    ("Open Problems", "", PaletteAction::ShowProblems),
    ("Open Console", "", PaletteAction::ShowConsole),
    ("Open Signals", "", PaletteAction::ShowSignals),
    ("Open Waveform", "", PaletteAction::ShowWaveform),
    ("Open Benchmark", "", PaletteAction::ShowBenchmark),
    ("Open Coverage", "", PaletteAction::ShowCoverage),
    ("Open Terminal", "", PaletteAction::ShowTerminal),
    ("Open Architecture", "", PaletteAction::ShowArchitecture),
    ("Open Dependency", "", PaletteAction::ShowDependency),
    ("Toggle Outline", "Ctrl+Shift+O", PaletteAction::ToggleOutline),
    ("Open Search", "", PaletteAction::ShowSearch),
    ("Clear Console", "", PaletteAction::ClearConsole),
];

/// Render palette. Membaca/menulis `state.palette_*`; menutup diri saat
/// Eksekusi atau Esc. Dipanggil dari `App::ui` bila `palette_open`.
pub fn show(ui: &mut egui::Ui, state: &mut GuiState) {
    let ctx = ui.ctx().clone();
    let filter_id = egui::Id::new("palette_filter_input");

    let mut close = false;
    let mut exec: Option<PaletteAction> = None;

    egui::Window::new("Command Palette")
        .id(egui::Id::new("cmd_palette"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_TOP, [0.0, 96.0])
        .default_width(580.0)
        .show(&ctx, |ui| {
            // Fokus ke filter hanya pada frame pertama pembukaan
            if state.palette_just_opened {
                ctx.memory_mut(|m| m.request_focus(filter_id));
                state.palette_just_opened = false;
            }
            ui.add(
                egui::TextEdit::singleline(&mut state.palette_filter)
                    .id_source(filter_id)
                    .hint_text("Ketik perintah… (↑/↓ pilih, Enter jalankan, Esc tutup)")
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(4.0);

            // ── Filter ──
            let q = state.palette_filter.to_lowercase();
            let mut filtered: Vec<usize> = Vec::new();
            for (i, (name, _, _)) in ACTIONS.iter().enumerate() {
                if q.is_empty() || name.to_lowercase().contains(&q) {
                    filtered.push(i);
                }
            }
            if filtered.is_empty() {
                ui.label(egui::RichText::new("Tidak ada perintah yang cocok").weak().italics());
                return;
            }
            if state.palette_selected >= filtered.len() {
                state.palette_selected = 0;
            }

            // ── Navigasi keyboard ──
            let up = ui.input(|i| i.key_pressed(egui::Key::ArrowUp));
            let down = ui.input(|i| i.key_pressed(egui::Key::ArrowDown));
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if up && !filtered.is_empty() {
                state.palette_selected = state.palette_selected.saturating_sub(1);
            }
            if down && !filtered.is_empty() {
                state.palette_selected = (state.palette_selected + 1).min(filtered.len() - 1);
            }
            if enter {
                if let Some(&idx) = filtered.get(state.palette_selected) {
                    exec = Some(ACTIONS[idx].2.clone());
                }
            }
            if esc {
                close = true;
                return;
            }

            // ── Daftar ──
            let sel_fill = ui.visuals().selection.bg_fill;
            egui::ScrollArea::vertical()
                .id_salt("palette_list")
                .max_height(380.0)
                .show(ui, |ui| {
                    for (row, &idx) in filtered.iter().enumerate() {
                        let (name, key, action) = &ACTIONS[idx];
                        let selected = row == state.palette_selected;
                        let text = egui::RichText::new(*name).monospace();
                        let row_ui = |ui: &mut egui::Ui| {
                            ui.horizontal(|ui| {
                                ui.label(text.clone());
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if !key.is_empty() {
                                            ui.label(
                                                egui::RichText::new(*key)
                                                    .weak()
                                                    .monospace()
                                                    .size(10.0),
                                            );
                                        }
                                    },
                                );
                            })
                        };
                        let resp = if selected {
                            egui::Frame::NONE.fill(sel_fill).show(ui, row_ui).inner
                        } else {
                            row_ui(ui)
                        };
                        if resp.response.clicked() {
                            exec = Some(action.clone());
                        }
                    }
                });
        });

    if let Some(action) = exec {
        execute(state, action);
        close = true;
    }
    if close {
        state.palette_open = false;
    }
}

fn execute(state: &mut GuiState, action: PaletteAction) {
    match action {
        PaletteAction::OpenProject => app::trigger_open_project(state),
        PaletteAction::Compile => app::trigger_compile(state),
        PaletteAction::Run => app::trigger_run(state),
        PaletteAction::Stop => app::trigger_stop(state),
        PaletteAction::Save => {
            if state.save_active_file() {
                state.log("💾 File disimpan");
            }
        }
        PaletteAction::ToggleSidebar => state.show_sidebar = !state.show_sidebar,
        PaletteAction::ToggleBottom => state.show_bottom = !state.show_bottom,
        PaletteAction::ShowProblems => {
            state.show_bottom = true;
            state.bottom_tab = BottomTab::Problems;
        }
        PaletteAction::ShowConsole => {
            state.show_bottom = true;
            state.bottom_tab = BottomTab::Console;
        }
        PaletteAction::ShowSignals => {
            state.show_bottom = true;
            state.bottom_tab = BottomTab::Signals;
        }
        PaletteAction::ShowWaveform => {
            state.show_bottom = true;
            state.bottom_tab = BottomTab::Waveform;
        }
        PaletteAction::ShowBenchmark => {
            state.show_bottom = true;
            state.bottom_tab = BottomTab::Benchmark;
        }
        PaletteAction::ShowCoverage => {
            state.show_bottom = true;
            state.bottom_tab = BottomTab::Coverage;
        }
        PaletteAction::ShowTerminal => {
            state.show_bottom = true;
            state.bottom_tab = BottomTab::Terminal;
        }
        PaletteAction::ShowArchitecture => {
            state.show_sidebar = true;
            state.sidebar_tab = SidebarTab::Architecture;
        }
        PaletteAction::ShowDependency => {
            state.show_sidebar = true;
            state.sidebar_tab = SidebarTab::Dependency;
        }
        PaletteAction::ToggleOutline => state.show_outline = !state.show_outline,
        PaletteAction::ShowSearch => {
            state.show_sidebar = true;
            state.sidebar_tab = SidebarTab::Search;
        }
        PaletteAction::ClearConsole => state.console.clear(),
    }
}
