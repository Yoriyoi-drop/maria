//! Backend GUI — memanggil API library `maria` langsung (tanpa IPC).
//!
//! Compile/elaborate via `CompileSession`, simulasi via `SimulationEngine`.
//! Operasi berat dijalankan di worker thread; hasil dikirim melalui channel
//! sehingga UI tetap responsif.

use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use crate::frontend::compile_session::{CompileSession, SessionConfig};
use crate::frontend::module_index::EntryKind;
use crate::ir::{IrDesign, LogicVal, LogicVec};
use crate::simulator::SimulationEngine;

use super::state::{CompileInfo, FileNode, GuiEvent, SimInfo, SignalRow};

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

/// Compile + elaborate semua file. Mengembalikan info + design ter-elaborasi.
pub fn compile_project(paths: &[PathBuf]) -> Result<(CompileInfo, IrDesign), String> {
    let start = std::time::Instant::now();

    let mut config = SessionConfig::default();
    for p in paths {
        config.sources.push(p.clone());
    }
    config.use_lazy_elab = true;
    config.auto_incdirs = true;

    let mut session = CompileSession::new(config);
    let (_design, ir_design, _idx) = session
        .compile_and_elaborate(None)
        .map_err(|e| e.to_string())?;

    let modules: Vec<String> = session
        .module_index
        .iter()
        .filter(|(_, kind, _)| *kind == EntryKind::Module)
        .map(|(name, _, _)| name.to_string())
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

    Ok((
        CompileInfo {
            success: true,
            modules,
            packages,
            interfaces,
            total_time_ms: start.elapsed().as_secs_f64() * 1000.0,
        },
        ir_design,
    ))
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

/// Jalankan simulasi pada design ter-elaborasi. Mengembalikan nilai akhir signal.
pub fn run_simulation(
    design: &IrDesign,
    max_time: u64,
    cancel: Arc<AtomicBool>,
) -> Result<SimInfo, String> {
    let start = std::time::Instant::now();
    let mut engine = SimulationEngine::new(design.clone(), max_time);
    engine.cancel_flag = Some(cancel);
    engine.run().map_err(|e| e.to_string())?;
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

    Ok(SimInfo {
        signals,
        cycles: engine.current_time,
        sim_time_ms: start.elapsed().as_secs_f64() * 1000.0,
    })
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
