//! Architecture viewer — pohon hierarki instansiasi (CPU → Cache → AXI → …).
//!
//! Dibangun dari `IrDesign`: root = modul top, lalu `sub_instances` di-resolve
//! rekursif lewat `design.modules`. Klik nama instance → buka file RTL module
//! (via `CompileInfo.module_files` dari module_index); panah ▾/▸ untuk
//! expand/collapse. Guard siklus mencegah loop tak hingga untuk modul rekursif.

use eframe::egui;

use maria_ir::IrDesign;
use maria_core::Symbol;

use super::super::state::GuiState;

/// Satu node pohon arsitektur.
#[derive(Debug, Clone)]
pub struct ArchNode {
    pub instance_name: String,
    pub module_name: String,
    pub line: usize,
    pub children: Vec<ArchNode>,
}

/// Bangun pohon hierarki dari design ter-elaborasi.
pub fn build_tree(design: &IrDesign) -> ArchNode {
    let top_name = design.top.name.to_string();
    let mut root = ArchNode {
        instance_name: top_name.clone(),
        module_name: top_name.clone(),
        line: 0,
        children: Vec::new(),
    };
    // ancestors: jumlah kemunculan modul pada path saat ini (guard siklus).
    let mut ancestors: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    ancestors.insert(top_name.clone(), 1);
    build_children(design, &design.top, &mut root, &mut ancestors, 0);
    root
}

fn build_children(
    design: &IrDesign,
    module: &maria_ir::IrModule,
    parent: &mut ArchNode,
    ancestors: &mut std::collections::HashMap<String, usize>,
    depth: usize,
) {
    if depth >= 64 {
        return; // cap kedalaman — desain monster sekalipun tetap aman
    }
    for inst in &module.sub_instances {
        let child_mod = inst.module_name.to_string();
        let mut node = ArchNode {
            instance_name: inst.instance_name.to_string(),
            module_name: child_mod.clone(),
            line: inst.line,
            children: Vec::new(),
        };
        // Hitung count dalam scope pendek agar borrow `ancestors` dilepas
        // sebelum rekursi (E0499) — decrement dilakukan via get_mut terpisah.
        let cnt_val = {
            let cnt = ancestors.entry(child_mod.clone()).or_insert(0);
            *cnt += 1;
            *cnt
        };
        if cnt_val < 2 {
            if let Some(cm) = design.modules.get(&Symbol::intern(&child_mod)) {
                build_children(design, cm, &mut node, ancestors, depth + 1);
            }
        }
        if let Some(e) = ancestors.get_mut(&child_mod) {
            *e = e.saturating_sub(1);
        }
        parent.children.push(node);
    }
}

/// Render tab Architecture di sidebar.
pub fn show(ui: &mut egui::Ui, state: &mut GuiState) {
    let Some(design) = &state.design else {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Compile dulu untuk melihat arsitektur")
                .weak()
                .italics(),
        );
        return;
    };
    let tree = build_tree(design);
    let module_files = state
        .compile_info
        .as_ref()
        .map(|i| i.module_files.clone())
        .unwrap_or_default();

    let mut to_open: Option<std::path::PathBuf> = None;
    egui::ScrollArea::vertical()
        .id_salt("arch_scroll")
        .show(ui, |ui| {
            render_node(ui, &tree, "", 0, state, &module_files, &mut to_open);
        });

    if let Some(path) = to_open {
        state.open_file(path);
    }
}

fn render_node(
    ui: &mut egui::Ui,
    node: &ArchNode,
    parent_key: &str,
    depth: usize,
    state: &mut GuiState,
    module_files: &std::collections::HashMap<String, std::path::PathBuf>,
    to_open: &mut Option<std::path::PathBuf>,
) {
    let has_children = !node.children.is_empty();
    let key = format!("{}::{}", parent_key, node.instance_name);
    let is_open = state.arch_open.get(&key).copied().unwrap_or(depth < 2);

    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 16.0);

        // Panah expand/collapse (atau titik untuk leaf)
        if has_children {
            let arrow = if is_open { "▾" } else { "▸" };
            if ui
                .button(egui::RichText::new(arrow).size(10.0))
                .clicked()
            {
                state.arch_open.insert(key.clone(), !is_open);
            }
        } else {
            ui.label(egui::RichText::new("·").weak().size(10.0));
        }

        // Nama instance — klik membuka file RTL module (aksi utama per desain)
        let label = format!("{}  ({})", node.instance_name, node.module_name);
        if ui
            .selectable_label(false, egui::RichText::new(label).monospace().size(12.0))
            .on_hover_text(format!(
                "{} · line {}\nKlik untuk membuka file",
                node.module_name, node.line
            ))
            .clicked()
        {
            if let Some(file) = module_files.get(&node.module_name) {
                *to_open = Some(file.clone());
            }
        }
    });

    if is_open {
        for c in &node.children {
            render_node(ui, c, &key, depth + 1, state, module_files, to_open);
        }
    }
}
