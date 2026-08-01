//! Editor tengah: tab bar file terbuka + CodeEditor (egui_code_editor)
//! dengan syntax highlighting SystemVerilog.

use eframe::egui;
use egui_code_editor::{CodeEditor, ColorTheme};

use super::super::sv_syntax::systemverilog_syntax;

pub fn show(ui: &mut egui::Ui, state: &mut super::super::state::GuiState) {
    // ── Welcome screen ──
    if state.open_files.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("Maria")
                        .size(42.0)
                        .strong()
                        .color(ui.visuals().selection.bg_fill),
                );
                ui.label(egui::RichText::new("RTL Engineering Control Center").size(14.0).weak());
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("Open project → compile → run simulation")
                        .weak()
                        .italics(),
                );
                ui.add_space(12.0);
                if ui.button("📂 Open Project").clicked() {
                    super::super::app::trigger_open_project(state);
                }
            });
        });
        return;
    }

    // ── Tab bar ──
    ui.horizontal(|ui| {
        let mut to_close: Option<usize> = None;
        for (i, f) in state.open_files.iter().enumerate() {
            let active = state.active_file == Some(i);
            let label = if f.dirty {
                format!("● {}", f.name)
            } else {
                f.name.clone()
            };
            let text = egui::RichText::new(label).monospace().size(12.0);
            if ui.selectable_label(active, text).clicked() {
                state.active_file = Some(i);
            }
            // tombol close kecil
            let resp = ui.add(
                egui::Button::new(egui::RichText::new("✕").size(10.0))
                    .frame(false)
                    .small(),
            );
            if resp.clicked() {
                to_close = Some(i);
            }
        }
        if let Some(i) = to_close {
            state.close_file(i);
        }
    });
    ui.separator();

    // ── Editor aktif ──
    let Some(idx) = state.active_file else {
        return;
    };
    let Some(f) = state.open_files.get_mut(idx) else {
        return;
    };

    // Breadcrumb sederhana
    ui.horizontal(|ui| {
        let path_str = f.path.display().to_string();
        ui.label(
            egui::RichText::new(&path_str)
                .weak()
                .monospace()
                .size(10.0),
        );
    });
    ui.add_space(2.0);

    let syntax = systemverilog_syntax();
    let id = format!("sv_editor:{}", f.path.display());
    let mut editor = CodeEditor::default()
        .id_source(id)
        .with_rows(40)
        .with_fontsize(13.0)
        .with_theme(ColorTheme::GITHUB_DARK)
        .with_numlines(true);

    // Deteksi perubahan → mark dirty
    let before = f.content.clone();
    editor.show(ui, &mut f.content, &syntax);
    if f.content != before {
        f.dirty = true;
    }
}
