//! `synth` (alias `msynth`) — Maria Synthesis (SYNTHESIS.md §15).
//!
//! RTL → SIR → netlist gate-level:
//! - `--check-only`: analisis sintesizability (SYN-1..9) tanpa netlist.
//! - `--dump-sir`: dump SIR node-based (fase RTL→SIR, `maria-sir`).
//! - default: SYN check + inferensi netlist pra-map + emit `.mvnet` + report
//!   utilisasi (estimasi S1).

use std::path::PathBuf;

use maria_core::error::SimError;
use maria_elaboration::elaborator::ElaborateMode;
use maria_synth::DeviceKind;
use maria_synth::prelude::*;
use crate::{section, kv};

/// Opsi `synth`.
pub struct SynthArgs<'a> {
    pub targets: &'a [String],
    pub incdirs: &'a [String],
    pub defines: &'a [String],
    pub top: Option<&'a str>,
    /// Output prefix (default: nama top). Menghasilkan `<prefix>.mvnet` dst.
    pub output: Option<String>,
    /// Hanya SYN subset check.
    pub check_only: bool,
    /// Device target: "fpga-x7" (default) atau "generic".
    pub device: String,
    /// Preset pipeline: generic | fpga | asic | custom.
    pub preset: String,
    /// Emisi `.mvnet`.
    pub emit_mvnet: bool,
    /// Dump SIR (node-based) ke stdout.
    pub dump_sir: bool,
    /// Dump SIR setelah pass optimizer.
    pub dump_sir_opt: bool,
    /// Dump netlist generik (Verilog + .mvnet) ke stdout.
    pub dump_netlist: bool,
    /// Emisi netlist ke file: <prefix>.netlist.v + .json + .mvnet.
    pub emit_netlist: bool,
    /// Tech mapping (phase 4): LUT cut + AIG dekomposisi + carry chain →
    /// <prefix>.tech.v/.json/.mvnet + report LUT/CARRY4/FF.
    pub tech_map: bool,
    /// Tulis report utilisasi ke file (opsional; tanpa ini ke stdout).
    pub report_util: Option<String>,
    pub quiet: bool,
}

