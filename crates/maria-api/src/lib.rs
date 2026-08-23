//! Maria API — lapisan public API RTL Simulator untuk SystemVerilog.
//!
//! Hasil akhir migrasi monorepo: `src/` di package `maria` hanya berisi
//! `main.rs` + `cli.rs` (binary-only). Seluruh logika pindah ke crates/,
//! dan API publik (compile_str/simulate_str/… + re-export maria_*) hidup
//! di crate `maria-api` ini. `maria::*` di main.rs kini = `maria_api::*`.

// Allow large Result Err variant for SimError (intentional — Diagnostic contains spans/files)
#![allow(clippy::result_large_err)]

// ── VPI (Verilog Procedural Interface) — pindah ke maria-simulator (crates/) ──
// ── LSP (Language Server Protocol) — pindah ke maria-env (crates/) ──
// `maria::lsp::*` di main.rs tetap valid via re-export.
#[cfg(feature = "lsp")]
pub use maria_env::lsp;


// ── Formal Verification Engine — pindah ke maria-formal (crates/) ──
// BMC + Z3 SMT + assertion checking. `maria::formal::*` di main.rs tetap valid.
#[cfg(feature = "formal")]
pub use maria_formal as formal;

// ── Core Infrastructure — pindah ke workspace crate `maria-core` (crates/) ──
// intern (Symbol/Span), arena, error, diagnostics, config, animasi, dan tipe
// nilai logika (LogicVal/LogicVec) kini hidup di crates/maria-core. Modul
// ini tetap diakses lintas crate via `maria_core::...`; re-export API inti
// ada di bagian bawah lib.rs.

// ── Enterprise Context Architecture (doc/env.md) — GlobalEnv + Context ──
// pindah ke maria-env (crates/) — `maria::env::*` di main.rs tetap valid.
pub use maria_env::env;
use std::sync::atomic::{AtomicUsize, Ordering};

// ── Legacy Modules ──
// ast → maria-ast, ir → maria-ir, parser → maria-parser, elaboration →
// maria-elaboration, compiler → maria-compiler, simulator/waveform/scheduler/
// debugger/vpi → maria-simulator (crates/) — lihat migrasi monorepo.
pub use maria_simulator::{debugger, foreign, pli, scheduler, simulator, vhpi, vpi, waveform};

// ── Maria HDL (.mv) — bahasa baru Maria, transpile ke SystemVerilog (MARIA-HDL.md) ──
// pindah ke maria-mv (crates/) — lihat migrasi monorepo.
pub use maria_mv as mv;

// ── Emulator (EMULATOR.md) — Hardware-Software Emulator; R0: MHIR ──
// `maria::emu::mhir::*` di main.rs tetap valid via re-export.
pub use maria_emu as emu;

// ── New Module Structure ──
// frontend/cache/micd/hir/mir/profiling + scheduler(task cluster) →
// maria-compiler (crates/) — lihat migrasi monorepo.
// scheduler (simulasi cluster: sim_dag/clock_domain/cdc) → maria-simulator.

// ── Plugin System — pindah ke maria-env (crates/) bersama env ──
// (plugin hanya dipakai oleh env/plugins; `maria::plugin` tetap tersedia)
pub use maria_env::plugin;

// ── CLI Tools (tools.md) — pindah ke maria-tools (crates/) ──
// 10 tool terminal (minspect/mlint/melab/msim/mcov/mwave/mfmt/mprof/mcheck/
// mbench) kini di crate maria-tools; `maria::tools::*` di main.rs tetap valid.
pub use maria_tools as tools;

// ── Native GUI (egui) — pindah ke maria-gui (crates/) ──
// `maria::gui::run()` di main.rs + bin/maria_gui.rs tetap valid via re-export.
#[cfg(feature = "gui")]
pub use maria_gui as gui;

pub use maria_core::arena::{BumpArena, TypedArena};
pub use maria_core::error::SimError;
pub use maria_core::diagnostics::{DiagCode, DiagLevel, Diagnostic, DiagSink, RuntimeContext, SourceSnippet};
pub use maria_core::intern::{init_string_table, Span, Symbol};
pub use maria_compiler::frontend::compile_session::{CompileSession, SessionConfig};
pub use maria_compiler::frontend::discovery::FileDiscovery;
use maria_elaboration::ElaborateMode;


