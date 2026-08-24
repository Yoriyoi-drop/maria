//! Dependency tab — GRAF VISUAL instansiasi antar module (CPU → AXI → Cache → …).
//!
//! Berbeda dari Architecture (pohon instance hierarkis): Dependency menampilkan
//! view module-level sebagai diagram node-link — setiap module adalah node,
//! edge dari parent ke module yang di-instansiasinya (label = jumlah instance).
//! Layout berlapis (layer = jarak topologis dari root) dihitung sekali per
//! compile dan di-cache di `GuiState.dep_graph` (deteksi siklus bisa mahal).
//! Klik node → buka file RTL via `module_files`; node dalam siklus dependensi
//! ditandai merah.

use std::collections::{HashMap, VecDeque};

use eframe::egui;

use super::super::state::{DepGraphLayout, DepGraphNode, DepRow, GuiState};

// ── Konstanta layout ──
const LAYER_GAP: f32 = 210.0; // jarak horizontal antar kolom (layer)
const NODE_H: f32 = 34.0;
const V_GAP: f32 = 16.0;
const PAD_X: f32 = 14.0;
const PAD_Y: f32 = 14.0;

// ── Warna ──
const EDGE_COLOR: egui::Color32 = egui::Color32::from_rgb(96, 100, 112);
const EDGE_CYCLE: egui::Color32 = egui::Color32::from_rgb(239, 68, 68);
const NODE_FILL: egui::Color32 = egui::Color32::from_rgb(24, 28, 36);
const NODE_FILL_HOVER: egui::Color32 = egui::Color32::from_rgb(30, 41, 59);
const NODE_BORDER: egui::Color32 = egui::Color32::from_rgb(70, 76, 90);
const NODE_ACCENT: egui::Color32 = egui::Color32::from_rgb(59, 130, 246);
const CYCLE_BORDER: egui::Color32 = egui::Color32::from_rgb(239, 68, 68);
const NAME_COLOR: egui::Color32 = egui::Color32::from_rgb(226, 232, 240);
const SUB_COLOR: egui::Color32 = egui::Color32::from_gray(140);

pub fn show(ui: &mut egui::Ui, state: &mut GuiState) {
    let Some(info) = &state.compile_info else {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Compile dulu untuk melihat dependency")
                .weak()
                .italics(),
        );
        return;
    };
    if info.deps.is_empty() {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Tidak ada module yang menginstansiasi module lain")
                .weak()
                .italics(),
        );
        return;
    }

    // ── Layout cache: hitung ulang hanya saat graf berubah (key berbeda) ──
    let key = dep_key(&info.deps);
    let layout = match &state.dep_graph {
        Some(l) if l.key == key => l.clone(),
        _ => {
            let l = build_layout(&info.deps);
            state.dep_graph = Some(l.clone());
            l
        }
    };
    let module_files = info.module_files.clone();

    // ── Legend ──
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("● module")
                .color(NODE_BORDER)
                .size(11.0),
        );
        ui.separator();
        ui.label(
            egui::RichText::new("● siklus dependensi")
                .color(CYCLE_BORDER)
                .size(11.0),
        );
        ui.separator();
        ui.label(
            egui::RichText::new("klik node → buka file RTL · drag/scroll untuk pan")
                .weak()
                .size(11.0),
        );
        ui.separator();
        let cyc = layout.nodes.iter().filter(|n| n.in_cycle).count();
        ui.label(
            egui::RichText::new(format!("{} node · {} siklus", layout.nodes.len(), cyc))
                .weak()
                .size(11.0),
        );
    });
    ui.separator();

    let mut to_open: Option<std::path::PathBuf> = None;
    egui::ScrollArea::both()
        .id_salt("dep_graph_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let canvas = egui::vec2(layout.width, layout.height);
            let (crect, cresp) = ui.allocate_exact_size(canvas, egui::Sense::click());
            let painter = ui.painter_at(crect);

            // ── Edge (di belakang node) ──
            for (i, node) in layout.nodes.iter().enumerate() {
                for (j, count) in &node.edges {
                    draw_edge(&painter, &layout, i, *j, *count);
                }
            }

            // ── Node ──
            let hover_pos = cresp.hover_pos();
            let mut hovered: Option<usize> = None;
            for (i, node) in layout.nodes.iter().enumerate() {
                let rect = node_rect(node);
                let is_hover = hover_pos.map(|p| rect.contains(p)).unwrap_or(false);
                if is_hover {
                    hovered = Some(i);
                }
                let fill = if is_hover { NODE_FILL_HOVER } else { NODE_FILL };
                painter.rect_filled(rect, egui::CornerRadius::same(6), fill);
                let border = if node.in_cycle {
                    CYCLE_BORDER
                } else if is_hover {
                    NODE_ACCENT
                } else {
                    NODE_BORDER
                };
                painter.rect_stroke(
                    rect,
                    egui::CornerRadius::same(6),
                    egui::Stroke::new(if node.in_cycle { 2.0 } else { 1.5 }, border),
                    egui::StrokeKind::Inside,
                );
                // Nama module (truncate bila terlalu panjang)
                let text_rect = egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 8.0, rect.top()),
                    egui::pos2(rect.right() - 6.0, rect.bottom()),
                );
                painter.text(
                    text_rect.left_center(),
                    egui::Align2::LEFT_CENTER,
                    truncate(&node.name, node.w - 30.0),
                    egui::FontId::monospace(11.0),
                    NAME_COLOR,
                );
                if !node.edges.is_empty() {
                    painter.text(
                        egui::pos2(rect.right() - 6.0, rect.top() + 3.0),
                        egui::Align2::RIGHT_TOP,
                        format!("×{}", node.edges.iter().map(|(_, n)| n).sum::<usize>()),
                        egui::FontId::monospace(9.0),
                        SUB_COLOR,
                    );
                }
            }

            // ── Tooltip hovered node (painter panel — tanpa API popup) ──
            if let Some(i) = hovered {
                draw_node_tooltip(&painter, &layout, i);
            }

            // ── Klik node → buka file RTL ──
            if cresp.clicked() {
                if let Some(p) = cresp.interact_pointer_pos() {
                    for node in &layout.nodes {
                        if node_rect(node).contains(p) {
                            if let Some(file) = module_files.get(&node.name) {
                                to_open = Some(file.clone());
                            }
                            break;
                        }
                    }
                }
            }
        });

    if let Some(path) = to_open {
        state.open_file(path);
    }
}

