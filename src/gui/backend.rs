//! Backend GUI — memanggil API library `maria` langsung (tanpa IPC).
//!
//! Compile/elaborate via `CompileSession`, simulasi via `SimulationEngine`.
//! Operasi berat dijalankan di worker thread; hasil dikirim melalui channel
//! sehingga UI tetap responsif.

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::sync::Arc;

use crate::error::SimError;
use crate::frontend::compile_session::{CompileSession, SessionConfig};
use crate::frontend::module_index::EntryKind;
use crate::ir::{IrDesign, LogicVal, LogicVec};
use crate::simulator::SimulationEngine;

use super::state::{
    CompileInfo, CovergroupRow, CoverageInfo, DepRow, DiagEntry, DiagLevel, FileNode, GuiEvent,
    SimInfo, SignalRow, WaveformSignal,
};

/// Scan direktori → pohon file (rekursif, sinkron).
pub fn scan_tree(root: &Path) -> Vec<FileNode> {
    fn build(dir: &Path) -> Vec<FileNode> {
        let mut nodes = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return nodes;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            // Skip hidden & build artifacts
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
            let is_dir = path.is_dir();
            let children = if is_dir { build(&path) } else { Vec::new() };
            nodes.push(FileNode {
                name,
                path,
                is_dir,
                children,
            });
        }
        nodes.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.cmp(&b.name))
        });
        nodes
    }
    build(root)
}

/// Compile + elaborate project di worker thread.
pub fn spawn_compile(tx: Sender<GuiEvent>, paths: Vec<PathBuf>) {
    std::thread::spawn(move || {
        let result = compile_project(&paths);
        let _ = tx.send(GuiEvent::CompileDone(result));
    });
}

/// Ubah `SimError` menjadi daftar `DiagEntry` (dengan `file`/`line` bila tersedia
/// dari source snippet). Dipakai supaya Mini Map editor & Problems tab bisa
/// menunjuk baris yang salah, bukan cuma error global.
fn simerr_to_diags(err: &SimError) -> Vec<DiagEntry> {
    let diag = err.to_diagnostic();
    let mut out = Vec::new();
    if let Some(snip) = &diag.source_snippet {
        out.push(DiagEntry {
            file: snip.file.clone(),
            line: snip.line,
            message: diag.message.to_string(),
            level: DiagLevel::Error,
        });
    }
    if out.is_empty() {
        out.push(DiagEntry {
            file: String::new(),
            line: 0,
            message: diag.message.to_string(),
            level: DiagLevel::Error,
        });
    }
    out
}