use maria_parser::lexer::Lexer;
use maria_parser::preprocessor::Preprocessor;
use maria_parser::Parser;
use std::fs;
use std::path::Path;



/// Compare two ASTs for regression testing. Returns list of structural differences.
pub fn compare_asts(design_a: &maria_ir::IrDesign, design_b: &maria_ir::IrDesign) -> Vec<String> {
    let mut diffs = Vec::new();

    // Compare module count
    if design_a.modules.len() != design_b.modules.len() {
        diffs.push(format!(
            "module count: {} vs {}",
            design_a.modules.len(),
            design_b.modules.len()
        ));
    }

    // Compare signal count
    if design_a.top.signals.len() != design_b.top.signals.len() {
        diffs.push(format!(
            "top signal count: {} vs {}",
            design_a.top.signals.len(),
            design_b.top.signals.len()
        ));
    }

    // Compare process count
    if design_a.top.processes.len() != design_b.top.processes.len() {
        diffs.push(format!(
            "process count: {} vs {}",
            design_a.top.processes.len(),
            design_b.top.processes.len()
        ));
    }

    // Compare each signal info
    for (i, (sa, sb)) in design_a.top.signals.iter().zip(design_b.top.signals.iter()).enumerate() {
        if sa.width != sb.width {
            diffs.push(format!("signal[{}] '{}' width: {} vs {}", i, sa.name, sa.width, sb.width));
        }
        if sa.is_signed != sb.is_signed {
            diffs.push(format!("signal[{}] '{}' signed: {} vs {}", i, sa.name, sa.is_signed, sb.is_signed));
        }
    }

    // Compare class definitions
    if design_a.classes.len() != design_b.classes.len() {
        diffs.push(format!(
            "class count: {} vs {}",
            design_a.classes.len(),
            design_b.classes.len()
        ));
    }

    // Compare covergroups
    if design_a.covergroups.len() != design_b.covergroups.len() {
        diffs.push(format!(
            "covergroup count: {} vs {}",
            design_a.covergroups.len(),
            design_b.covergroups.len()
        ));
    }

    diffs
}

/// Read a .maria project file and return list of .sv file paths
/// Paths in .maria are resolved relative to the .maria file's directory
pub fn read_project_file(path: &str) -> Result<Vec<String>, SimError> {
    let content = fs::read_to_string(path)
        .map_err(|e| SimError::with_diag(DiagCode::InvalidSyntax, format!("cannot read '{}': {}", path, e)))?;
    let base = Path::new(path).parent().unwrap_or(Path::new("."));
    // Section `[...]` di project file .maria ([foreign] untuk library
    // VHPI/PLI/DPI, dan header lain) — header DAN isi section (baris
    // `key = value`) bukan file .sv. Konvensi: daftar file dulu, section di
    // akhir — setelah header section pertama, sisanya di-skip. [foreign]
    // di-parse terpisah oleh read_project_with_foreign.
    let mut in_section = false;
    let mut skipped_templates = 0usize;
    let files: Vec<String> = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter(|l| {
            if l.starts_with('[') && l.ends_with(']') {
                in_section = true;
                return false;
            }
            !in_section
        })
        .filter(|l| {
            let p = base.join(l);
            if maria_core::template::is_template_source(&p) {
                skipped_templates += 1;
                false
            } else {
                true
            }
        })
        .map(|l| {
            let p = base.join(l);
            p.to_string_lossy().to_string()
        })
        .collect();
    if skipped_templates > 0 {
        eprintln!(
            "warning: filelist '{}': melewati {} file template (*.tpl*) — bukan SystemVerilog",
            path, skipped_templates
        );
    }
    if files.is_empty() {
        return Err(SimError::with_diag(
            DiagCode::ModuleNotFound,
            format!("no .sv files listed in '{}'", path),
        ));
    }
    Ok(files)
}

/// Isi file project .maria — daftar file .sv + bagian `[foreign]`
/// (arsitektur masukan user poin 9):
///
/// ```text
/// tb_top.sv
/// rtl/counter.sv
///
/// [foreign]
/// vhpi = ["libvhpi_test.so"]
/// pli  = ["libpli_test.so"]
/// dpi  = ["libdpi_test.so"]
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectFile {
    /// File .sv (path sudah relatif ke direktori .maria).
    pub files: Vec<String>,
    /// Library VHPI (IEEE 1076-2008) dari `[foreign] vhpi = [...]`.
    pub vhpi_libs: Vec<String>,
    /// Library PLI (IEEE 1364) dari `[foreign] pli = [...]`.
    pub pli_libs: Vec<String>,
    /// Library DPI (IEEE 1800 §35) dari `[foreign] dpi = [...]`.
    pub dpi_libs: Vec<String>,
}