/// Rect layar node (posisi absolut dalam canvas).
fn node_rect(node: &DepGraphNode) -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(node.x, node.y), egui::vec2(node.w, node.h))
}

/// Potong nama module agar muat dalam lebar node (elipsis).
fn truncate(name: &str, max_w: f32) -> String {
    // ~7.2px per char monospace 11
    let max_chars = (max_w / 7.2).max(4.0) as usize;
    if name.chars().count() > max_chars {
        let cut: String = name.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{}…", cut)
    } else {
        name.to_string()
    }
}

/// Gambar satu edge: cubic bezier dari sisi kanan parent ke sisi kiri child
/// + label jumlah instance + panah di ujung.
fn draw_edge(painter: &egui::Painter, layout: &DepGraphLayout, i: usize, j: usize, count: usize) {
    let a = &layout.nodes[i];
    let b = &layout.nodes[j];
    let p0 = egui::pos2(a.x + a.w, a.y + a.h / 2.0);
    let p3 = egui::pos2(b.x, b.y + b.h / 2.0);
    let same_col = (p3.x - p0.x).abs() < 1.0;
    let dx = if same_col {
        60.0
    } else {
        (p3.x - p0.x).abs() * 0.5
    };
    let (c1, c2) = if same_col {
        // Edge antar node sekolom (siklus) — lekuk ke bawah.
        (
            egui::pos2(p0.x + dx, p0.y + 20.0),
            egui::pos2(p3.x - dx, p3.y + 20.0),
        )
    } else {
        (egui::pos2(p0.x + dx, p0.y), egui::pos2(p3.x - dx, p3.y))
    };

    let in_cycle = a.in_cycle || b.in_cycle;
    let color = if in_cycle { EDGE_CYCLE } else { EDGE_COLOR };
    painter.add(egui::Shape::CubicBezier(egui::epaint::CubicBezierShape {
        points: [p0, c1, c2, p3],
        closed: false,
        fill: egui::Color32::TRANSPARENT,
        stroke: egui::epaint::PathStroke::new(1.5, color),
    }));

    // Panah di ujung (arah = p3 - c2)
    let dir = (p3 - c2).normalized();
    let perp = egui::vec2(-dir.y, dir.x);
    let base = p3 - dir * 8.0;
    painter.add(egui::Shape::convex_polygon(
        vec![base + perp * 4.0, base - perp * 4.0, p3],
        color,
        egui::Stroke::NONE,
    ));

    // Label jumlah instance di titik tengah kurva
    if count > 1 {
        let mid = egui::pos2((p0.x + p3.x) / 2.0, (p0.y + p3.y) / 2.0);
        painter.text(
            mid,
            egui::Align2::CENTER_CENTER,
            format!("×{}", count),
            egui::FontId::monospace(9.0),
            color,
        );
    }
}