/// Jalankan `synth`.
pub fn run(args: &SynthArgs) -> Result<(), SimError> {
    let (session, _design, ir) = crate::open_elaborated(
        args.targets,
        args.incdirs,
        args.defines,
        args.top,
        ElaborateMode::AnalysisRecovery,
    )?;
    let top_name = ir.top.name.as_str().to_string();

    // ── Device ──
    let device = match args.device.as_str() {
        "fpga-x7" => DeviceKind::FpgaX7,
        "generic" => DeviceKind::Generic,
        other => {
            return Err(SimError::with_diag(
                maria_core::diagnostics::DiagCode::InvalidSyntax,
                format!(
                    "device '{}' tidak dikenal — pakai 'fpga-x7' (default) atau 'generic' (ASIC menyusul S4)",
                    other
                ),
            ));
        }
    };

    // ── SYN check ──
    let check = synth_check(&ir);
    let syn_report = render_syn_report(&check);
    if !args.quiet {
        print!("{}", syn_report);
    }

    // ── SIR: lowering + optimizer (Phase 2, SYNTHESIS.md §4/§6) ──
    let need_sir = !args.check_only
        || args.dump_sir
        || args.dump_sir_opt
        || args.dump_netlist
        || args.emit_netlist
        || args.tech_map;
    if need_sir {
        let sir = maria_sir::lower(&ir);
        if args.dump_sir && !args.quiet {
            section("SIR (sebelum optimasi)");
            print!("{}", maria_sir::render_sir(&sir.module));
            if !sir.skipped.is_empty() {
                println!(
                    "  [skipped {} konstruk yang belum didukung SIR fase 1]",
                    sir.skipped.len()
                );
                for s in &sir.skipped {
                    println!("    - {s}");
                }
            }
            println!();
        }
        // Pass manager + preset.
        let mut pipeline = maria_synth::SynthPipeline::with_preset(&args.preset)?;
        let (sir_opt, results) = pipeline.run(sir.module)?;
        if args.dump_sir_opt && !args.quiet {
            section("SIR (setelah optimasi)");
            print!("{}", maria_sir::render_sir(&sir_opt));
            println!();
        }
        section(&format!("Optimization (preset: {})", &args.preset));
        for r in &results {
            kv(
                r.name,
                format!("{} → {} node ({} rewrite)", r.nodes_before, r.nodes_after, r.changed),
            );
        }

        // ── Netlist: SIR → generic netlist (Phase 3, SYNTHESIS.md §11/§13) ──
        if args.dump_netlist || args.emit_netlist {
            let nl = maria_netlist::lower_module(&sir_opt);
            section("Netlist (generic, SIR → gate-level)");
            print!("{}", maria_netlist::emit_summary(&nl));
            let check = maria_netlist::verify_dag(&nl);
            if !check.ok {
                if !check.double_drivers.is_empty() {
                    println!("  ⚠ double driver: {}", check.double_drivers.join(", "));
                }
                if !check.floating.is_empty() {
                    println!("  ⚠ floating net: {}", check.floating.join(", "));
                }
            }
            if args.dump_netlist && !args.quiet {
                println!("\n── netlist.v ──");
                print!("{}", maria_netlist::emit_verilog(&nl));
                println!("\n── .mvnet ──");
                print!("{}", maria_netlist::emit_mvnet(&nl));
            }
            if args.emit_netlist {
                let prefix = args
                    .output
                    .clone()
                    .unwrap_or_else(|| top_name.clone());
                let v = maria_netlist::emit_verilog(&nl);
                let json = maria_netlist::emit_json(&nl);
                let mvnet = maria_netlist::emit_mvnet(&nl);
                for (suffix, content) in [
                    ("netlist.v", v),
                    ("netlist.json", json),
                    ("netlist.mvnet", mvnet),
                ] {
                    let path = PathBuf::from(format!("{}.{}", prefix, suffix));
                    std::fs::write(&path, content).map_err(|e| {
                        SimError::with_diag(
                            maria_core::diagnostics::DiagCode::IoError,
                            format!("{}: {}", path.display(), e),
                        )
                    })?;
                    if !args.quiet {
                        println!("  netlist → {}", path.display());
                    }
                }
            }
        }

        // ── Tech mapping (Phase 4, SYNTHESIS.md §5/§12) ──
        // LUT cut (≤K input, init nyata) + AIG dekomposisi (>K input) +
        // carry chain (CARRY4) + FF per-bit. Emisi <prefix>.tech.v/.json/.mvnet
        // + report LUT/CARRY4/FF. `--device` memilih arsitektur (generic/fpga-x7).
        if args.tech_map {
            let arch = maria_tech::arch_for(match args.device.as_str() {
                "fpga-x7" => "fpga",
                other => other,
            })
            .expect("device generic/fpga punya back-end");
            let res = maria_synth::tech_map(&sir_opt, arch.as_ref());
            section(&format!(
                "Tech Mapping (phase 4 — {})",
                arch.name()
            ));
            for s in &res.skipped {
                println!("  [skipped] {s}");
            }
            kv("LUT", res.lut_count.to_string());
            kv("CARRY4", res.carry4_count.to_string());
            kv("FF", res.ff_count.to_string());
            let dag = maria_netlist::verify_dag(&res.netlist);
            kv(
                "DAG",
                if dag.ok { "ok" } else { "violation!" },
            );
            let prefix = args
                .output
                .clone()
                .unwrap_or_else(|| top_name.clone());
            for (suffix, content) in [
                ("tech.v", maria_netlist::emit_verilog(&res.netlist)),
                ("tech.json", maria_netlist::emit_json(&res.netlist)),
                ("tech.mvnet", maria_netlist::emit_mvnet(&res.netlist)),
            ] {
                let path = PathBuf::from(format!("{}.{}", prefix, suffix));
                std::fs::write(&path, content).map_err(|e| {
                    SimError::with_diag(
                        maria_core::diagnostics::DiagCode::IoError,
                        format!("{}: {}", path.display(), e),
                    )
                })?;
                if !args.quiet {
                    println!("  tech netlist → {}", path.display());
                }
            }
        }
    }

    if args.check_only {
        // `--check-only`: berhenti di sini; exit non-zero bila ada SYN error.
        if check.error_count() > 0 {
            let first = maria_synth::report::first_error(&check).unwrap_or_default();
            return Err(SimError::with_diag(
                maria_core::diagnostics::DiagCode::InvalidSyntax,
                format!("synthesis check FAILED: {} error(s) — {}", check.error_count(), first),
            ));
        }
        println!("\n✅ synthesis check OK — design sintesizable (skor {:.1}/100)", check.overall_score());
        return Ok(());
    }

    // ── Inferensi netlist ──
    let opts = maria_synth::SynthOpts { device };
    let out = synthesize(&ir, &opts);
    let nl = &out.netlist;

    section("Synthesis Result");
    print!("{}", emit_summary(nl));
    kv("check score", format!("{:.1}/100", check.overall_score()));
    kv("elab time", format!("{} ms", session.timing.elab_ms));

    // ── Output ──
    let prefix = args
        .output
        .clone()
        .unwrap_or_else(|| format!("{}", top_name));
    if args.emit_mvnet {
        let mvnet = emit_mvnet(nl, maria_synth::VERSION);
        let path = PathBuf::from(format!("{}.mvnet", prefix));
        std::fs::write(&path, mvnet).map_err(|e| {
            SimError::with_diag(
                maria_core::diagnostics::DiagCode::IoError,
                format!("{}: {}", path.display(), e),
            )
        })?;
        if !args.quiet {
            println!("  .mvnet → {}", path.display());
        }
    }

    // ── Report utilisasi ──
    let cap = match nl.device {
        DeviceKind::FpgaX7 => maria_synth::DeviceCapacity::fpga_x7(),
        DeviceKind::Generic => maria_synth::DeviceCapacity::generic(),
    };
    let util = render_util_report(nl, &cap);
    if let Some(path_str) = &args.report_util {
        let path = PathBuf::from(path_str);
        std::fs::write(&path, &util).map_err(|e| {
            SimError::with_diag(
                maria_core::diagnostics::DiagCode::IoError,
                format!("{}: {}", path.display(), e),
            )
        })?;
        if !args.quiet {
            println!("  util report → {}", path.display());
        }
    } else if !args.quiet {
        print!("{}", util);
    }

    Ok(())
}