/// Baca file project .maria — daftar file .sv + bagian `[foreign]`.
/// Baris non-kosong non-komentar di luar `[foreign]` = file .sv (satu per
/// baris, path relatif ke direktori .maria, pola lama). Bagian `[foreign]`
/// berisi list library per interface (format TOML-like `key = ["a.so", ...]`).
pub fn read_project_with_foreign(path: &str) -> Result<ProjectFile, SimError> {
    let content = fs::read_to_string(path).map_err(|e| {
        SimError::with_diag(DiagCode::InvalidSyntax, format!("cannot read '{}': {}", path, e))
    })?;
    let base = Path::new(path).parent().unwrap_or(Path::new("."));
    let mut proj = ProjectFile::default();
    let mut in_foreign = false;
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_foreign = line.trim_matches(['[', ']']).trim() == "foreign";
            continue;
        }
        if in_foreign {
            // Format: `key = ["lib1.so", "lib2.so"]` (atau tanpa kurung).
            if let Some((key, val)) = line.split_once('=') {
                let key = key.trim();
                let list: Vec<String> = val
                    .trim()
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .split(',') // komentar //
                    .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if list.is_empty() {
                    continue;
                }
                // Path library di-resolve relatif ke direktori .maria (pola
                // sama dengan file .sv) — dlopen butuh path absolut.
                let resolved: Vec<String> = list
                    .iter()
                    .map(|p| {
                        let pbuf = base.join(p);
                        pbuf.to_string_lossy().to_string()
                    })
                    .collect();
                match key {
                    "vhpi" => proj.vhpi_libs.extend(resolved),
                    "pli" => proj.pli_libs.extend(resolved),
                    "dpi" => proj.dpi_libs.extend(resolved),
                    _ => {
                        // Kunci tak dikenal → peringatan via stderr (tidak gagal).
                        eprintln!("warning: [foreign] key '{}' tak dikenal di '{}'", key, path);
                    }
                }
            }
        } else {
            let p = base.join(line);
            proj.files.push(p.to_string_lossy().to_string());
        }
    }
    Ok(proj)
}

/// Compile multiple .sv files into IR design
pub fn compile_files(paths: &[String]) -> Result<maria_ir::IrDesign, SimError> {
    let mut combined = String::new();
    let mut last_timescale = None;
    for path in paths {
        let mut pp = Preprocessor::new();
        let processed = pp.preprocess_file(path)?;
        if pp.timescale.is_some() {
            last_timescale = pp.timescale.clone();
        }
        combined.push_str(&format!("`line 1 \"{}\"\n", path));
        combined.push_str(&processed);
        combined.push('\n');
    }
    let mut result = compile_str(&combined)?;
    if last_timescale.is_some() && result.timescale.is_none() {
        result.timescale = last_timescale;
    }
    Ok(result)
}

/// Compile a SystemVerilog source file and run simulation
pub fn simulate_file(path: &str, max_time: u64) -> Result<(), SimError> {
    let source = fs::read_to_string(path)
        .map_err(|e| SimError::with_diag(DiagCode::InvalidSyntax, format!("cannot read '{}': {}", path, e)))?;
    simulate_str(&source, max_time)
}

/// Compile SystemVerilog source string and run simulation
pub fn simulate_str(source: &str, max_time: u64) -> Result<(), SimError> {
    let design = compile_str(source)?;
    run_simulation(design, max_time)
}