/// Panel info kecil di bawah node yang di-hover (pengganti tooltip popup —
/// stabil tanpa API show_tooltip versi-spesifik).
fn draw_node_tooltip(painter: &egui::Painter, layout: &DepGraphLayout, i: usize) {
    let node = &layout.nodes[i];
    let name = node.name.clone();
    let edges: usize = node.edges.iter().map(|(_, n)| n).sum();
    let mut lines: Vec<String> = vec![format!("module {}", name), format!("{} dependency", edges)];
    if node.in_cycle {
        lines.push("⚠ bagian dari SIKLUS dependensi".to_string());
    }
    lines.push("klik → buka file RTL".to_string());

    let font = egui::FontId::monospace(10.0);
    let char_w = 6.4f32;
    let line_h = 14.0f32;
    let pad = 6.0f32;
    let w = lines
        .iter()
        .map(|l| l.chars().count() as f32 * char_w + pad * 2.0)
        .fold(0.0f32, f32::max)
        .min(280.0);
    let h = lines.len() as f32 * line_h + pad * 2.0;

    let mut rect =
        egui::Rect::from_min_size(egui::pos2(node.x, node.y + node.h + 6.0), egui::vec2(w, h));
    // Jangan keluar canvas
    if rect.bottom() > layout.height - 4.0 {
        rect = rect.translate(egui::vec2(0.0, -(h + node.h + 12.0)));
    }
    if rect.right() > layout.width - 4.0 {
        rect = rect.translate(egui::vec2(layout.width - 4.0 - rect.right(), 0.0));
    }
    if rect.left() < 4.0 {
        rect = rect.translate(egui::vec2(4.0 - rect.left(), 0.0));
    }
    // Batas atas (node dekat puncak canvas — translate ke bawah lagi)
    if rect.top() < 4.0 {
        rect = rect.translate(egui::vec2(0.0, 4.0 - rect.top()));
    }

    painter.rect_filled(
        rect,
        egui::CornerRadius::same(6),
        egui::Color32::from_rgb(15, 18, 24),
    );
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(6),
        egui::Stroke::new(1.0, NODE_BORDER),
        egui::StrokeKind::Inside,
    );
    let mut y = rect.top() + pad + line_h / 2.0;
    for (k, line) in lines.iter().enumerate() {
        let color = if k == 0 {
            NAME_COLOR
        } else if line.contains("SIKLUS") {
            CYCLE_BORDER
        } else {
            SUB_COLOR
        };
        painter.text(
            egui::pos2(rect.left() + pad, y),
            egui::Align2::LEFT_CENTER,
            line.clone(),
            font.clone(),
            color,
        );
        y += line_h;
    }
}

// ────────────────────────── Layout & analisis ──────────────────────────

/// Tambah nama ke graf (jika belum ada), kembalikan index internal. Dipakai
/// `build_layout` — free function agar tidak ada borrow conflict closure
/// dengan `children`/`parents` (E0499).
fn ensure_index(
    index: &mut HashMap<String, usize>,
    names: &mut Vec<String>,
    children: &mut Vec<Vec<(usize, usize)>>,
    parents: &mut Vec<Vec<usize>>,
    name: &str,
) -> usize {
    if let Some(&i) = index.get(name) {
        return i;
    }
    let i = names.len();
    names.push(name.to_string());
    index.insert(name.to_string(), i);
    children.push(Vec::new());
    parents.push(Vec::new());
    i
}

