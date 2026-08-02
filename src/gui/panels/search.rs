//! Sidebar Search — cari simbol design sesuai desain Maria: "bukan sekadar
//! grep, tetapi Find module / signal / parameter / package / macro / instance".
//!
//! Data di-precompute saat compile (di backend.rs) — index parameter, macro,
//! dan instance tersimpan di `CompileInfo`, jadi pencarian per frame tidak
//! perlu meng-iterasi design atau scan ulang source. Klik hasil → buka file
//! (module/package/interface/parameter) atau lompat ke baris (macro/instance).

use eframe::egui;
use std::path::PathBuf;

use super::super::state::{CompileInfo, GuiState, SearchCat};

pub fn show(ui: &mut egui::Ui, state: &mut GuiState) {
    if state.compile_info.is_none() && state.design.is_none() {
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Compile dulu untuk mencari").weak().italics());
        return;
    }

    // ── Kategori ──
    ui.horizontal_wrapped(|ui| {
        let mut clicked: Option<SearchCat> = None;
        for (cat, label) in [
            (SearchCat::Module, "Module"),
            (SearchCat::Signal, "Signal"),
            (SearchCat::Parameter, "Parameter"),
            (SearchCat::Package, "Package"),
            (SearchCat::Macro, "Macro"),
            (SearchCat::Instance, "Instance"),
        ] {
            if ui.selectable_label(state.search_cat == cat, label).clicked() {
                clicked = Some(cat);
            }
        }
        if let Some(c) = clicked {
            state.search_cat = c;
            state.search_filter.clear();
        }
    });
    ui.add(
        egui::TextEdit::singleline(&mut state.search_filter)
            .hint_text("Cari…")
            .desired_width(f32::INFINITY),
    );
    ui.separator();

    let q = state.search_filter.to_lowercase();
    let info = state.compile_info.as_ref();
    let design = state.design.as_ref();
    let mut to_open: Option<(PathBuf, Option<usize>)> = None;
    let mut found_any = false;

    egui::ScrollArea::vertical()
        .id_salt("search_scroll")
        .show(ui, |ui| match state.search_cat {
            SearchCat::Module => modules_ui(ui, info, &q, &mut to_open, &mut found_any),
            SearchCat::Signal => signals_ui(ui, info, design, &q, &mut to_open, &mut found_any),
            SearchCat::Parameter => parameters_ui(ui, info, &q, &mut to_open, &mut found_any),
            SearchCat::Package => packages_ui(ui, info, &q, &mut to_open, &mut found_any),
            SearchCat::Macro => macros_ui(ui, info, &q, &mut to_open, &mut found_any),
            SearchCat::Instance => instances_ui(ui, info, &q, &mut to_open, &mut found_any),
        });

    if !q.is_empty() && !found_any {
        ui.label(egui::RichText::new("Tidak ada hasil").weak().italics());
    }

    if let Some((path, line)) = to_open {
        if state.open_files.iter().all(|of| of.path != path) {
            state.open_file(path.clone());
        }
        if let Some(idx) = state.open_files.iter().position(|of| of.path == path) {
            state.active_file = Some(idx);
            if let Some(l) = line {
                state.open_files[idx].pending_goto = Some(l);
            }
        }
    }
}

/// Satu baris hasil klik → buka file (opsional lompat baris). Mengembalikan
/// true jika baris diklik.
fn result_row(
    ui: &mut egui::Ui,
    primary: &str,
    secondary: &str,
    file: Option<&PathBuf>,
) -> bool {
    let resp = ui
        .horizontal(|ui| {
            ui.label(egui::RichText::new("·").weak().size(10.0));
            ui.label(egui::RichText::new(primary).monospace().size(12.0));
            if !secondary.is_empty() {
                ui.label(egui::RichText::new(secondary).weak().size(10.0));
            }
        })
        .response
        .on_hover_text(file.map(|f| f.display().to_string()).unwrap_or_default());
    resp.clicked()
}

// ───────────────────────────── Kategori ─────────────────────────────

fn modules_ui(
    ui: &mut egui::Ui,
    info: Option<&CompileInfo>,
    q: &str,
    to_open: &mut Option<(PathBuf, Option<usize>)>,
    found_any: &mut bool,
) {
    let Some(info) = info else { return };
    let modules: Vec<&String> = info
        .modules
        .iter()
        .filter(|m| q.is_empty() || m.to_lowercase().contains(q))
        .collect();
    if modules.is_empty() {
        return;
    }
    *found_any = true;
    ui.label(egui::RichText::new(format!("Modules ({})", modules.len())).strong().size(11.0));
    for m in modules {
        if result_row(ui, m, "", info.module_files.get(m)) {
            if let Some(file) = info.module_files.get(m) {
                *to_open = Some((file.clone(), None));
            }
        }
    }
}