/// Compile SystemVerilog source string into IR
pub fn compile_str(source: &str) -> Result<maria_ir::IrDesign, SimError> {
    let mut pp = Preprocessor::new();
    let preprocessed = pp
        .preprocess(source, None)
        .map_err(|e| SimError::with_diag(DiagCode::InvalidSyntax, format!("preprocessor: {}", e)))?;
    let timescale = pp.timescale.clone();
    let mut lexer = Lexer::new(&preprocessed);
    let mut tokens = Vec::new();
    loop {
        let (tok, line, col) = lexer.next_token();
        if tok == maria_parser::lexer::Token::Eof {
            break;
        }
        tokens.push((tok, line, col));
    }

    let file_line_map = lexer.file_line_map.clone();
    let first_source = if file_line_map.is_empty() {
        "<string>".to_string()
    } else {
        file_line_map[0].1.clone()
    };
    let mut parser = Parser::new(tokens, &first_source)
        .with_source_lines(&preprocessed)
        .with_file_line_map(file_line_map);
    let mut design = match parser.parse_design() {
        Ok(d) => d,
        Err(e) => {
            // Parse function returned fatal error — emit collected errors too
            if !parser.errors.is_empty() {
                let mut emitter = maria_core::diagnostics::TerminalEmitter::new().with_simple_mode(true);
                for diag in &parser.errors {
                    let _ = emitter.emit(diag);
                }
            }
            return Err(e);
        }
    };
    // Cek accumulated parser diagnostics (warnings + errors)
    // Hanya abort untuk real errors, warnings seperti "skipping construct" tetap lanjut
    if !parser.errors.is_empty() {
        let has_real_errors = parser.errors.iter().any(|d| d.is_error());
        let mut emitter = maria_core::diagnostics::TerminalEmitter::new().with_simple_mode(true);
        for diag in &parser.errors {
            let _ = emitter.emit(diag);
        }
        if has_real_errors {
            return Err(SimError::from_parse_diagnostic(parser.errors[0].clone()));
        }
    }
    design.timescale = timescale;

    let source_lines: Vec<String> = preprocessed.lines().map(|s| s.to_string()).collect();
    let mut elaborator = maria_elaboration::Elaborator::with_source(design, source_lines, first_source);
    let ir_design = elaborator.elaborate(None, ElaborateMode::StrictSimulation)?;

    // SIM-29: bawa exclusion ranges dari `` `coverage_off ``/`` `coverage_on ``
    // (koordinat output preprocessed) ke design untuk engine line coverage.
    let mut ir_design = ir_design;
    ir_design.coverage_exclusions = pp.coverage_exclusions.clone();

    // Flush elaboration-time diagnostics (warnings like WR0102)
    let elab_diags = elaborator.flush_diagnostics();
    if !elab_diags.is_empty() {
        let mut emitter = maria_core::diagnostics::TerminalEmitter::new().with_simple_mode(true);
        for diag in &elab_diags {
            let _ = emitter.emit(diag);
        }
    }

    Ok(ir_design)
}

/// Run simulation on compiled IR
pub fn run_simulation(ir_design: maria_ir::IrDesign, max_time: u64) -> Result<(), SimError> {
    let mut engine = simulator::SimulationEngine::new(ir_design, max_time);

    let design_name = &engine.design.top.name.clone();
    // Use a unique prefix to avoid file name collisions when running tests in parallel
    // Many tests use "top" as the module name, which would cause file conflicts
    static SIMULATION_COUNTER: AtomicUsize = AtomicUsize::new(0);
    let unique_id = SIMULATION_COUNTER.fetch_add(1, Ordering::Relaxed);
    let unique_prefix = format!("{}_{}", design_name, unique_id);
    let vcd_path = format!("{}.vcd", unique_prefix);
    let vcd = waveform::VcdWriter::new(&vcd_path, &engine.design)
        .map_err(|e| SimError::with_diag(DiagCode::WaveformError, format!("VCD creation failed: {}", e)))?;
    engine.set_vcd(vcd);

    // Also create FST waveform
    let fst_path = format!("{}.fst", unique_prefix);
    match waveform::FstWaveWriter::new(&fst_path, &engine.design) {
        Ok(fst) => engine.set_fst(fst),
        Err(e) => {
            let diag = maria_core::diagnostics::Diagnostic::warning(
                maria_core::diagnostics::DiagCode::WaveformError,
                format!("FST: cannot create '{}': {}", fst_path, e),
            );
            let mut emitter = maria_core::diagnostics::TerminalEmitter::new().with_simple_mode(true);
            let _ = emitter.emit(&diag);
        }
    }

    engine.run()?;

    // Flush any runtime diagnostics
    let diagnostics = engine.flush_diagnostics();
    if !diagnostics.is_empty() {
        let mut emitter = maria_core::diagnostics::TerminalEmitter::new().with_simple_mode(true);
        for diag in &diagnostics {
            let _ = emitter.emit(diag);
        }
    }

    println!("Simulation completed at time {}", engine.state.time);
    println!("VCD waveform written to '{}'", vcd_path);
    println!("FST waveform written to '{}'", fst_path);

    Ok(())
}