/// Compile + elaborate semua file. Mengembalikan info + design ter-elaborasi,
/// atau daftar diagnostics (error) dengan lokasi file/line.
pub fn compile_project(paths: &[PathBuf]) -> Result<(CompileInfo, IrDesign), Vec<DiagEntry>> {
    let start = std::time::Instant::now();

    let mut config = SessionConfig::default();
    for p in paths {
        config.sources.push(p.clone());
    }
    config.use_lazy_elab = true;
    config.auto_incdirs = true;

    let mut session = CompileSession::new(config);
    let (_design, ir_design, _idx) = match session.compile_and_elaborate(None) {
        Ok(v) => v,
        Err(e) => return Err(simerr_to_diags(&e)),
    };

    let modules: Vec<String> = session
        .module_index
        .iter()
        .filter(|(_, kind, _)| *kind == EntryKind::Module)
        .map(|(name, _, _)| name.to_string())
        .collect();
    // Module → file path (untuk klik-ke-buka di Architecture viewer)
    let module_files: HashMap<String, PathBuf> = session
        .module_index
        .iter()
        .filter(|(_, kind, _)| *kind == EntryKind::Module)
        .map(|(name, _, meta)| (name.to_string(), meta.file.clone()))
        .collect();
    let packages: Vec<String> = session
        .module_index
        .iter()
        .filter(|(_, kind, _)| *kind == EntryKind::Package)
        .map(|(name, _, _)| name.to_string())
        .collect();
    let interfaces: Vec<String> = session
        .module_index
        .iter()
        .filter(|(_, kind, _)| *kind == EntryKind::Interface)
        .map(|(name, _, _)| name.to_string())
        .collect();

    // ── Graf dependency: setiap module → module yang diinstansiasinya.
    // Dibangun dari IrDesign (sub_instances), deduplikasi per (parent, child)
    // dengan hitungan instance. Root = module top. ──
    let mut deps = build_dep_graph(&ir_design);
    // Taruh module top di paling atas agar mudah ditemukan.
    if let Some(pos) = deps.iter().position(|d| d.module == ir_design.top.name.as_str()) {
        let root = deps.remove(pos);
        deps.insert(0, root);
    }

    // ── Reference counts: module → berapa kali di-instansiasi di seluruh
    // design (termasuk top). Precompute sekali di sini supaya Code Lens editor
    // tidak perlu meng-iterasi design setiap frame. ──
    let ref_counts = build_ref_counts(&ir_design);

    // ── Signal info: nama signal → (tipe, lebar bit) untuk Hover tooltip
    // editor. Dibangun dari SEMUA module (top + submodule) — signal bisa
    // berada di file mana pun yang sedang dibuka. Precompute sekali di sini.
    // ──
    let mut signal_info: HashMap<String, (String, usize)> = HashMap::new();
    {
        let mut add_mod = |m: &crate::ir::IrModule| {
            for s in &m.signals {
                signal_info.entry(s.name.to_string()).or_insert_with(|| {
                    (signal_type_str(s.kind.clone()).to_string(), s.width)
                });
            }
        };
        add_mod(&ir_design.top);
        for m in ir_design.modules.values() {
            add_mod(m);
        }
    }

    // ── Symbol → file asal (module/interface/package) untuk Go To Definition
    // (Ctrl+Click). Precompute dari module_index sekali saat compile. ──
    let mut symbol_files: HashMap<String, PathBuf> = HashMap::new();
    for (name, kind, meta) in session.module_index.iter() {
        if matches!(kind, EntryKind::Module | EntryKind::Package | EntryKind::Interface) {
            symbol_files.insert(name.to_string(), meta.file.clone());
        }
    }

    Ok((
        CompileInfo {
            success: true,
            modules,
            packages,
            interfaces,
            total_time_ms: start.elapsed().as_secs_f64() * 1000.0,
            module_files,
            deps,
            ref_counts,
            signal_info,
            symbol_files,
        },
        ir_design,
    ))
}

/// Tipe display signal dari `SignalKind` (untuk Hover tooltip editor).
fn signal_type_str(kind: crate::ir::SignalKind) -> &'static str {
    match kind {
        crate::ir::SignalKind::Wire => "wire",
        crate::ir::SignalKind::Reg => "reg",
        crate::ir::SignalKind::Logic => "logic",
        crate::ir::SignalKind::Input => "input",
        crate::ir::SignalKind::Output => "output",
        crate::ir::SignalKind::Inout => "inout",
    }
}

/// Hitung jumlah instansiasi per module name di seluruh design (top + semua
/// module lain). Dipakai Code Lens — dihitung sekali saat compile.
pub fn build_ref_counts(design: &IrDesign) -> std::collections::HashMap<String, usize> {
    let mut refs: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut count_insts = |module: &crate::ir::IrModule| {
        for inst in &module.sub_instances {
            let n = inst.module_name.to_string();
            *refs.entry(n).or_insert(0) += 1;
        }
    };
    count_insts(&design.top);
    for m in design.modules.values() {
        // Guard: jangan hitung dua kali bila top module juga ada di design.modules.
        if m.name != design.top.name {
            count_insts(m);
        }
    }
    refs
}

/// Bangun graf dependency module → (child_module, count) dari design ter-elaborasi.
/// Hanya module yang punya instance yang dimasukkan; count = jumlah instance.
pub fn build_dep_graph(design: &IrDesign) -> Vec<DepRow> {
    let mut rows: Vec<DepRow> = Vec::new();
    let top_name = design.top.name.to_string();

    // Kumpulkan semua module (top + yang ada di design.modules)
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    seen.insert(top_name.clone());
    for name in design.modules.keys() {
        seen.insert(name.to_string());
    }

    let mut module_list: Vec<String> = seen.into_iter().collect();
    module_list.sort();

    for name in module_list {
        let module = if name == top_name {
            &design.top
        } else {
            match design.modules.get(&crate::Symbol::intern(&name)) {
                Some(m) => m,
                None => continue,
            }
        };
        if module.sub_instances.is_empty() {
            continue;
        }
        let mut children: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for inst in &module.sub_instances {
            let child = inst.module_name.to_string();
            *children.entry(child).or_insert(0) += 1;
        }
        let mut children: Vec<(String, usize)> = children.into_iter().collect();
        children.sort_by(|a, b| a.0.cmp(&b.0));
        rows.push(DepRow {
            module: name,
            children,
        });
    }
    rows
}

