//! Sidebar kiri: tab Project (file tree), Symbols, Architecture, dan Search.

use eframe::egui;

use super::super::state::{classify_file, FileCat, GuiState, SidebarTab};
use super::architecture;
use super::dependency;
use super::search;

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
            .selectable_label(
                state.sidebar_tab == SidebarTab::Architecture,
                "Architecture",
            )
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

// ── Project: file tree + bookmark ──
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

    let mut to_open: Option<std::path::PathBuf> = None;
    let mut to_toggle: Option<std::path::PathBuf> = None;

    // ── Toolbar kecil: filter kategori + bookmark ──
    ui.horizontal_wrapped(|ui| {
        let mut cat_click: Option<FileCat> = None;
        for (cat, label) in [
            (FileCat::All, "Semua"),
            (FileCat::Rtl, "RTL"),
            (FileCat::Testbench, "TB"),
            (FileCat::Library, "Lib"),
            (FileCat::Include, "Inc"),
        ] {
            if ui
                .selectable_label(state.explorer_cat == cat, label)
                .on_hover_text(match cat {
                    FileCat::All => "Semua file",
                    FileCat::Rtl => "RTL (default)",
                    FileCat::Testbench => "Testbench & UVM",
                    FileCat::Library => "Library / IP / cell",
                    FileCat::Include => "Header .svh",
                })
                .clicked()
            {
                cat_click = Some(cat);
            }
        }
        if let Some(c) = cat_click {
            state.explorer_cat = c;
        }
        // Pemisah visual + filter bookmark.
        ui.separator();
        let label = if state.bookmarks_only { "★" } else { "☆" };
        if ui
            .selectable_label(
                state.bookmarks_only,
                egui::RichText::new(format!("{} {}", label, state.bookmarks.len())),
            )
            .on_hover_text("Tampilkan hanya file yang di-bookmark")
            .clicked()
        {
            state.bookmarks_only = !state.bookmarks_only;
        }
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .id_salt("file_tree_scroll")
        .show(ui, |ui| {
            // ── Section Bookmarks (akses cepat) — tampil saat ada bookmark ──
            if !state.bookmarks.is_empty() {
                ui.label(egui::RichText::new("Bookmarks").strong().size(11.0));
                let mut bm: Vec<&std::path::PathBuf> = state.bookmarks.iter().collect();
                bm.sort();
                for p in bm {
                    let name = p
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| p.display().to_string());
                    let rel = state
                        .project_root
                        .as_ref()
                        .and_then(|r| p.strip_prefix(r).ok())
                        .map(|r| r.display().to_string())
                        .unwrap_or_else(|| p.display().to_string());
                    // `file_row` menangani klik bintang (toggle) & klik nama (buka).
                    let cat = classify_file(p);
                    file_row(ui, &name, &rel, p, true, cat, &mut to_open, &mut to_toggle);
                }
                ui.add_space(6.0);
            }

            // ── File tree (filter kategori + bookmark bila aktif) ──
            if !state.bookmarks_only {
                tree_nodes(
                    ui,
                    &state.files,
                    0,
                    &state.bookmarks,
                    state.bookmarks_only,
                    state.explorer_cat,
                    &mut to_open,
                    &mut to_toggle,
                );
            }
        });

    if let Some(path) = to_toggle {
        state.toggle_bookmark(&path);
    }
    if let Some(path) = to_open {
        state.open_file(path);
    }
}

/// Warna nama file per kategori (desain: sedikit warna, banyak informasi).
fn file_cat_color(cat: FileCat) -> egui::Color32 {
    match cat {
        FileCat::Rtl => egui::Color32::from_rgb(79, 193, 255), // biru — RTL
        FileCat::Testbench => egui::Color32::from_rgb(34, 197, 94), // hijau — TB
        FileCat::Library => egui::Color32::from_rgb(168, 85, 247), // ungu — lib/IP
        FileCat::Include => egui::Color32::from_rgb(6, 182, 212), // cyan — header
        FileCat::All => egui::Color32::GRAY,
    }
}

/// Satu baris file: [★/☆] nama — klik bintang toggle bookmark, klik baris buka.
/// Aksi diterapkan via `to_toggle`/`to_open` (diproses setelah ScrollArea).
/// `cat` dipakai mewarnai nama sesuai kategori file.
fn file_row(
    ui: &mut egui::Ui,
    name: &str,
    rel: &str,
    path: &std::path::PathBuf,
    is_bookmarked: bool,
    cat: FileCat,
    to_open: &mut Option<std::path::PathBuf>,
    to_toggle: &mut Option<std::path::PathBuf>,
) {
    let star = if is_bookmarked { "★" } else { "☆" };
    let star_color = if is_bookmarked {
        egui::Color32::from_rgb(234, 179, 8)
    } else {
        egui::Color32::from_rgb(110, 118, 129)
    };
    ui.horizontal(|ui| {
        // Ikon bintang — toggle bookmark.
        let s = ui.add(
            egui::Button::new(egui::RichText::new(star).size(11.0).color(star_color))
                .frame(false)
                .small(),
        );
        if s.on_hover_text("Bookmark file").clicked() {
            *to_toggle = Some(path.clone());
        }
        // Nama file — klik buka; diwarnai sesuai kategori.
        let text = egui::RichText::new(name)
            .monospace()
            .size(12.0)
            .color(file_cat_color(cat));
        if ui
            .selectable_label(false, text)
            .on_hover_text(rel)
            .clicked()
        {
            *to_open = Some(path.clone());
        }
    });
}