fn signals_ui(
    ui: &mut egui::Ui,
    info: Option<&CompileInfo>,
    design: Option<&crate::ir::IrDesign>,
    q: &str,
    to_open: &mut Option<(PathBuf, Option<usize>)>,
    found_any: &mut bool,
) {
    let Some(design) = design else { return };
    // Signal dari SEMUA module (top + submodule) — nama di-prefix module agar
    // bisa membedakan signal yang sama di module berbeda (hirarki penuh).
    // `owner` = module pemilik untuk resolve file (top module pun ikut — klik
    // signal top membuka file module top, bukan no-op).
    let mut matched: Vec<(String, usize, String, Option<PathBuf>)> = Vec::new();
    {
        // Semua signal di-prefix nama module pemilik (top module ikut) —
        // display konsisten seluruh hirarki & file ter-resolve via module_files.
        let mut add_mod = |owner: &str, m: &crate::ir::IrModule| {
            for s in &m.signals {
                let full = format!("{}.{}", owner, s.name);
                if q.is_empty() || full.to_lowercase().contains(q) {
                    matched.push((full, s.width, owner.to_string(), None));
                }
            }
        };
        add_mod(design.top.name.as_str(), &design.top);
        for m in design.modules.values() {
            if m.name != design.top.name {
                add_mod(m.name.as_str(), m);
            }
        }
    }
    // Resolve file per signal: owner (module name) → module_files.
    if let Some(info) = info {
        for (_, _, owner, file) in matched.iter_mut() {
            if let Some(f) = info.module_files.get(owner) {
                *file = Some(f.clone());
            }
        }
    }
    matched.sort_by(|a, b| a.0.cmp(&b.0));
    if matched.is_empty() {
        return;
    }
    *found_any = true;
    ui.label(egui::RichText::new(format!("Signals ({})", matched.len())).strong().size(11.0));
    for (full, width, _owner, file) in matched.into_iter().take(500) {
        if result_row(ui, &full, &format!("[{}]", width), file.as_ref()) {
            if let Some(f) = file {
                *to_open = Some((f.clone(), None));
            }
        }
    }
}

fn parameters_ui(
    ui: &mut egui::Ui,
    info: Option<&CompileInfo>,
    q: &str,
    to_open: &mut Option<(PathBuf, Option<usize>)>,
    found_any: &mut bool,
) {
    let Some(info) = info else { return };
    let matched: Vec<&crate::gui::state::ParamRow> = info
        .param_index
        .iter()
        .filter(|p| {
            q.is_empty()
                || p.name.to_lowercase().contains(q)
                || p.module.to_lowercase().contains(q)
        })
        .take(500)
        .collect();
    if matched.is_empty() {
        return;
    }
    *found_any = true;
    ui.label(egui::RichText::new(format!("Parameters ({})", matched.len())).strong().size(11.0));
    for p in matched {
        if result_row(ui, &p.name, &format!("@ {}", p.module), Some(&p.file)) {
            *to_open = Some((p.file.clone(), None));
        }
    }
}

fn packages_ui(
    ui: &mut egui::Ui,
    info: Option<&CompileInfo>,
    q: &str,
    to_open: &mut Option<(PathBuf, Option<usize>)>,
    found_any: &mut bool,
) {
    let Some(info) = info else { return };
    let matched: Vec<&String> = info
        .packages
        .iter()
        .filter(|m| q.is_empty() || m.to_lowercase().contains(q))
        .collect();
    if matched.is_empty() {
        return;
    }
    *found_any = true;
    ui.label(egui::RichText::new(format!("Packages ({})", matched.len())).strong().size(11.0));
    for p in matched {
        let file = info.symbol_files.get(p);
        if result_row(ui, p, "", file) {
            if let Some(f) = file {
                *to_open = Some((f.clone(), None));
            }
        }
    }
}

fn macros_ui(
    ui: &mut egui::Ui,
    info: Option<&CompileInfo>,
    q: &str,
    to_open: &mut Option<(PathBuf, Option<usize>)>,
    found_any: &mut bool,
) {
    let Some(info) = info else { return };
    let matched: Vec<&crate::gui::state::MacroRow> = info
        .macro_index
        .iter()
        .filter(|m| q.is_empty() || m.name.to_lowercase().contains(q))
        .take(500)
        .collect();
    if matched.is_empty() {
        return;
    }
    *found_any = true;
    ui.label(egui::RichText::new(format!("Macros ({})", matched.len())).strong().size(11.0));
    for m in matched {
        let sec = format!("L{}", m.line);
        if result_row(ui, &format!("`{}", m.name), &sec, Some(&m.file)) {
            *to_open = Some((m.file.clone(), Some(m.line)));
        }
    }
}

fn instances_ui(
    ui: &mut egui::Ui,
    info: Option<&CompileInfo>,
    q: &str,
    to_open: &mut Option<(PathBuf, Option<usize>)>,
    found_any: &mut bool,
) {
    let Some(info) = info else { return };
    let matched: Vec<&crate::gui::state::InstanceRow> = info
        .instance_index
        .iter()
        .filter(|i| {
            q.is_empty()
                || i.name.to_lowercase().contains(q)
                || i.module.to_lowercase().contains(q)
        })
        .take(500)
        .collect();
    if matched.is_empty() {
        return;
    }
    *found_any = true;
    ui.label(egui::RichText::new(format!("Instances ({})", matched.len())).strong().size(11.0));
    for i in matched {
        let has_file = !i.file.as_os_str().is_empty();
        let sec = if has_file {
            format!("{} · L{}", i.module, i.line)
        } else {
            i.module.clone()
        };
        let file = if has_file { Some(&i.file) } else { None };
        if result_row(ui, &i.name, &sec, file) {
            if has_file {
                *to_open = Some((i.file.clone(), Some(i.line)));
            }
        }
    }
}