/// Jalankan perintah shell di worker thread; stdout/stderr di-stream baris per
/// baris ke channel (`TermOutput`), dan `TermExit(code)` dikirim saat selesai.
/// `cwd` = direktori kerja (project root). Aman dipanggil berkali-kali — tiap
/// pemanggilan membuat child process sendiri.
pub fn spawn_term(tx: Sender<GuiEvent>, cmd: String, cwd: Option<PathBuf>) {
    std::thread::spawn(move || {
        let mut child = match std::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .current_dir(cwd.unwrap_or_else(std::env::temp_dir))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(GuiEvent::TermOutput(format!("error: {}", e), true));
                let _ = tx.send(GuiEvent::TermExit(1));
                return;
            }
        };

        // Pipe stdout & stderr secara bersamaan ke channel. `tx` asli
        // dipakai untuk TermExit setelah reader selesai (jangan dipindah ke
        // thread reader — akan jadi use-after-move).
        let out = child.stdout.take();
        let err = child.stderr.take();
        let (tx_out, tx_err) = (tx.clone(), tx.clone());
        if let Some(out) = out {
            std::thread::spawn(move || {
                use std::io::BufRead;
                for line in std::io::BufReader::new(out).lines() {
                    if let Ok(l) = line {
                        let _ = tx_out.send(GuiEvent::TermOutput(l, false));
                    }
                }
            });
        }
        if let Some(err) = err {
            std::thread::spawn(move || {
                use std::io::BufRead;
                for line in std::io::BufReader::new(err).lines() {
                    if let Ok(l) = line {
                        let _ = tx_err.send(GuiEvent::TermOutput(l, true));
                    }
                }
            });
        }
        let code = child.wait().map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
        let _ = tx.send(GuiEvent::TermExit(code));
    });
}

/// Jalankan simulasi di worker thread. `cancel` adalah flag bersama dengan GUI:
/// jika di-set true (tombol Stop), simulasi berhenti lebih awal.
pub fn spawn_sim(
    tx: Sender<GuiEvent>,
    design: IrDesign,
    max_time: u64,
    cancel: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let result = run_simulation(&design, max_time, cancel);
        let _ = tx.send(GuiEvent::SimDone(result));
    });
}

