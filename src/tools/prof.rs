//! `mprof` — Performance Profiler.
//!
//! Ukur waktu tiap fase pipeline (discovery → preprocess → parse → index →
//! elab → simulation) dan sorot bottleneck.

use crate::elaboration::elaborator::ElaborateMode;
use crate::error::SimError;
use crate::frontend::CompileSession;
use crate::simulator::SimulationEngine;
use crate::tools::{collect_targets, make_session_config_with_mv, section, kv};
use std::time::Instant;

/// Opsi mprof.
pub struct ProfArgs<'a> {
    pub targets: &'a [String],
    pub incdirs: &'a [String],
    pub defines: &'a [String],
    pub top: Option<&'a str>,
    pub max_time: u64,
}

/// Satu baris timing fase.
struct PhaseTime {
    name: &'static str,
    ms: u64,
}

/// Jalankan mprof.
pub fn run(args: &ProfArgs) -> Result<(), SimError> {
    let files = collect_targets(args.targets)?;
    // F10: `.mv` di-transpile ke buffer inline (svh+sv) agar mprof bisa
    // memprofil pipeline Maria HDL tanpa menulis file ke disk.
    let cfg = make_session_config_with_mv(
        files,
        args.incdirs,
        args.defines,
        args.top.map(|s| s.to_string()),
    )?;
    let mut session = CompileSession::new(cfg);

    let compile_start = Instant::now();
    // Use StrictSimulation mode for simulation profiling
    let (_design, ir, _len) = session.compile_and_elaborate_with_mode(args.top, ElaborateMode::StrictSimulation)?;
    let compile_ms = compile_start.elapsed().as_millis() as u64;

    // Simulation
    let mut engine = SimulationEngine::new(ir, args.max_time);
    let sim_start = Instant::now();
    engine.run()?;
    let sim_ms = sim_start.elapsed().as_millis() as u64;

    let t = &session.timing;
    let mut phases: Vec<PhaseTime> = vec![
        PhaseTime { name: "Discovery", ms: t.discovery_ms },
        PhaseTime { name: "Preprocess", ms: t.preprocess_ms },
        PhaseTime { name: "Parse", ms: t.parse_ms },
        PhaseTime { name: "Index", ms: t.index_ms },
        PhaseTime { name: "Elaboration", ms: t.elab_ms },
        PhaseTime { name: "Simulation", ms: sim_ms },
    ];

    // Drop fase yang tidak terukur (0)
    phases.retain(|p| p.ms > 0);

    let total: u64 = phases.iter().map(|p| p.ms).sum::<u64>().max(1);

    section("Pipeline Profile");
    println!("  {:<14} {:>10} {:>8}", "Phase", "Time (ms)", "%");
    println!("  {}────────────{}──────────{}────────", "─", "─", "─");
    for p in &phases {
        let bar_len = ((p.ms as f64 / total as f64) * 40.0) as usize;
        let bar = "█".repeat(bar_len);
        println!(
            "  {:<14} {:>10} {:>7.1}%  {}",
            p.name,
            p.ms,
            (p.ms as f64 / total as f64) * 100.0,
            bar
        );
    }
    println!("  {}────────────{}──────────{}────────", "─", "─", "─");
    println!("  {:<14} {:>10}", "Total", total);

    // Bottleneck
    if let Some(bn) = phases.iter().max_by_key(|p| p.ms) {
        section("Bottleneck");
        kv("phase", bn.name);
        kv("time", format!("{} ms ({:.1}%)", bn.ms, (bn.ms as f64 / total as f64) * 100.0));
        kv("hint", bottleneck_hint(bn.name));
    }

    kv("compile (total)", format!("{} ms", compile_ms));
    kv("files", session.config.sources.len());
    kv("processed", t.processed_files);
    kv("cached", t.cached_files);

    Ok(())
}

fn bottleneck_hint(name: &str) -> &'static str {
    match name {
        "Discovery" => "file scan terlalu banyak direktori; pakai file list (-f)",
        "Preprocess" => "`include / `define berat; pakai MICD (cache preprocess)",
        "Parse" => "banyak file besar; aktifkan --fast (FastLexer + paralel)",
        "Index" => "index module — umumnya cepat",
        "Elaboration" => "generate/parameter expansion berat; pakai --lazy",
        "Simulation" => "kecilkan max_time atau pakai --packed / --jit-body",
        _ => "—",
    }
}
