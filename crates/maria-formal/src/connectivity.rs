//! Connectivity Checking — FORMAL-13 (tahap 1).
//!
//! Analisis statis jalur koneksi combinational antar sinyal:
//! bangun graf dependensi (rhs → lhs) dari semua assignment proses
//! Combinational/CombReactive + Initial, lalu BFS dari sumber ke tujuan.
//!
//! MVP scope: jalur combinational murni (stateless). Jalur melalui
//! register/NBA (sequential dependency lintas cycle) belum dimodelkan —
//! terdokumentasi sebagai keterbatasan.

use maria_ir::*;

/// Hasil pemeriksaan satu pasangan (src, dst).
#[derive(Debug, Clone)]
pub struct ConnectivityResult {
    pub src: String,
    pub dst: String,
    /// true bila ada jalur kombinational dari src ke dst.
    pub connected: bool,
    /// Jumlah hop assignment pada jalur terpendek (None bila tak terhubung).
    pub path_len: Option<usize>,
    /// Nama sinyal sepanjang jalur: [src, ..., dst].
    pub path: Vec<String>,
    /// Pesan kesalahan (mis. sinyal tidak dikenal).
    pub error: Option<String>,
}

/// Kumpulkan semua pasangan (lhs_id, rhs_expr) dari proses yang relevan.
fn collect_assignments(processes: &[Process]) -> Vec<(usize, IrExpr)> {
    fn walk(stmts: &[IrStmt], out: &mut Vec<(usize, IrExpr)>) {
        for stmt in stmts {
            match stmt {
                IrStmt::BlockingAssign {
                    lhs: IrLValue::Signal(id, _),
                    rhs,
                    ..
                } => out.push((*id, rhs.clone())),
                IrStmt::Block { stmts: inner } => walk(inner, out),
                IrStmt::NamedBlock { stmts: inner, .. } => walk(inner, out),
                IrStmt::If {
                    true_branch,
                    false_branch,
                    ..
                } => {
                    walk(true_branch, out);
                    walk(false_branch, out);
                }
                IrStmt::Case { items, default, .. } => {
                    for item in items {
                        walk(&item.body, out);
                    }
                    walk(default, out);
                }
                _ => {}
            }
        }
    }

    let mut out = Vec::new();
    for process in processes {
        match process {
            Process::Combinational { body, .. }
            | Process::CombReactive { body, .. }
            | Process::Initial { name: _, body } => walk(body, &mut out),
            _ => {}
        }
    }
    out
}

/// Kumpulkan id sinyal yang dirujuk dalam ekspresi.
fn collect_expr_signals(expr: &IrExpr, out: &mut Vec<usize>) {
    match expr {
        IrExpr::Signal(id, _) => out.push(*id),
        IrExpr::BinaryOp(_, l, r) => {
            collect_expr_signals(l, out);
            collect_expr_signals(r, out);
        }
        IrExpr::UnaryOp(_, v) => collect_expr_signals(v, out),
        IrExpr::Cond(c, t, f) => {
            collect_expr_signals(c, out);
            collect_expr_signals(t, out);
            collect_expr_signals(f, out);
        }
        IrExpr::Signed(v) => collect_expr_signals(v, out),
        // Const/FillLit/RangeSelect atas const dll: tanpa referensi sinyal
        // tambahan untuk MVP (RangeSelect dengan basis Signal ditangani
        // lewat arm lain bila bentuknya berbeda di skema IR saat ini).
        _ => {}
    }
}

/// Resolve nama sinyal → index di design.top.signals.
fn find_signal_index(design: &IrDesign, name: &str) -> Option<usize> {
    design
        .top
        .signals
        .iter()
        .position(|s| s.name.as_str() == name)
}

/// Periksa konektivitas kombinational untuk setiap pasangan (src, dst).
pub fn check_connectivity(
    design: &IrDesign,
    pairs: &[(String, String)],
) -> Vec<ConnectivityResult> {
    let n = design.top.signals.len();
    let name_of = |id: usize| -> String {
        design
            .top
            .signals
            .get(id)
            .map(|s| s.name.to_string())
            .unwrap_or_else(|| format!("sig_{}", id))
    };

    // Adjacency: lhs <- daftar sinyal sumber rhs. Simpan juga hop untuk
    // rekonstruksi jalur (BFS dari src maju mengikuti edge rhs→lhs).
    let mut adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n]; // (dst_id, assign_idx)
    let assigns = collect_assignments(&design.top.processes);
    for (ai, (lhs, rhs)) in assigns.iter().enumerate() {
        if *lhs >= n {
            continue;
        }
        let mut sources = Vec::new();
        collect_expr_signals(rhs, &mut sources);
        sources.sort_unstable();
        sources.dedup();
        for s in sources {
            if s < n && s != *lhs {
                adj[s].push((*lhs, ai));
            }
        }
    }

    let mut results = Vec::new();
    for (src_name, dst_name) in pairs {
        let mut res = ConnectivityResult {
            src: src_name.clone(),
            dst: dst_name.clone(),
            connected: false,
            path_len: None,
            path: Vec::new(),
            error: None,
        };
        let Some(src) = find_signal_index(design, src_name) else {
            res.error = Some(format!("sinyal '{}' tidak ditemukan", src_name));
            results.push(res);
            continue;
        };
        let Some(dst) = find_signal_index(design, dst_name) else {
            res.error = Some(format!("sinyal '{}' tidak ditemukan", dst_name));
            results.push(res);
            continue;
        };

        // BFS dari src.
        let mut prev: Vec<Option<(usize, usize)>> = vec![None; n]; // (node_sebelumnya, assign_idx)
        let mut visited = vec![false; n];
        visited[src] = true;
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(src);
        while let Some(cur) = queue.pop_front() {
            if cur == dst && cur != src {
                break;
            }
            for &(next, ai) in &adj[cur] {
                if !visited[next] {
                    visited[next] = true;
                    prev[next] = Some((cur, ai));
                    queue.push_back(next);
                }
            }
        }

        if !visited[dst] || (src == dst) {
            if src == dst {
                // Trivially connected ke dirinya sendiri.
                res.connected = true;
                res.path_len = Some(0);
                res.path = vec![src_name.clone()];
            }
            results.push(res);
            continue;
        }

        // Rekonstruksi jalur dst ← ... ← src.
        let mut path_ids = vec![dst];
        let mut cur = dst;
        while let Some((p, _)) = prev[cur] {
            path_ids.push(p);
            cur = p;
            if cur == src {
                break;
            }
        }
        path_ids.reverse();
        res.connected = true;
        res.path_len = Some(path_ids.len() - 1);
        res.path = path_ids.iter().map(|&id| name_of(id)).collect();
        results.push(res);
    }
    results
}