/// Run simulation and return final signal values
pub fn simulate_signals(
    source: &str,
    max_time: u64,
) -> Result<Vec<(String, maria_ir::LogicVec)>, SimError> {
    let design = compile_str(source)?;
    let mut engine = simulator::SimulationEngine::new(design, max_time);
    engine.run()?;
    let sigs: Vec<(String, maria_ir::LogicVec)> = engine
        .design
        .top
        .signals
        .iter()
        .map(|s| {
            (
                s.name.to_string(),
                engine
                    .state
                    .read_signal(
                        engine
                            .design
                            .top
                            .signals
                            .iter()
                            .position(|x| x.name == s.name)
                            .unwrap_or(0),
                    )
                    .clone(),
            )
        })
        .collect();
    Ok(sigs)
}

#[cfg(test)]
#[macro_use]
pub mod test_util;

// Test suite utama pindah ke crate `maria-tests` (crates/) — lib ini hanya
// menyimpan API publik + helper test (test_util).

#[cfg(test)]
mod project_file_tests {
    use super::*;

    fn write_temp(name: &str, content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("maria_proj_{}_{}", name, std::process::id()));
        std::fs::create_dir_all(&dir).expect("buat dir");
        let p = dir.join("proj.maria");
        std::fs::write(&p, content).expect("tulis");
        p
    }

    #[test]
    fn test_read_project_with_foreign_parses_libs() {
        let p = write_temp("foreign", r#"
rtl/top.sv
rtl/counter.sv

[foreign]
vhpi = ["libvhpi_a.so", "libvhpi_b.so"]
pli = ["libpli.so"]
dpi = ["libdpi.so"]
"#);
        let proj = read_project_with_foreign(p.to_str().unwrap()).expect("parse");
        // File .sv ter-resolve relatif ke direktori project.
        assert_eq!(proj.files.len(), 2);
        let base = p.parent().unwrap();
        assert!(proj.files[0].starts_with(base.to_str().unwrap()), "path relatif ke .maria");
        // Library ter-resolve + terpisah per interface.
        assert_eq!(proj.vhpi_libs.len(), 2);
        assert!(proj.vhpi_libs[0].contains("libvhpi_a.so"));
        assert!(proj.vhpi_libs[1].contains("libvhpi_b.so"));
        assert_eq!(proj.pli_libs.len(), 1);
        assert!(proj.pli_libs[0].contains("libpli.so"));
        assert_eq!(proj.dpi_libs.len(), 1);
        assert!(proj.dpi_libs[0].contains("libdpi.so"));
        // Path library juga relatif ke direktori project.
        assert!(proj.vhpi_libs[0].starts_with(base.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&p.parent().unwrap());
    }

    #[test]
    fn test_read_project_file_skips_foreign_section() {
        let p = write_temp("skip", r#"
rtl/top.sv

[foreign]
vhpi = ["libvhpi.so"]
"#);
        let files = read_project_file(p.to_str().unwrap()).expect("parse");
        assert_eq!(files.len(), 1, "bagian [foreign] TIDAK boleh jadi file .sv");
        assert!(files[0].contains("rtl/top.sv"));
        let _ = std::fs::remove_dir_all(&p.parent().unwrap());
    }

    #[test]
    fn test_read_project_with_foreign_unknown_key_warns() {
        let p = write_temp("unknown", r#"
rtl/top.sv

[foreign]
vhpi = ["libvhpi.so"]
foo = ["libfoo.so"]
"#);
        let proj = read_project_with_foreign(p.to_str().unwrap()).expect("parse");
        assert_eq!(proj.vhpi_libs.len(), 1);
        assert!(proj.dpi_libs.is_empty(), "kunci tak dikenal diabaikan");
        assert!(proj.pli_libs.is_empty());
        let _ = std::fs::remove_dir_all(&p.parent().unwrap());
    }

    #[test]
    fn test_read_project_with_foreign_no_section() {
        let p = write_temp("nosect", "rtl/top.sv\n");
        let proj = read_project_with_foreign(p.to_str().unwrap()).expect("parse");
        assert_eq!(proj.files.len(), 1);
        assert!(proj.vhpi_libs.is_empty());
        assert!(proj.pli_libs.is_empty());
        assert!(proj.dpi_libs.is_empty());
        let _ = std::fs::remove_dir_all(&p.parent().unwrap());
    }
}
