//! `msim` — Simulator.
//!
//! Wrapper ringkas pipeline simulasi: compile → elaborate → run engine →
//! VCD/FST + ringkasan assertion/coverage.

use crate::elaboration::elaborator::ElaborateMode;
use crate::error::SimError;
use crate::simulator::SimulationEngine;
use std::time::Instant;
use crate::tools::{open_elaborated, section, kv};

/// Opsi msim.
pub struct SimArgs<'a> {
    pub files: &'a [String],
    pub incdirs: &'a [String],
    pub defines: &'a [String],
    pub top: Option<&'a str>,
    pub max_time: u64,
    pub output: Option<&'a str>,
    pub fst: bool,
    pub assertions: bool,
    pub coverage: bool,
}

/// Jalankan msim.
pub fn run(args: &SimArgs) -> Result<(), SimError> {
    // Use StrictSimulation mode for simulation tools (Rule 10)
    let (session, _design, ir) = open_elaborated(args.files, args.incdirs, args.defines, args.top, ElaborateMode::StrictSimulation)?;
    let top_name = ir.top.name.as_str();

    let mut engine = SimulationEngine::new(ir, args.max_time);

    // VCD output
    let vcd_path = args
        .output
        .map(|o| {
            if o.ends_with(".vcd") {
                o.to_string()
            } else {
                format!("{}.vcd", o)
            }
        })
        .unwrap_or_else(|| format!("{}.vcd", top_name));
    let vcd = crate::waveform::VcdWriter::new(&vcd_path, &engine.design)
        .map_err(|e| SimError::with_diag(crate::diagnostics::DiagCode::WaveformError, e))?;
    engine.set_vcd(vcd);

    // FST output
    if args.fst {
        let fst_path = format!("{}.fst", top_name);
        match crate::waveform::FstWaveWriter::new(&fst_path, &engine.design) {
            Ok(fst) => {
                engine.set_fst(fst);
            }
            Err(e) => {
                let diag = crate::diagnostics::Diagnostic::warning(
                    crate::diagnostics::DiagCode::WaveformError,
                    format!("FST: cannot create '{}': {}", fst_path, e),
                );
                let mut emitter = crate::diagnostics::TerminalEmitter::new();
                let _ = emitter.emit(&diag);
            }
        }
    }

    section("Simulation");
    let sim_start = Instant::now();
    engine.run()?;
    let sim_ms = sim_start.elapsed().as_millis() as u64;

    kv("end time", format!("#{}", engine.state.time));
    kv("sim time", format!("{} ms", sim_ms));
    kv("VCD", vcd_path);

    // Diagnostics runtime
    let diags = engine.flush_diagnostics();
    let assertion_errors = diags
        .iter()
        .filter(|d| d.code == crate::diagnostics::DiagCode::AssertionFailed)
        .count();
    if !diags.is_empty() {
        let mut emitter = crate::diagnostics::TerminalEmitter::new();
        for d in &diags {
            let _ = emitter.emit(d);
        }
    }

    if args.assertions {
        section("Assertion Summary");
        kv("failed", assertion_errors);
        kv("passed", "—");
    }

    if args.coverage {
        section("Coverage Summary");
        let stats = engine.coverage_stats();
        for (k, v) in stats.iter().collect::<Vec<_>>().iter() {
            kv(k, v);
        }
    }

    if args.assertions && assertion_errors > 0 {
        return Err(SimError::with_diag(
            crate::diagnostics::DiagCode::AssertionFailed,
            format!("{} assertion(s) gagal", assertion_errors),
        ));
    }
    // F15: $fatal menghentikan sim dengan kegagalan → exit code non-zero
    // (pola sama dengan assertion failed).
    if engine.sev_fatal_count > 0 {
        return Err(SimError::with_diag(
            crate::diagnostics::DiagCode::AssertionFailed,
            format!("$fatal: simulasi dihentikan ({} fatal)", engine.sev_fatal_count),
        ));
    }
    Ok(())
}
