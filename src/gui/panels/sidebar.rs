//! Sidebar kiri: tab Project (file tree), Symbols, Architecture, dan Search.

use eframe::egui;

use super::architecture;
use super::dependency;
use super::search;
use super::super::state::{GuiState, SidebarTab};

pub fn show(ui: &mut egui::Ui, state: &mut GuiState) {
    // ── Tab selector ──
    ui.horizontal(|ui| {
        if ui
            .selectable_label(state.sidebar_tab == SidebarTab::Project, "Project")
            .clicked()
        {
            state.sidebar_tab = SidebarTab::Project;
        }
        if ui
            .selectable_label(state.sidebar_tab == SidebarTab::Symbols, "Symbols")
            .clicked()
        {
            state.sidebar_tab = SidebarTab::Symbols;
        }
        if ui
            .selectable_label(state.sidebar_tab == SidebarTab::Architecture, "Architecture")
            .clicked()
        {
            state.sidebar_tab = SidebarTab::Architecture;
        }
        if ui
            .selectable_label(state.sidebar_tab == SidebarTab::Dependency, "Dependency")
            .clicked()
        {
            state.sidebar_tab = SidebarTab::Dependency;
        }
        if ui
            .selectable_label(state.sidebar_tab == SidebarTab::Search, "Search")
            .clicked()
        {
            state.sidebar_tab = SidebarTab::Search;
        }
    });
    ui.separator();

    match state.sidebar_tab {
        SidebarTab::Project => project_tab(ui, state),
        SidebarTab::Symbols => symbols_tab(ui, state),
        SidebarTab::Architecture => architecture::show(ui, state),
        SidebarTab::Dependency => dependency::show(ui, state),
        SidebarTab::Search => search::show(ui, state),
    }
}

// ── Project: file tree ──
fn project_tab(ui: &mut egui::Ui, state: &mut GuiState) {
    if state.project_name.is_empty() {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Buka proyek untuk melihat file")
                .weak()
                .italics(),
        );
        ui.add_space(4.0);
        if ui.button("📂 Open Project").clicked() {
            super::super::app::trigger_open_project(state);
        }
        return;
    }

    let mut to_open = None;
    egui::ScrollArea::vertical()
        .id_salt("file_tree_scroll")
        .show(ui, |ui| {
            tree_nodes(ui, &state.files, 0, &mut to_open);
        });

    if let Some(path) = to_open {
        state.open_file(path);
    }
}

fn tree_nodes(ui: &mut egui::Ui, nodes: &[super::super::state::FileNode], depth: usize, to_open: &mut Option<std::path::PathBuf>) {
    for node in nodes {
        if node.is_dir {
            let id = egui::Id::new(&node.path).with("dir");
            egui::CollapsingHeader::new(
                egui::RichText::new(format!("📁 {}", node.name)).weak(),
            )
            .id_salt(id)
            .default_open(depth < 1)
            .show(ui, |ui| {
                tree_nodes(ui, &node.children, depth + 1, to_open);
            });
        } else {
            let text = egui::RichText::new(format!("📄 {}", node.name));
            if ui
                .selectable_label(false, text)
                .on_hover_text(node.path.display().to_string())
                .clicked()
            {
                *to_open = Some(node.path.clone());
            }
        }
    }
}

// ── Symbols: modules/packages/interfaces dari hasil compile ──
fn symbols_tab(ui: &mut egui::Ui, state: &mut GuiState) {
    let Some(info) = &state.compile_info else {
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Compile dulu untuk melihat symbols").weak().italics());
        return;
    };

    egui::ScrollArea::vertical().id_salt("symbols_scroll").show(ui, |ui| {
        if !info.modules.is_empty() {
            ui.label(egui::RichText::new("Modules").strong().size(11.0));
            for m in &info.modules {
                ui.label(egui::RichText::new(format!("▸ {}", m)).monospace().size(12.0));
            }
            ui.add_space(6.0);
        }
        if !info.packages.is_empty() {
            ui.label(egui::RichText::new("Packages").strong().size(11.0));
            for m in &info.packages {
                ui.label(egui::RichText::new(format!("◈ {}", m)).monospace().size(12.0));
            }
            ui.add_space(6.0);
        }
        if !info.interfaces.is_empty() {
            ui.label(egui::RichText::new("Interfaces").strong().size(11.0));
            for m in &info.interfaces {
                ui.label(egui::RichText::new(format!("◇ {}", m)).monospace().size(12.0));
            }
        }
    });
}
