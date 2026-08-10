//! `mcheck` — Project Health Checker.
//!
//! Memeriksa: missing file (`include), circular include, unresolved dependency,
//! module instantiation cycle, inkonsistensi timescale.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use maria_ast::types::GenerateItem;
use maria_core::error::SimError;
use crate::{collect_targets, section, kv};

/// Opsi mcheck.
pub struct CheckArgs<'a> {
    pub targets: &'a [String],
    pub all: bool,
    pub missing: bool,
    pub circular: bool,
    pub deps: bool,
    pub cycles: bool,
    pub timescale: bool,
}

/// Jalankan mcheck.
pub fn run(args: &CheckArgs) -> Result<(), SimError> {
    let all = args.all;
    let check = |flag: bool| flag || all;

    let mut problems = 0usize;

    if check(args.missing) || check(args.circular) {
        problems += scan_includes(args.targets, check(args.missing), check(args.circular))?;
    }
    if check(args.deps) || check(args.cycles) || check(args.timescale) {
        problems += scan_design(
            args.targets,
            check(args.deps),
            check(args.cycles),
            check(args.timescale),
        )?;
    }

    if problems == 0 {
        println!("✅ project sehat — tidak ada masalah ditemukan");
    } else {
        println!("\n⚠️  {} masalah ditemukan", problems);
    }
    Ok(())
}

/// Representasi include per file.
struct IncludeInfo {
    /// Includes yang berhasil di-resolve (path absolut).
    resolved: Vec<PathBuf>,
    /// Include yang tidak ditemukan.
    missing: Vec<String>,
}

/// Scan semua file: cek `include missing + circular include graph.
fn scan_includes(targets: &[String], do_missing: bool, do_circular: bool) -> Result<usize, SimError> {
    let files = collect_targets(targets)?;
    if files.is_empty() {
        return Ok(0);
    }

    // Build include graph + missing list
    let mut graph: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    let mut missing_total = 0usize;
    let mut missing_seen: HashSet<PathBuf> = HashSet::new();

    section("Include Scan");
    for path in &files {
        let info = analyze_includes(path, &files)?;
        graph.insert(path.clone(), info.resolved.clone());
        if do_missing {
            for m in &info.missing {
                println!(
                    "  ! MISSING INCLUDE: {} (`include \"{}\")",
                    path.display(),
                    m
                );
                missing_total += 1;
            }
        }
        missing_seen.extend(info.missing.iter().map(PathBuf::from));
    }

    let mut problem = 0usize;

    if do_missing {
        kv("missing includes", missing_total);
        problem += missing_total;
    }

    if do_circular {
        let mut cycles = 0usize;
        let cycle = find_include_cycle(&graph, &files);
        if let Some(c) = cycle {
            println!("  ! CIRCULAR INCLUDE:");
            for p in &c {
                println!("      {}", p.display());
            }
            cycles += 1;
        }
        kv("circular include cycles", cycles);
        problem += cycles;
    }

    Ok(problem)
}

/// Ekstrak `include directive dari satu file + resolve terhadap dir/incdirs.
fn analyze_includes(path: &Path, all_files: &[PathBuf]) -> Result<IncludeInfo, SimError> {
    let src = std::fs::read_to_string(path)
        .map_err(|e| SimError::with_diag(maria_core::diagnostics::DiagCode::IoError, format!("{}: {}", path.display(), e)))?;

    let file_set: HashSet<&Path> = all_files.iter().map(|p| p.as_path()).collect();
    let dir = path.parent().unwrap_or(Path::new("."));

    let mut resolved = Vec::new();
    let mut missing = Vec::new();

    for line in src.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("`include") && !trimmed.starts_with("`INCLUDE") {
            continue;
        }
        // Ekstrak nama file antara tanda kutip
        let Some(open) = trimmed.find('"') else { continue };
        let rest = &trimmed[open + 1..];
        let Some(close) = rest.find('"') else { continue };
        let name = &rest[..close];
        if name.is_empty() {
            continue;
        }

        // Cari: direktori file dulu, lalu cari di semua file yang di-scan
        let cand = dir.join(name);
        if cand.exists() {
            let abs = std::fs::canonicalize(&cand).unwrap_or(cand);
            resolved.push(abs);
        } else if let Some(hit) = all_files.iter().find(|f| {
            f.file_name().map(|n| n == name).unwrap_or(false)
        }) {
            let abs = std::fs::canonicalize(hit).unwrap_or_else(|_| hit.clone());
            resolved.push(abs);
        } else if file_set.contains(cand.as_path()) {
            resolved.push(cand);
        } else {
            missing.push(name.to_string());
        }
    }
    Ok(IncludeInfo { resolved, missing })
}