fn tree_nodes(
    ui: &mut egui::Ui,
    nodes: &[super::super::state::FileNode],
    depth: usize,
    bookmarks: &std::collections::HashSet<std::path::PathBuf>,
    bookmarks_only: bool,
    cat: FileCat,
    to_open: &mut Option<std::path::PathBuf>,
    to_toggle: &mut Option<std::path::PathBuf>,
) {
    for node in nodes {
        if node.is_dir {
            // Filter aktif (bookmark &/atau kategori): folder hanya tampil bila
            // berisi minimal satu file yang memenuhi SEMUA filter aktif — satu
            // predikat tunggal agar tidak muncul folder kosong (dua guard
            // terpisah bisa cocok terhadap file yang berbeda).
            if (bookmarks_only || cat != FileCat::All)
                && !dir_has(node, bookmarks, bookmarks_only, cat)
            {
                continue;
            }
            let id = egui::Id::new(&node.path).with("dir");
            egui::CollapsingHeader::new(egui::RichText::new(format!("📁 {}", node.name)).weak())
                .id_salt(id)
                .default_open(depth < 1)
                .show(ui, |ui| {
                    tree_nodes(
                        ui,
                        &node.children,
                        depth + 1,
                        bookmarks,
                        bookmarks_only,
                        cat,
                        to_open,
                        to_toggle,
                    );
                });
        } else {
            // Filter bookmark: hanya file yang di-bookmark yang tampil.
            if bookmarks_only && !bookmarks.contains(&node.path) {
                continue;
            }
            // Filter kategori: lewati file yang tidak cocok.
            let fcat = classify_file(&node.path);
            if cat != FileCat::All && fcat != cat {
                continue;
            }
            let is_bm = bookmarks.contains(&node.path);
            file_row(
                ui,
                &node.name,
                &node.path.display().to_string(),
                &node.path,
                is_bm,
                fcat,
                to_open,
                to_toggle,
            );
        }
    }
}

/// Apakah subtree berisi file yang memenuhi SEMUA filter aktif: bookmark
/// (bila `bookmark_filter`) DAN kategori (bila `cat != All`). Satu predikat
/// tunggal — folder tidak tampil bila filter gabungan tidak punya file yang
/// memenuhi keduanya sekaligus (cegah folder kosong).
fn dir_has(
    node: &super::super::state::FileNode,
    bookmarks: &std::collections::HashSet<std::path::PathBuf>,
    bookmark_filter: bool,
    cat: FileCat,
) -> bool {
    if !node.is_dir {
        if bookmark_filter && !bookmarks.contains(&node.path) {
            return false;
        }
        if cat != FileCat::All && classify_file(&node.path) != cat {
            return false;
        }
        return true;
    }
    node.children
        .iter()
        .any(|c| dir_has(c, bookmarks, bookmark_filter, cat))
}

// ── Symbols: modules/packages/interfaces dari hasil compile ──
fn symbols_tab(ui: &mut egui::Ui, state: &mut GuiState) {
    let Some(info) = &state.compile_info else {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Compile dulu untuk melihat symbols")
                .weak()
                .italics(),
        );
        return;
    };

    egui::ScrollArea::vertical()
        .id_salt("symbols_scroll")
        .show(ui, |ui| {
            if !info.modules.is_empty() {
                ui.label(egui::RichText::new("Modules").strong().size(11.0));
                for m in &info.modules {
                    ui.label(
                        egui::RichText::new(format!("▸ {}", m))
                            .monospace()
                            .size(12.0),
                    );
                }
                ui.add_space(6.0);
            }
            if !info.packages.is_empty() {
                ui.label(egui::RichText::new("Packages").strong().size(11.0));
                for m in &info.packages {
                    ui.label(
                        egui::RichText::new(format!("◈ {}", m))
                            .monospace()
                            .size(12.0),
                    );
                }
                ui.add_space(6.0);
            }
            if !info.interfaces.is_empty() {
                ui.label(egui::RichText::new("Interfaces").strong().size(11.0));
                for m in &info.interfaces {
                    ui.label(
                        egui::RichText::new(format!("◇ {}", m))
                            .monospace()
                            .size(12.0),
                    );
                }
            }
        });
}