/// Jalankan simulasi pada design ter-elaborasi. Mengembalikan nilai akhir signal
/// plus trace waveform (ditangkap via VCD temporer → di-parse).
pub fn run_simulation(
    design: &IrDesign,
    max_time: u64,
    cancel: Arc<AtomicBool>,
) -> Result<SimInfo, String> {
    let start = std::time::Instant::now();
    let mut engine = SimulationEngine::new(design.clone(), max_time);
    engine.cancel_flag = Some(cancel.clone());

    // ── Waveform capture: VCD temporer yang ditulis engine per time step,
    // lalu di-parse menjadi trace transisi untuk Waveform viewer. File dihapus
    // setelah dibaca — tidak ada artefak tersisa. Gagal VCD ≠ gagal sim. ──
    let vcd_path = std::env::temp_dir().join(format!(
        "maria_gui_{}_{}.vcd",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let vcd_path_str = vcd_path.to_string_lossy().to_string();
    match crate::waveform::vcd::VcdWriter::new(&vcd_path_str, design) {
        Ok(mut vcd) => {
            // Batasi ukuran dump (anti-bloat untuk desain besar).
            vcd.max_dump_size = Some(256 * 1024 * 1024);
            engine.set_vcd(vcd);
        }
        Err(e) => {
            eprintln!("waveform capture skipped: {}", e);
        }
    }

    let run_result = engine.run();

    // ── Baca VCD → trace waveform, lalu bersihkan file temporer. Cleanup
    // dijalankan di SEMUA jalur (sukses, error sim, maupun Stop) supaya file
    // .vcd tidak menumpuk di /tmp. ──
    let waveform = if run_result.is_ok() && !engine.is_cancelled() {
        std::fs::read_to_string(&vcd_path)
            .map(|text| parse_vcd(&text))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let _ = std::fs::remove_file(&vcd_path);

    run_result.map_err(|e| e.to_string())?;
    if engine.is_cancelled() {
        return Err("Simulasi dihentikan (Stop)".into());
    }

    let signals: Vec<SignalRow> = engine
        .design
        .top
        .signals
        .iter()
        .enumerate()
        .map(|(id, sig)| {
            let lv = engine.state.read_signal(id);
            SignalRow {
                name: sig.name.to_string(),
                width: lv.width,
                value: logicvec_to_hex(lv),
                kind: format!("{:?}", sig.kind),
            }
        })
        .collect();

    let counters = &engine.sim_perf.counters;

    // ── Coverage: ringkasan dari coverage_stats() + detail covergroup ──
    let cov = engine.coverage_stats();
    let mut covergroups: Vec<CovergroupRow> = Vec::new();
    for cg in &engine.design.covergroups {
        let mut total = 0u64;
        let mut hits = 0u64;
        for cp in &cg.coverpoints {
            let key = crate::Symbol::intern(&format!("{}.{}", cg.name, cp.name));
            total += engine.cover_total.get(&key).copied().unwrap_or(0);
            hits += engine.cover_hits.get(&key).copied().unwrap_or(0);
        }
        for cross in &cg.crosses {
            let key = crate::Symbol::intern(&format!("{}.{}", cg.name, cross.name));
            total += engine.cover_total.get(&key).copied().unwrap_or(0);
            hits += engine.cover_hits.get(&key).copied().unwrap_or(0);
        }
        covergroups.push(CovergroupRow {
            name: cg.name.to_string(),
            total,
            hits,
        });
    }
    let coverage = CoverageInfo {
        line_items: cov.get("line_items").copied().unwrap_or(0.0) as u64,
        line_hits: cov.get("line_total_hits").copied().unwrap_or(0.0) as u64,
        toggle_signals: cov.get("toggle_signals").copied().unwrap_or(0.0) as u64,
        toggle_transitions: cov.get("toggle_transitions").copied().unwrap_or(0.0) as u64,
        branch_total: cov.get("branch_total").copied().unwrap_or(0.0) as u64,
        branch_covered: cov.get("branch_covered").copied().unwrap_or(0.0) as u64,
        branch_percent: cov.get("branch_percent").copied().unwrap_or(0.0),
        fsm_signals: cov.get("fsm_signals").copied().unwrap_or(0.0) as u64,
        fsm_states: cov.get("fsm_states").copied().unwrap_or(0.0) as u64,
        covergroups,
    };

    Ok(SimInfo {
        signals,
        cycles: engine.current_time,
        sim_time_ms: start.elapsed().as_secs_f64() * 1000.0,
        waveform,
        delta_cycles: counters.delta_cycles,
        events_processed: counters.events_processed,
        processes_evaluated: counters.processes_evaluated,
        nba_commits: counters.nba_commits,
        sensitive_triggers: counters.sensitive_triggers,
        events_per_delta: engine.sim_perf.events_per_delta(),
        coverage,
    })
}

/// Parse konten VCD (format yang ditulis `VcdWriter`) menjadi trace transisi
/// per signal. Nilai VCD: `$var wire <w> <code> <name> ... $end`, `#<time>`,
/// `<v><code>` (1-bit) dan `b<value> <code>` (multi-bit). Nilai bertahan sampai
/// transisi berikut — sudah cukup untuk rendering waveform.
pub fn parse_vcd(text: &str) -> Vec<WaveformSignal> {
    let mut vars: HashMap<String, (String, usize)> = HashMap::new();
    let mut traces: HashMap<String, Vec<(u64, String)>> = HashMap::new();
    let mut cur_time: u64 = 0;
    // Scope stack: VcdWriter menulis `$scope module <top> $end` lalu scope
    // hierarkis per sinyal. Scope pertama = modul top (dilewati — nama sinyal
    // top-level adalah nama bare); scope berikutnya digabung utk nama penuh
    // (mis. "u1.q") agar konsisten dgn tab Signals.
    let mut scope: Vec<String> = Vec::new();
    let mut seen_top_scope = false;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("$scope ") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 2 {
                if !seen_top_scope {
                    seen_top_scope = true; // modul top — jangan jadi prefix nama
                } else {
                    scope.push(parts[1].to_string());
                }
            }
            continue;
        }
        if line.starts_with("$upscope") {
            scope.pop();
            continue;
        }
        if let Some(rest) = line.strip_prefix("$var ") {
            // "wire <w> <code> <name> [range] $end" (range hilang utk width 1)
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 4 {
                if let Ok(width) = parts[1].parse::<usize>() {
                    let code = parts[2].to_string();
                    let bare = parts[3].to_string();
                    let name = if scope.is_empty() {
                        bare
                    } else {
                        format!("{}.{}", scope.join("."), bare)
                    };
                    vars.insert(code.clone(), (name, width));
                    traces.entry(code).or_default();
                }
            }
            continue;
        }
        if let Some(t) = line.strip_prefix('#') {
            cur_time = t.trim().parse().unwrap_or(cur_time);
            continue;
        }
        if let Some(b) = line.strip_prefix('b') {
            // b<value> <code>
            if let Some((val, code)) = b.split_once(char::is_whitespace) {
                let code = code.trim();
                if !code.is_empty() {
                    traces
                        .entry(code.to_string())
                        .or_default()
                        .push((cur_time, val.trim().to_string()));
                }
            }
            continue;
        }
        // 1-bit: "0s0" / "1s0" / "xs0" / "zs0"
        if line.len() >= 2 {
            let (val, code) = line.split_at(1);
            if matches!(val, "0" | "1" | "x" | "z" | "X" | "Z") && !code.trim().is_empty() {
                traces
                    .entry(code.trim().to_string())
                    .or_default()
                    .push((cur_time, val.to_string()));
            }
        }
    }

    let mut out: Vec<WaveformSignal> = vars
        .into_iter()
        .filter_map(|(code, (name, width))| {
            let mut trace = traces.remove(&code).unwrap_or_default();
            trace.sort_by_key(|(t, _)| *t);
            // Dedupe nilai berurutan yang sama (pertahankan waktu terakhir)
            let mut dedup: Vec<(u64, String)> = Vec::with_capacity(trace.len());
            for (t, v) in trace {
                match dedup.last_mut() {
                    Some((lt, lv)) if *lv == v => *lt = t,
                    _ => dedup.push((t, v)),
                }
            }
            Some(WaveformSignal {
                name,
                width,
                trace: dedup,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Konversi LogicVec ke hex string (mendukung X/Z dan width > 64-bit).
pub fn logicvec_to_hex(lv: &LogicVec) -> String {
    if lv.width == 0 {
        return "0".into();
    }
    let all_x = lv.bits.iter().all(|b| *b == LogicVal::X);
    let all_z = lv.bits.iter().all(|b| *b == LogicVal::Z);
    if all_x {
        return format!("'x{}", lv.width);
    }
    if all_z {
        return format!("'z{}", lv.width);
    }
    let nibbles = (lv.width + 3) / 4;
    let mut hex = String::with_capacity(nibbles);
    for nib in (0..nibbles).rev() {
        let mut val = 0u8;
        let mut has_x = false;
        let mut has_z = false;
        for bit in 0..4 {
            let idx = nib * 4 + bit;
            if idx < lv.width {
                match lv.bits[idx] {
                    LogicVal::One => val |= 1 << bit,
                    LogicVal::X => has_x = true,
                    LogicVal::Z => has_z = true,
                    LogicVal::Zero => {}
                }
            }
        }
        if has_x {
            hex.push('x');
        } else if has_z {
            hex.push('z');
        } else {
            hex.push(std::char::from_digit(val as u32, 16).unwrap_or('0'));
        }
    }
    let trimmed = hex.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".into()
    } else {
        trimmed.to_string()
    }
}