/// Hash FNV-1a ringkas dari graf dependensi — kunci cache layout.
fn dep_key(deps: &[DepRow]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for row in deps {
        for b in row.module.bytes() {
            h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
        for (c, n) in &row.children {
            for b in c.bytes() {
                h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
            }
            h = (h ^ *n as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// Bangun layout berlapis dari graf `deps`: layer = jarak BFS dari root,
/// node disusun vertikal dalam kolom per layer, edge parent → child.
/// Node dalam siklus (tak terjangkau root) ditaruh di layer terakhir.
fn build_layout(deps: &[DepRow]) -> DepGraphLayout {
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut names: Vec<String> = Vec::new();
    let mut children: Vec<Vec<(usize, usize)>> = Vec::new();
    let mut parents: Vec<Vec<usize>> = Vec::new();
    for row in deps {
        let p = ensure_index(
            &mut index,
            &mut names,
            &mut children,
            &mut parents,
            &row.module,
        );
        for (c, n) in &row.children {
            let ci = ensure_index(&mut index, &mut names, &mut children, &mut parents, c);
            children[p].push((ci, *n));
            parents[ci].push(p);
        }
    }

    let n = names.len();
    let roots: Vec<usize> = (0..n).filter(|&i| parents[i].is_empty()).collect();

    // BFS layers dari root
    let mut layer: Vec<usize> = vec![usize::MAX; n];
    let mut queue: VecDeque<usize> = VecDeque::new();
    for r in roots {
        layer[r] = 0;
        queue.push_back(r);
    }
    let mut max_layer = 0usize;
    while let Some(u) = queue.pop_front() {
        for (c, _) in &children[u] {
            if layer[*c] == usize::MAX {
                layer[*c] = layer[u] + 1;
                max_layer = max_layer.max(layer[*c]);
                queue.push_back(*c);
            }
        }
    }
    // Sisa (siklus tak terjangkau root) → layer terakhir
    for i in 0..n {
        if layer[i] == usize::MAX {
            layer[i] = max_layer + 1;
            max_layer = max_layer.max(layer[i]);
        }
    }

    // Kelompok per layer, urut nama (layout stabil)
    let mut by_layer: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        by_layer.entry(layer[i]).or_default().push(i);
    }
    for list in by_layer.values_mut() {
        list.sort_by_key(|&i| names[i].clone());
    }

    // Lebar kolom & tinggi per layer
    let mut col_w: Vec<f32> = vec![0.0; max_layer + 1];
    let mut col_h: Vec<f32> = vec![0.0; max_layer + 1];
    for (l, list) in &by_layer {
        let w = list
            .iter()
            .map(|&i| node_width(&names[i]))
            .fold(0.0f32, f32::max);
        col_w[*l] = w;
        col_h[*l] = list.len() as f32 * (NODE_H + V_GAP) - V_GAP;
    }
    let max_col_h = col_h.iter().cloned().fold(0.0f32, f32::max);

    // Offset horizontal & vertikal per layer
    let mut x_off: Vec<f32> = vec![0.0; max_layer + 1];
    {
        let mut acc = PAD_X;
        for l in 0..=max_layer {
            x_off[l] = acc;
            acc += col_w[l] + LAYER_GAP;
        }
    }
    let mut y_off: Vec<f32> = vec![0.0; max_layer + 1];
    for l in 0..=max_layer {
        y_off[l] = PAD_Y + (max_col_h - col_h[l]) * 0.5;
    }

    // Bangun node (posisi node[i] dikaitkan ke nama index via pos_of)
    let mut nodes: Vec<DepGraphNode> = Vec::with_capacity(n);
    let mut pos_of: HashMap<usize, usize> = HashMap::new();
    for l in 0..=max_layer {
        let Some(list) = by_layer.get(&l) else {
            continue;
        };
        for (k, &i) in list.iter().enumerate() {
            pos_of.insert(i, nodes.len());
            let w = node_width(&names[i]);
            nodes.push(DepGraphNode {
                name: names[i].clone(),
                x: x_off[l] + (col_w[l] - w) * 0.5,
                y: y_off[l] + k as f32 * (NODE_H + V_GAP),
                w,
                h: NODE_H,
                in_cycle: false,
                edges: Vec::new(),
            });
        }
    }
    // Remap edge ke index node (target)
    for i in 0..n {
        let pi = pos_of[&i];
        nodes[pi].edges = children[i]
            .iter()
            .filter_map(|(c, cnt)| pos_of.get(c).map(|pc| (*pc, *cnt)))
            .collect();
    }

    let width = x_off[max_layer] + col_w[max_layer] + PAD_X;
    let height = max_col_h + PAD_Y * 2.0;
    let mut layout = DepGraphLayout {
        nodes,
        width,
        height,
        key: dep_key(deps),
    };

    // Tandai node dalam siklus (reachability per edge — di-cache, jadi hanya
    // dihitung sekali per compile).
    let in_cycle = find_cycles(&layout.nodes);
    for (k, node) in layout.nodes.iter_mut().enumerate() {
        node.in_cycle = in_cycle[k];
    }
    layout
}

/// Lebar node berdasarkan panjang nama (monospace 11px ≈ 7.2px/char).
fn node_width(name: &str) -> f32 {
    (name.len() as f32 * 7.2 + 40.0).clamp(120.0, 220.0)
}

/// Deteksi node yang bagian dari siklus: node `u` ber-siklus bila ada edge
/// (u→v) dan `v` dapat mencapai `u` (BFS). Menangkap semua node dalam SCC
/// non-trivial maupun self-loop.
fn find_cycles(nodes: &[DepGraphNode]) -> Vec<bool> {
    let mut in_cycle = vec![false; nodes.len()];
    for u in 0..nodes.len() {
        for (v, _) in &nodes[u].edges {
            if reachable(nodes, *v, u) {
                in_cycle[u] = true;
                in_cycle[*v] = true;
            }
        }
    }
    in_cycle
}

/// BFS: apakah `start` dapat mencapai `target`?
fn reachable(nodes: &[DepGraphNode], start: usize, target: usize) -> bool {
    if start == target {
        return true;
    }
    let mut seen = vec![false; nodes.len()];
    let mut stack: Vec<usize> = vec![start];
    seen[start] = true;
    while let Some(u) = stack.pop() {
        for (v, _) in &nodes[u].edges {
            if *v == target {
                return true;
            }
            if !seen[*v] {
                seen[*v] = true;
                stack.push(*v);
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(module: &str, children: &[(&str, usize)]) -> DepRow {
        DepRow {
            module: module.to_string(),
            children: children.iter().map(|(c, n)| (c.to_string(), *n)).collect(),
        }
    }

    #[test]
    fn layout_puts_root_in_first_layer() {
        let deps = vec![row("cpu", &[("axi", 1)]), row("axi", &[("dram", 2)])];
        let layout = build_layout(&deps);
        assert_eq!(layout.nodes.len(), 3);
        let cpu = layout.nodes.iter().find(|n| n.name == "cpu").unwrap();
        let axi = layout.nodes.iter().find(|n| n.name == "axi").unwrap();
        let dram = layout.nodes.iter().find(|n| n.name == "dram").unwrap();
        assert!(cpu.x < axi.x, "root di kolom paling kiri");
        assert!(axi.x < dram.x, "layer bertambah ke kanan");
        assert_eq!(cpu.edges.len(), 1);
        assert_eq!(cpu.edges[0].1, 1);
        let axi_edges: usize = axi.edges.iter().map(|(_, n)| n).sum();
        assert_eq!(axi_edges, 2);
    }

    #[test]
    fn cycle_nodes_are_marked() {
        // a → b → c → a (siklus penuh) + d → b (d tidak siklus)
        let deps = vec![
            row("a", &[("b", 1)]),
            row("b", &[("c", 1)]),
            row("c", &[("a", 1)]),
            row("d", &[("b", 1)]),
        ];
        let layout = build_layout(&deps);
        let flag = |name: &str| {
            layout
                .nodes
                .iter()
                .find(|n| n.name == name)
                .map(|n| n.in_cycle)
                .unwrap()
        };
        assert!(flag("a"), "a dalam siklus");
        assert!(flag("b"), "b dalam siklus");
        assert!(flag("c"), "c dalam siklus");
        assert!(!flag("d"), "d bukan bagian siklus");
    }

    #[test]
    fn self_loop_is_cycle() {
        let deps = vec![row("x", &[("x", 1)])];
        let layout = build_layout(&deps);
        assert!(layout.nodes[0].in_cycle);
    }

    #[test]
    fn acyclic_graph_has_no_cycles() {
        let deps = vec![
            row("top", &[("alu", 1), ("mem", 1)]),
            row("alu", &[("regfile", 1)]),
        ];
        let layout = build_layout(&deps);
        assert!(layout.nodes.iter().all(|n| !n.in_cycle));
    }
}