/// Deteksi cycle pada include graph via DFS coloring.
fn find_include_cycle(graph: &HashMap<PathBuf, Vec<PathBuf>>, files: &[PathBuf]) -> Option<Vec<PathBuf>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White,
        Gray,
        Black,
    }
    let mut color: HashMap<PathBuf, Color> = HashMap::new();
    for f in files {
        color.insert(f.clone(), Color::White);
    }

    fn dfs(
        u: &PathBuf,
        graph: &HashMap<PathBuf, Vec<PathBuf>>,
        color: &mut HashMap<PathBuf, Color>,
        stack: &mut Vec<PathBuf>,
    ) -> Option<Vec<PathBuf>> {
        color.insert(u.clone(), Color::Gray);
        stack.push(u.clone());
        if let Some(neighbors) = graph.get(u) {
            for v in neighbors {
                match color.get(v).copied().unwrap_or(Color::White) {
                    Color::Gray => {
                        let start = stack.iter().position(|x| x == v).unwrap_or(0);
                        let mut cycle = stack[start..].to_vec();
                        cycle.push(v.clone());
                        return Some(cycle);
                    }
                    Color::White => {
                        if let Some(c) = dfs(v, graph, color, stack) {
                            return Some(c);
                        }
                    }
                    Color::Black => {}
                }
            }
        }
        stack.pop();
        color.insert(u.clone(), Color::Black);
        None
    }

    let mut stack = Vec::new();
    for f in files {
        if color.get(f).copied().unwrap_or(Color::White) == Color::White {
            if let Some(c) = dfs(f, graph, &mut color, &mut stack) {
                return Some(c);
            }
        }
    }
    None
}

/// Cek berbasis AST: unresolved dependency, module cycle, timescale.
fn scan_design(
    targets: &[String],
    do_deps: bool,
    do_cycles: bool,
    do_timescale: bool,
) -> Result<usize, SimError> {
    let (design, session) = crate::open_project(targets, &[], &[], None)?;
    let mut problem = 0usize;

    let mods: HashMap<maria_core::intern::Symbol, _> =
        design.modules.iter().map(|m| (m.name, m.clone())).collect();

    if do_deps {
        section("Dependency Check");
        let mut known: HashSet<maria_core::intern::Symbol> = mods.keys().copied().collect();
        for i in &design.interfaces {
            known.insert(i.name);
        }
        let mut unresolved: Vec<String> = Vec::new();
        for m in &design.modules {
            let mut insts = Vec::new();
            collect_mod_instances(&m.items, &mut insts);
            for i in insts {
                if !known.contains(&i) {
                    let s = i.as_str().to_string();
                    if !unresolved.contains(&s) {
                        unresolved.push(s);
                    }
                }
            }
        }
        unresolved.sort();
        for u in &unresolved {
            println!("  ! UNRESOLVED: module '{}' di-instantiate tapi tidak terdefinisi", u);
        }
        kv("unresolved modules", unresolved.len());
        problem += unresolved.len();
    }

    if do_cycles {
        section("Instantiation Cycle Check");
        if let Some(cycle) = session.module_index.detect_cycles() {
            let path = cycle.join(" → ");
            println!("  ! CYCLE: {}", path);
            problem += 1;
        } else {
            kv("cycles", 0);
        }
    }

    if do_timescale {
        section("Timescale Check");
        let mut seen: HashMap<String, usize> = HashMap::new();
        for m in &design.modules {
            // Pakai timescale design (dari file terakhir) — cek per module tidak
            // punya info timescale; verifikasi lewat Preprocessor per file.
            let _ = m;
        }
        if let Some(ts) = &design.timescale {
            let key = format!("{}/{}", ts.0, ts.1);
            *seen.entry(key).or_insert(0) += 1;
        }
        // Scan tiap file untuk `timescale
        let files = collect_targets(targets)?;
        for f in &files {
            if let Ok(src) = std::fs::read_to_string(f) {
                for line in src.lines() {
                    let t = line.trim();
                    if t.starts_with('`') && t.contains("timescale") {
                        // Ambil dua token setelah backtick-timescale
                        let after = t.replacen('`', "", 1);
                        let parts: Vec<&str> = after
                            .split_whitespace()
                            .filter(|w| !w.is_empty() && *w != "timescale")
                            .collect();
                        if parts.len() >= 2 {
                            let key = format!("{}/{}", parts[0].trim_matches('`'), parts[1]);
                            *seen.entry(key).or_insert(0) += 1;
                        }
                        break;
                    }
                }
            }
        }
        if seen.len() > 1 {
            let mut list: Vec<String> = seen.iter().map(|(k, v)| format!("{} ({} file)", k, v)).collect();
            list.sort();
            for l in &list {
                println!("  ! {}", l);
            }
            println!("  ! INKONSISTENSI timescale: {} nilai berbeda", seen.len());
            problem += seen.len().saturating_sub(1);
        } else if let Some((k, v)) = seen.iter().next() {
            kv("timescale", format!("{} ({} file)", k, v));
        }
    }

    Ok(problem)
}

fn collect_mod_instances(items: &[maria_ast::types::ModuleItem], out: &mut Vec<maria_core::intern::Symbol>) {
    use maria_ast::types::ModuleItem;
    for it in items {
        match it {
            ModuleItem::Instance(inst) => out.push(inst.module_name),
            ModuleItem::Generate(g) => {
                for gi in &g.items {
                    collect_gen_instances(gi, out);
                }
            }
            _ => {}
        }
    }
}

fn collect_gen_instances(gi: &GenerateItem, out: &mut Vec<maria_core::intern::Symbol>) {
    match gi {
        GenerateItem::Items(items) => collect_mod_instances(items, out),
        GenerateItem::If { true_items, false_items, .. } => {
            collect_mod_instances(true_items, out);
            collect_mod_instances(false_items, out);
        }
        GenerateItem::For { body_items, .. } => collect_mod_instances(body_items, out),
        GenerateItem::Case { items, default, .. } => {
            for ci in items {
                collect_mod_instances(&ci.body, out);
            }
            if let Some(d) = default {
                collect_mod_instances(d, out);
            }
        }
    }
}
