// Allow large Result Err variant for SimError (intentional)
#![allow(clippy::result_large_err)]

mod cli;
use cli::Cli;
use clap::Parser as ClapParser;
use std::path::PathBuf;
use std::process;

use maria::frontend::CompileSession;
use maria::SessionConfig;
use maria::debugger::Debugger;
use maria::elaboration::Elaborator;
use maria::error::SimError;
use maria::ir::LogicVec;
use maria::parser::lexer::Lexer;
use maria::parser::Parser;
use maria::parser::preprocessor::Preprocessor;
use maria::read_project_file;
use maria::simulator::Breakpoint;
use maria::simulator::DebugMode;
use maria::simulator::SimulationEngine;
use maria::simulator::Watchpoint;
use maria::waveform::VcdWriter;
use rayon::prelude::*;

/// Emit a list of diagnostics through TerminalEmitter.
fn emit_diags(diags: &[maria::diagnostics::diagnostic::Diagnostic]) {
    if diags.is_empty() {
        return;
    }
    let mut emitter = maria::diagnostics::TerminalEmitter::new();
    for diag in diags {
        let _ = emitter.emit(diag);
    }
}

/// Run formal verification (BMC) and print results.
/// Returns Err if any assertion fails (counterexample found — for CI/CD integration).
#[cfg(feature = "formal")]
fn run_formal(ir_design: &maria::ir::IrDesign, bound: u64, quiet: bool) -> Result<(), SimError> {
    use maria::formal::*;
    let mut formal_cfg = FormalConfig::default();
    formal_cfg.bound = bound;
    let mut formal_engine = FormalEngine::new(formal_cfg);
    let results = formal_engine.check_assertions_bmc(ir_design);

    if !quiet {
        println!("\n── Formal Verification Results (BMC bound={}) ──", bound);
    }

    let has_fail = results.iter().any(|(_, r)| matches!(r, FormalResult::Counterexample(_)));
    let has_error = results.iter().any(|(_, r)| matches!(r, FormalResult::Error(_)));

    for (name, result) in &results {
        if quiet { continue; }
        match result {
            FormalResult::Pass => println!("  ✓ PASS: {}", name),
            FormalResult::Counterexample(d) => println!("  ✗ FAIL: {} — counterexample at depth {}", name, d),
            FormalResult::Unknown => println!("  ? UNKNOWN: {}", name),
            FormalResult::Error(e) => println!("  ! ERROR: {} — {}", name, e),
            FormalResult::InductiveProof => println!("  ✓ INDUCTIVE PROOF: {}", name),
        }
    }

    if !quiet {
        if results.is_empty() {
            println!("  (no assertions found)");
        }
        println!("── End of Formal Results ({}/{} passed) ──\n",
            results.iter().filter(|(_, r)| matches!(r, FormalResult::Pass)).count(),
            results.len());
    }

    if has_error {
        return Err(SimError::new(None, "formal verification encountered errors"));
    }
    if has_fail {
        return Err(SimError::new(None, "formal verification FAILED — counterexample(s) found"));
    }
    Ok(())
}

fn main() {
    // Configure rayon thread pool with larger stack for deep recursion in parser
    // Some SV files have deeply nested blocks that need more than the default 2MB stack
    rayon::ThreadPoolBuilder::new()
        .stack_size(16 * 1024 * 1024) // 16MB stack
        .build_global()
        .ok();

    let cli = Cli::parse();

    // ── LSP mode: start language server (stdio transport) ──
    #[cfg(feature = "lsp")]
    if cli.lsp {
        let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime for LSP");
        rt.block_on(maria::lsp::run_lsp_server());
        return;
    }
    #[cfg(not(feature = "lsp"))]
    if cli.lsp {
        eprintln!("LSP server not available: compile with --features lsp");
        return;
    }

    let result = run(cli);
    if let Err(e) = result {
        // Use TerminalEmitter for pretty diagnostic output
        let mut emitter = maria::diagnostics::TerminalEmitter::new();
        let diag = e.to_diagnostic();
        let _ = emitter.emit(&diag);
        process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), SimError> {
    let mut sources: Vec<String> = if cli.start {
        read_project_file(".maria")?
    } else {
        cli.files.clone()
    };

    // Read file list from -f
    if let Some(ref fpath) = cli.filelist {
        let flist = read_project_file(fpath)?;
        sources.extend(flist);
    }

    // Create shared preprocessor with CLI config
    let mut base_pp = Preprocessor::new();
    base_pp.quiet = cli.quiet;
    for path in &cli.incdirs {
        base_pp.add_search_path(path);
    }
    for def in &cli.defines {
        if let Some((name, value)) = def.split_once('=') {
            base_pp.define(name, value);
        } else {
            base_pp.define(def, "");
        }
    }
    // ── Fast pipeline via CompileSession (skip legacy pipeline entirely) ──
    // Auto-use fast pipeline when filelist is specified (legacy can't handle large file sets)
    // Also skip expensive auto-incdir scanning for the fast path
    if cli.fast || cli.filelist.is_some() {
        return run_fast(cli, None);
    }

    // Auto-detect include paths: consolidated single-pass scan
    // Walk up from each source dir's ancestors, recursively scan for SV files (depth ≤ 4)
    // NOTE: This is expensive so only runs for the legacy pipeline, not the fast path.
    let mut seen_dirs = std::collections::HashSet::new();
    let mut src_dirs = std::collections::HashSet::new();
    for src in &sources {
        if let Some(dir) = std::path::Path::new(src).parent() {
            let canonical = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
            src_dirs.insert(canonical);
        }
    }
    fn collect_sv_dirs(
        dir: &std::path::PathBuf,
        base_pp: &mut Preprocessor,
        seen: &mut std::collections::HashSet<PathBuf>,
        depth: usize,
    ) {
        if depth > 4 {
            return;
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                if let Ok(ft) = entry.file_type() {
                    let path = entry.path();
                    if ft.is_dir() && depth < 4 {
                        collect_sv_dirs(&path, base_pp, seen, depth + 1);
                    } else if ft.is_file() {
                        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                        if (ext == "svh" || ext == "sv") && seen.insert(path.clone()) {
                            if let Some(parent) = path.parent() {
                                base_pp.add_search_path(parent.to_str().unwrap());
                            }
                        }
                    }
                }
            }
        }
    }
    for src_dir in &src_dirs {
        let mut anc = Some(src_dir.clone());
        while let Some(ref d) = anc {
            if !seen_dirs.insert(d.clone()) {
                break;
            }
            if let Ok(entries) = std::fs::read_dir(d) {
                for entry in entries.flatten() {
                    if let Ok(ft) = entry.file_type() {
                        let path = entry.path();
                        if ft.is_dir() {
                            collect_sv_dirs(&path, &mut base_pp, &mut seen_dirs, 0);
                        }
                    }
                }
            }
            anc = d.parent().map(|p| p.to_path_buf());
        }
    }

    // Combine all sources (parallel preprocessing for many files)
    
    let mut combined = String::new();
    let mut design_timescale = None;

    // Preprocess files in parallel using rayon
    
    eprintln!("[TIMING] Preprocessing {} files...", sources.len());
    let pp_start = std::time::Instant::now();
    let pp_for_parallel = &base_pp;
    let pp_results: Vec<Result<(String, Option<(String, String)>), String>> = sources
        .par_iter()
        .enumerate()
        .map(|(_idx, path)| {

            let mut pp = pp_for_parallel.clone();
            match pp.preprocess_file(path) {
                Ok(processed) => {
                    let ts = pp.timescale.clone();
                    Ok((processed, ts))
                }
                Err(e) => Err(format!("preprocessor '{}': {}", path, e)),
            }
        })
        .collect();
    eprintln!("[TIMING] Preprocessing done in {:?}", pp_start.elapsed());

    for (i, path) in sources.iter().enumerate() {
        let (processed, ts) = match &pp_results[i] {
            Ok(r) => (r.0.clone(), r.1.clone()),
            Err(e) => {
                return Err(SimError::new(None, format!("preprocessing failed: {}", e)));
            }
        };
        if let Some(ref ts) = ts {
            design_timescale = Some(ts.clone());
        }
        combined.push_str(&format!("`line 1 \"{}\"\n", path));
        combined.push_str(&processed);
        combined.push('\n');
    }

    eprintln!("[TIMING] Starting lexer (combined size: {} bytes)...", combined.len());
    let lex_start = std::time::Instant::now();
    let mut lexer = Lexer::new(&combined);
    let mut tokens = Vec::new();
    
    loop {
        let (tok, line, col) = lexer.next_token();
        if cli.print_tokens {
            println!("  {:4}:{:4} {}", line, col, tok);
        }
        if tok == maria::parser::lexer::Token::Eof {
            break;
        }
        tokens.push((tok, line, col));
    }
    eprintln!("[TIMING] Lexer done: {} tokens in {:?}", tokens.len(), lex_start.elapsed());

    if tokens.is_empty() {
        return Err(SimError::new(None, "no tokens found (empty source?)"));
    }

    let first_source = sources.first().map(|s| s.as_str()).unwrap_or("<unknown>");

    let file_line_map = lexer.file_line_map.clone();
    eprintln!("[TIMING] Starting parser...");
    let parse_start = std::time::Instant::now();
    let mut parser = Parser::new(tokens, first_source)
        .with_source_lines(&combined)
        .with_file_line_map(file_line_map);
    let mut design = match parser.parse_design() {
        Ok(d) => {
            eprintln!("[TIMING] Parser done in {:?}", parse_start.elapsed());
            d
        },
        Err(e) => {
            if !parser.errors.is_empty() {
                let mut emitter = maria::diagnostics::TerminalEmitter::new();
                for diag in &parser.errors {
                    let _ = emitter.emit(diag);
                }
            }
            return Err(e);
        }
    };
    // Emit parser diagnostics (warnings + errors) — only abort for real errors
    // In compile-only mode, individual construct errors are non-fatal
    if !parser.errors.is_empty() {
        let has_real_errors = parser.errors.iter().any(|d| d.is_error());
        let mut emitter = maria::diagnostics::TerminalEmitter::new();
        for diag in &parser.errors {
            let _ = emitter.emit(diag);
        }
        if has_real_errors && !cli.compile_only {
            return Err(maria::error::SimError::from_parse_diagnostic(parser.errors[0].clone()));
        }
    }
    let ts_for_ir = design_timescale.clone();
    design.timescale = design_timescale;

    // ── Library scanning: always scan library directories/files before elaboration ──
    for libdir in &cli.libdirs {
        base_pp.add_search_path(libdir);
        if let Ok(entries) = std::fs::read_dir(libdir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if ext == "v" || ext == "sv" {
                        let mut pp = base_pp.clone();
                        let path_str = path.to_string_lossy().to_string();
                        match pp.preprocess_file(&path_str) {
                            Ok(processed) => {
                                let combined_lib =
                                    format!("`line 1 \"{}\"\n{}", path.display(), processed);
                                let mut lexer = Lexer::new(&combined_lib);
                                let mut lib_tokens = Vec::new();
                                loop {
                                    let (tok, line, col) = lexer.next_token();
                                    if tok == maria::parser::lexer::Token::Eof {
                                        break;
                                    }
                                    lib_tokens.push((tok, line, col));
                                }
                                let mut parser =
                                    Parser::new(lib_tokens, path.to_str().unwrap_or("<lib>"));
                                parser = parser.with_source_lines(&combined_lib);
                                match parser.parse_design() {
                                    Ok(lib_design) => {
                                        for m in lib_design.modules {
                                            if !design.modules.iter().any(|dm| dm.name == m.name) {
                                                design.modules.push(m);
                                            }
                                        }
                                    }
                                    Err(e) => eprintln!(
                                        "warning: library file '{}' parse error: {}",
                                        path.display(),
                                        e
                                    ),
                                }
                            }
                            Err(e) => eprintln!(
                                "warning: library file '{}' preprocess error: {}",
                                path.display(),
                                e
                            ),
                        }
                    }
                }
            }
        }
    }
    for libfile in &cli.libfiles {
        let mut pp = base_pp.clone();
        let libfile_path = std::path::Path::new(libfile);
        if let Some(dir) = libfile_path.parent() {
            if let Some(dir_str) = dir.to_str() {
                base_pp.add_search_path(dir_str);
            }
        }
        match pp.preprocess_file(libfile) {
            Ok(processed) => {
                let combined_lib = format!("`line 1 \"{}\"\n{}", libfile, processed);
                let mut lexer = Lexer::new(&combined_lib);
                let mut lib_tokens = Vec::new();
                loop {
                    let (tok, line, col) = lexer.next_token();
                    if tok == maria::parser::lexer::Token::Eof {
                        break;
                    }
                    lib_tokens.push((tok, line, col));
                }
                let mut parser = Parser::new(lib_tokens, libfile);
                parser = parser.with_source_lines(&combined_lib);
                match parser.parse_design() {
                    Ok(lib_design) => {
                        for m in lib_design.modules {
                            if !design.modules.iter().any(|dm| dm.name == m.name) {
                                design.modules.push(m);
                            }
                        }
                    }
                    Err(e) => eprintln!("warning: library file '{}' parse error: {}", libfile, e),
                }
            }
            Err(e) => eprintln!(
                "warning: library file '{}' preprocess error: {}",
                libfile, e
            ),
        }
    }

    if design.modules.is_empty() {
        // If there are packages, interfaces, or other items but no modules, it's not fatal
        if !design.packages.is_empty()
            || !design.interfaces.is_empty()
            || !design.classes.is_empty()
        {
            if !cli.quiet {
                eprintln!("note: no modules found in design (packages/interfaces present, skipping simulation)");
            }
            return Ok(());
        }
        return Err(SimError::new(None, "no modules found in design"));
    }

    let top_name = cli.top.as_deref();
    if !cli.quiet {
        println!("Compiling design ({} file sources)...", sources.len());
    }
    eprintln!("[TIMING] Starting elaboration...");
    let elab_start = std::time::Instant::now();
    let source_lines: Vec<String> = combined.lines().map(|s| s.to_string()).collect();
    let mut elaborator = Elaborator::with_source(design, source_lines, first_source.to_string());
    let mut ir_design = elaborator.elaborate(top_name)?;
    eprintln!("[TIMING] Elaboration done in {:?}", elab_start.elapsed());

    // Flush elaboration-time diagnostics (warnings like WR0102)
    emit_diags(&elaborator.flush_diagnostics());

    ir_design.timescale = ts_for_ir;

    if !cli.quiet {
        println!(
            "Module '{}': {} signals, {} processes",
            ir_design.top.name,
            ir_design.top.signals.len(),
            ir_design.top.processes.len()
        );
    }

    // ── Formal Verification (runs before simulation, skips sim) ──
    #[cfg(feature = "formal")]
    if cli.formal {
        return run_formal(&ir_design, cli.formal_bound, cli.quiet);
    }
    #[cfg(not(feature = "formal"))]
    if cli.formal {
        eprintln!("Formal verification not available: compile with --features formal");
        return Err(maria::error::SimError::new(None, "formal feature not enabled"));
    }

    // ── Compile-only mode: skip simulation & VCD ──
    if cli.compile_only {
        if !cli.quiet {
            println!("Compile-only mode: skipping simulation");
        }
        return Ok(());
    }

    // ── Setup ──
    let debug_mode = if cli.deep_debug {
        DebugMode::DeepDebug
    } else if cli.debug
        || cli.step
        || !cli.break_cycle.is_empty()
        || !cli.break_change.is_empty()
        || !cli.break_eq.is_empty()
        || !cli.watch.is_empty()
    {
        DebugMode::Debug
    } else {
        DebugMode::Normal
    };

    // Set X-propagation mode from CLI
    if let Some(mode) = maria::simulator::types::XPropagationMode::from_str(&cli.xprop) {
        maria::simulator::value::set_xprop_mode(mode);
        if !cli.quiet {
            println!("X-propagation mode: {}", mode.as_str());
        }
    } else {
        return Err(SimError::new(None, format!("invalid --xprop '{}': use optimistic, pessimistic, or x-anywhere", cli.xprop)));
    }

    // ── Distributed simulation mode ──
    if cli.dist_master {
        let config = maria::simulator::distributed::MasterConfig {
            port: cli.dist_port,
            num_partitions: cli.num_partitions,
            verbose: !cli.quiet,
            ..Default::default()
        };
        let mut master = maria::simulator::distributed::DistributedMaster::new(config);
        master.run(&ir_design, cli.max_time)?;
        if !cli.quiet {
            println!("Distributed simulation (master) complete");
        }
        return Ok(());
    }

    if cli.dist_slave {
        let config = maria::simulator::distributed::SlaveConfig {
            master_host: cli.master_host.clone(),
            master_port: cli.dist_port,
            max_time: cli.max_time,
            verbose: !cli.quiet,
        };
        let mut slave = maria::simulator::distributed::DistributedSlave::new(config);
        slave.run(&ir_design)?;
        if !cli.quiet {
            println!("Distributed simulation (slave) complete");
        }
        return Ok(());
    }

    let mut engine = SimulationEngine::new(ir_design, cli.max_time);

    // ── Set SDF timing mode ──
    if let Some(mode) = maria::simulator::sdf::TimingMode::from_str(&cli.timing_mode) {
        maria::simulator::sdf::set_timing_mode(mode);
        if !cli.quiet {
            println!("SDF timing mode: {}", mode.as_str());
        }
    } else {
        return Err(SimError::new(None, format!("invalid --timing-mode '{}': use min, typ, or max", cli.timing_mode)));
    }

    // ── SDF Annotation (applies timing delays from Standard Delay Format file) ──
    if let Some(ref sdf_path) = cli.sdf {
        let sdf_data = maria::simulator::sdf::SdfData::parse_file(sdf_path)
            .map_err(|e| SimError::new(None, format!("SDF parse failed: {}", e)))?;
        engine.annotate_sdf(&sdf_data)?;
        if !cli.quiet {
            println!("SDF annotation loaded from '{}'", sdf_path);
        }
    }

    engine.debug_mode = debug_mode;
    engine.snapshot_interval = cli.snap_interval;
    engine.use_packed_eval = cli.packed;
    engine.use_dag_parallel = cli.parallel;
    engine.use_mir_jit = cli.jit_body;
    engine.use_timing_wheel = cli.use_timing_wheel;
    if cli.use_timing_wheel && !cli.quiet {
        println!("Timing wheel enabled (O(1) event scheduling)");
    }
    if cli.jit_body && !cli.quiet {
        println!("Body-level MIR JIT enabled (compiled-code simulation path)");
    }

    // ── Co-simulation (VHDL/SystemVerilog bridge) ──
    if let Some(cosim_port) = cli.cosim_port {
        // Build signal mapping from --cosim-signals or auto-detect
        let signal_mapping: Vec<(usize, String, bool)> = if let Some(ref sig_names) = cli.cosim_signals {
            sig_names.split(',')
                .filter_map(|name| {
                    let trimmed = name.trim();
                    let is_output = trimmed.starts_with('+');
                    let clean_name = trimmed.trim_start_matches('+');
                    engine.design.top.signals.iter().position(|s| s.name.as_str() == clean_name)
                        .map(|id| (id, clean_name.to_string(), is_output))
                })
                .collect()
        } else {
            // Auto-detect: all output ports are outputs, all input ports are inputs
            engine.design.top.signals.iter().enumerate()
                .filter_map(|(id, s)| {
                    let is_output = matches!(s.kind, maria::ir::SignalKind::Output | maria::ir::SignalKind::Inout);
                    Some((id, s.name.to_string(), is_output))
                })
                .collect()
        };

        if signal_mapping.is_empty() && !cli.quiet {
            eprintln!("warning: no signals mapped for co-simulation on port {}", cosim_port);
        }

        let n_sigs = signal_mapping.len();
        let cosim_state = maria::simulator::cosim::start_cosim_server(cosim_port, n_sigs);
        engine.cosim_state = cosim_state;
        engine.cosim_signals = signal_mapping.clone();

        if !cli.quiet {
            println!("Co-simulation bridge active on port {} ({} signals)", cosim_port, signal_mapping.len());
        }
    }

    // Configure signal history disk spill
    if let Some(ref spill_path) = cli.signal_history_spill {
        engine.signal_history.enable_spill(std::path::PathBuf::from(spill_path));
        if !cli.quiet {
            println!("Signal history spill to '{}'", spill_path);
        }
    }

    // ── UPF Power Intent (power-aware simulation) ──
    if let Some(ref upf_path) = cli.upf {
        match maria::simulator::upf::PowerIntent::parse_file(upf_path) {
            Ok(mut power_intent) => {
                power_intent.build_signal_mapping(&engine.design.top.signals);
                if !cli.quiet {
                    println!("UPF power intent loaded from '{}' ({} domains, {} supply nets)",
                        upf_path, power_intent.domains.len(), power_intent.supply_nets.len());
                }
                engine.power_intent = Some(power_intent);
            }
            Err(e) => {
                eprintln!("warning: UPF parse failed: {}", e);
            }
        }
    }

    // Load DPI shared libraries
    #[cfg(feature = "dpi")]
    if !cli.dpi_libs.is_empty() {
        use maria::simulator::dpi::DpiEngine;
        #[allow(unused_imports)]
        use std::sync::Mutex;
        fn get_dpi_engine() -> &'static Mutex<Option<DpiEngine>> {
            use std::sync::OnceLock;
            static DPI: OnceLock<Mutex<Option<DpiEngine>>> = OnceLock::new();
            DPI.get_or_init(|| Mutex::new(Some(DpiEngine::new())))
        }
        if let Ok(mut guard) = get_dpi_engine().lock() {
            if let Some(ref mut eng) = *guard {
                for lib_path in &cli.dpi_libs {
                    match eng.load_library(lib_path) {
                        Ok(_) => {
                            if !cli.quiet {
                                println!("  DPI library loaded: {}", lib_path);
                            }
                        }
                        Err(e) => {
                            eprintln!("warning: failed to load DPI library '{}': {}", lib_path, e);
                        }
                    }
                }
            }
        }
    }

    // Apply plusargs
    for pa in &cli.plusargs {
        if let Some((key, val)) = pa.split_once('=') {
            engine.plusargs.insert(key.to_string(), val.to_string());
        } else {
            engine.plusargs.insert(pa.clone(), String::new());
        }
    }

    // Apply breakpoints
    for c in &cli.break_cycle {
        engine.breakpoints.push(Breakpoint::Cycle(*c));
        if !cli.quiet {
            println!("  breakpoint: cycle {}", c);
        }
    }
    for name in &cli.break_change {
        engine
            .breakpoints
            .push(Breakpoint::SignalChange(name.clone()));
        if !cli.quiet {
            println!("  breakpoint: change {}", name);
        }
    }
    for eq in &cli.break_eq {
        if let Some((name, val_hex)) = eq.split_once('=') {
            if let Ok(val) = u64::from_str_radix(
                val_hex.trim_start_matches("0x").trim_start_matches("0X"),
                16,
            ) {
                let w = engine
                    .design
                    .top
                    .signals
                    .iter()
                    .find(|s| s.name == name)
                    .map(|s| s.width)
                    .unwrap_or(32);
                engine.breakpoints.push(Breakpoint::SignalEq(
                    name.to_string(),
                    LogicVec::from_u64(val, w),
                ));
                if !cli.quiet {
                    println!("  breakpoint: {} == 0x{:X}", name, val);
                }
            }
        }
    }
    // Apply watchpoints
    for name in &cli.watch {
        engine.watchpoints.push(Watchpoint::Signal(name.clone()));
        if !cli.quiet {
            println!("  watchpoint: {}", name);
        }
    }

    // VCD setup
    let vcd_path = cli
        .output
        .unwrap_or_else(|| format!("{}.vcd", engine.design.top.name));
    let mut vcd = VcdWriter::new(&vcd_path, &engine.design)
        .map_err(|e| SimError::new(None, format!("VCD creation failed: {}", e)))?;
    if cli.waveform_stream {
        vcd.stream_flush_interval = 1;
        if !cli.quiet {
            println!("Waveform streaming enabled (flush every time step)");
        }
    }
    engine.set_vcd(vcd);

    // CSV waveform setup
    if let Some(ref csv_path) = cli.waveform_csv {
        let csv = maria::waveform::CsvWaveWriter::new(csv_path, &engine.design)
            .map_err(|e| SimError::new(None, format!("CSV creation failed: {}", e)))?;
        engine.set_csv(csv);
        if !cli.quiet {
            println!("CSV waveform: {}", csv_path);
        }
    }

    // Signal statistics setup
    if cli.signal_stats.is_some() {
        let stats = maria::waveform::SignalStats::new(&engine.design);
        engine.set_signal_stats(stats);
    }

    // ── Simulation ──
    let mut debugger = Debugger::new(engine);

    if cli.print_tree {
        println!("\n{}", debugger.hierarchy_tree());
    }

    if cli.step && debug_mode != DebugMode::Normal {
        if !cli.quiet {
            println!("\nStep mode: running one cycle...");
        }
        debugger.step_cycle()?;
        if !cli.quiet {
            println!("{}\n", debugger.print_state_summary());
        }
        if !debugger.engine.event_log.is_empty() && !cli.quiet {
            println!("{}", debugger.print_event_log());
        }
    } else {
        if !cli.quiet {
            println!(
                "\nStarting simulation (max time={}, vcd={})",
                cli.max_time, vcd_path
            );
        }
        debugger.run()?;
    }

    // ── Post-simulation output ──
    if !cli.quiet {
        println!(
            "\nSimulation completed at time {}",
            debugger.engine.state.time
        );
    }

    // Flush runtime diagnostics (warnings, etc.)
    emit_diags(&debugger.engine.flush_diagnostics());

    if debug_mode != DebugMode::Normal && !cli.quiet {
        if debugger.engine.paused {
            println!("(debugger paused)");
        }
        if !debugger.engine.event_log.is_empty() {
            println!("\nDebug events:");
            println!("{}", debugger.print_event_log());
        }
    }

    // Print signals
    if cli.print_state {
        println!("\n{}", debugger.print_all_signals());
    }
    for name in &cli.print_signal {
        println!("  {}", debugger.print_signal(name));
    }
    for name in &cli.timeline {
        println!("\n{}", debugger.timeline(name, cli.timeline_len));
    }
    if cli.mem.len() == 2 {
        if let (Ok(addr), Ok(len)) = (
            u64::from_str_radix(
                cli.mem[0].trim_start_matches("0x").trim_start_matches("0X"),
                16,
            ),
            cli.mem[1].parse::<usize>(),
        ) {
            println!("\n{}", debugger.memory_inspect(addr, len));
        }
    }

    if !cli.quiet {
        println!("VCD waveform written to '{}'", vcd_path);
    }

    // ── CSV close ──
    let _ = debugger.engine.close_csv();

    // ── Signal statistics ──
    if let Some(ref stats_path) = cli.signal_stats {
        let path = if stats_path.is_empty() {
            format!("{}.stats.txt", debugger.engine.design.top.name)
        } else {
            stats_path.clone()
        };
        if let Some(ref stats) = debugger.engine.signal_stats {
            if let Err(e) = stats.write_to_file(&path) {
                eprintln!("warning: signal stats write failed: {}", e);
            } else if !cli.quiet {
                println!("Signal statistics written to '{}'", path);
            }
        }
    }

    // ── GTKWave save file ──
    if let Some(ref gtkw_path) = cli.gtkw {
        let path = if gtkw_path.is_empty() {
            format!("{}.gtkw", debugger.engine.design.top.name)
        } else {
            gtkw_path.clone()
        };
        match maria::waveform::save_gtkw(&path, &vcd_path, &debugger.engine.design) {
            Ok(()) => {
                if !cli.quiet {
                    println!("GTKWave save file written to '{}'", path);
                }
            }
            Err(e) => eprintln!("warning: GTKW save failed: {}", e),
        }
    }

    // ── HTML waveform viewer ──
    if let Some(ref html_path) = cli.waveform_html_viewer {
        let csv_ref = cli.waveform_csv.as_deref().unwrap_or("output.csv");
        let path = if html_path.is_empty() {
            format!("{}.html", debugger.engine.design.top.name)
        } else {
            html_path.clone()
        };
        match maria::waveform::save_html_viewer(&path, csv_ref, &debugger.engine.design) {
            Ok(()) => {
                if !cli.quiet {
                    println!("HTML waveform viewer written to '{}'", path);
                }
            }
            Err(e) => eprintln!("warning: HTML viewer failed: {}", e),
        }
    }

    // UCIS coverage export
    if let Some(ref ucis_path) = cli.coverage_ucis {
        let path = if ucis_path.is_empty() {
            format!("{}.ucis.xml", debugger.engine.design.top.name)
        } else {
            ucis_path.clone()
        };
        match debugger.engine.export_coverage_ucis(&path) {
            Ok(()) => {
                if !cli.quiet {
                    println!("UCIS coverage written to '{}'", path);
                }
            }
            Err(e) => eprintln!("UCIS export failed: {}", e),
        }
    }

    // Signal history stats
    if !cli.quiet && cli.signal_history_spill.is_some() {
        let stats = debugger.engine.signal_history.stats();
        println!("Signal history: {} mem entries, {} spilled to disk",
            stats.total_memory_entries, stats.total_spilled_entries);
    }

    // Save coverage database if requested
    if let Some(ref covdb_path) = cli.coverage_ucdb {
        let mut covdb = maria::simulator::coverage_db::CoverageDatabase::with_path(covdb_path);
        covdb.merge_from_engine(&debugger.engine);
        if let Err(e) = covdb.save() {
            eprintln!("warning: coverage DB save failed: {}", e);
        } else if !cli.quiet {
            println!("Coverage database saved to '{}'", covdb_path);
        }

        // Coverage threshold check
        if let Some(threshold) = cli.coverage_threshold {
            let stats = debugger.engine.coverage_stats();
            let branch_pct = stats.get("branch_percent").copied().unwrap_or(0.0);
            if branch_pct < threshold {
                let msg = format!("COVERAGE FAILED: branch coverage {:.1}% < threshold {:.1}%", branch_pct, threshold);
                eprintln!("warning: {}", msg);
                return Err(SimError::new(None, msg));
            } else if !cli.quiet {
                println!("Coverage threshold: {:.1}% >= {:.1}% ✅", branch_pct, threshold);
            }
        }
    }

    // Export HTML coverage report
    if let Some(ref html_path) = cli.coverage_html {
        let mut covdb = maria::simulator::coverage_db::CoverageDatabase::new();
        covdb.merge_from_engine(&debugger.engine);
        if let Err(e) = covdb.export_html(html_path) {
            eprintln!("warning: HTML coverage report failed: {}", e);
        } else if !cli.quiet {
            println!("HTML coverage report written to '{}'", html_path);
        }
    }

    // Save checkpoint to file if requested
    if let Some(save_path) = &cli.save {
        let _ = debugger.engine.signal_history.flush();
        let path = std::path::Path::new(save_path);
        debugger.engine.save_checkpoint(path)
            .map_err(|e| SimError::new(None, format!("checkpoint save failed: {}", e)))?;
        if !cli.quiet {
            println!("Checkpoint saved to '{}'", save_path);
        }
    }

    // CDC (Clock-Domain Crossing) analysis
    if let Some(ref cdc_path) = cli.cdc_report {
        if !cli.quiet {
            println!("Running CDC analysis...");
        }
        let cdc_analysis = maria::scheduler::cdc::CdcAnalysis::analyze(&debugger.engine.design);

        // Print summary to console
        if !cli.quiet {
            println!(
                "CDC: {} crossings — {} unsynchronized, {} single-flop, {} OK (2+ flops)",
                cdc_analysis.total_crossings,
                cdc_analysis.unsynchronized_count,
                cdc_analysis.single_flop_count,
                cdc_analysis.sync_ok_count,
            );
            if cdc_analysis.unsynchronized_count > 0 {
                eprintln!(
                    "  ⚠️ {} unsynchronized crossing(s) detected — check CDC report for details",
                    cdc_analysis.unsynchronized_count
                );
            }
        }

        // Write detailed report
        let path = if cdc_path.is_empty() {
            format!("{}_cdc_report.txt", debugger.engine.design.top.name.as_str())
        } else {
            cdc_path.clone()
        };
        match cdc_analysis.write_report(&path) {
            Ok(()) => {
                if !cli.quiet {
                    println!("CDC report written to '{}'", path);
                }
            }
            Err(e) => eprintln!("warning: CDC report write failed: {}", e),
        }
    }

    Ok(())
}

/// Run compilation + simulation using the new parallel pipeline (CompileSession + FastLexer).
fn run_fast(cli: Cli, _timescale: Option<(String, String)>) -> Result<(), SimError> {
    let mut sources: Vec<PathBuf> = if cli.start {
        read_project_file(".maria")?
            .into_iter()
            .map(PathBuf::from)
            .collect()
    } else {
        cli.files.iter().map(PathBuf::from).collect()
    };
    if let Some(ref fpath) = cli.filelist {
        let flist = read_project_file(fpath)?;
        sources.extend(flist.into_iter().map(PathBuf::from));
    }

    let config = SessionConfig {
        sources,
        incdirs: cli.incdirs.iter().map(PathBuf::from).collect(),
        defines: cli
            .defines
            .iter()
            .filter_map(|d| d.split_once('='))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        top_module: cli.top.clone(),
        auto_incdirs: cli.start || cli.files.is_empty(),
        libdirs: cli.libdirs.iter().map(PathBuf::from).collect(),
        libfiles: cli.libfiles.iter().map(PathBuf::from).collect(),
        use_fast_lexer: !cli.legacy_lexer,
        use_lazy_elab: cli.lazy,
    };

    let mut session = CompileSession::new(config);

    if cli.profile {
        session.enable_profiling();
    }

    // ── Compile-only mode: parse only, skip elaboration & simulation ──
    if cli.compile_only {
        let (design, module_index) = if cli.lazy {
            let (design, hir_count, index_len) = session.compile_lazy_only()?;
            if !cli.quiet {
                session.print_timing();
                println!("Modules indexed: {}", index_len);
                println!("Lazy-elaborated modules (HIR): {}", hir_count);
                if let Some(_top) = &session.config.top_module {
                    println!("HIR query ready: session.elaborate_lazy_module(...)");
                }
            }
            return Ok(());
        } else {
            session.compile()?
        };
        let index_len = module_index.len();
        if !cli.quiet {
            session.print_timing();
            println!("Modules indexed: {}", index_len);
        }
        if cli.print_ast {
            println!("{:#?}", design);
        }
        return Ok(());
    }
    // ── Lazy mode: skip full elaboration ──
    let top_name = cli.top.as_deref();
    if cli.lazy {
        let (_design, _ir_design, index_len) = session.compile_and_elaborate(top_name)?;
        if !cli.quiet {
            session.print_timing();
            println!("Modules indexed: {}, lazy HIR modules: {}", index_len, session.lazy_elaborated_count());
        }
        return Ok(());
    }

    // ── Full pipeline: compile + elaborate ──
    let (design, ir_design, index_len) = if cli.recompile {
        if !cli.quiet { eprintln!("Forcing full recompile..."); }
        let all_sources: Vec<PathBuf> = session.config.sources.clone();
        let (design, module_index) = session.compile_incremental(&all_sources)?;
        let index_len = module_index.len();
        if !cli.quiet { session.print_timing(); }
        let design_clone = design.clone();
        let mut elab = Elaborator::new(design);
        let ir_design = elab.elaborate(top_name)?;
        emit_diags(&elab.flush_diagnostics());
        (design_clone, ir_design, index_len)
    } else {
        let (design, module_index) = session.compile()?;
        let index_len = module_index.len();
        if !cli.quiet { session.print_timing(); }
        let design_clone = design.clone();
        let mut elab = Elaborator::new(design);
        let ir_design = elab.elaborate(top_name)?;
        emit_diags(&elab.flush_diagnostics());
        (design_clone, ir_design, index_len)
    };

    if !cli.quiet {
        println!("Modules indexed: {}", index_len);
    }

    // Configure remote cache backend
    if let Some(ref remote_dir) = cli.cache_remote_dir {
        let sync_mode = maria::cache::cache_manager::RemoteSyncMode::from_str(&cli.cache_remote_sync);
        match maria::cache::FilesystemCache::new(remote_dir) {
            Ok(backend) => {
                let remote: std::sync::Arc<dyn maria::cache::RemoteCacheBackend> =
                    std::sync::Arc::new(backend);
                session.set_remote_cache(remote, sync_mode);
                if !cli.quiet {
                    eprintln!("Remote cache enabled: {} (sync: {})", remote_dir, sync_mode.as_str());
                }
            }
            Err(e) => {
                eprintln!("Warning: cannot open remote cache '{}': {}", remote_dir, e);
            }
        }
    }

    // Clear all cache if requested
    if cli.cache_clear {
        session.clear_cache();
        if !cli.quiet {
            eprintln!("All caches cleared.");
        }
    }

    // Show cache stats if enabled
    if cli.cache_stats {
        let stats = session.cache_stats();
        eprintln!("{}", stats);
    }

    // Show profile report if enabled
    if cli.profile {
        if let Some(report) = session.profile_report() {
            eprintln!("{}", report);
        }
    }

    if cli.print_ast {
        println!("{:#?}", design);
    }

    if design.modules.is_empty() {
        return Err(SimError::new(None, "no modules found in design"));
    }

    // ── Lazy mode info (when not compile-only) ──
    if cli.lazy && !cli.quiet {
        let lazy_count = session.lazy_elaborated_count();
        if let Some(ir) = session.get_cached_ir() {
            println!(
                "Module '{}': {} signals, {} processes | Lazy HIR: {} modules elapsed",
                ir.top.name,
                ir.top.signals.len(),
                ir.top.processes.len(),
                lazy_count
            );
        }
    } else if !cli.quiet {
        println!(
            "Module '{}': {} signals, {} processes",
            ir_design.top.name,
            ir_design.top.signals.len(),
            ir_design.top.processes.len()
        );
    }

    // ── Formal Verification (runs before simulation, skips sim) ──
    #[cfg(feature = "formal")]
    if cli.formal {
        return run_formal(&ir_design, cli.formal_bound, cli.quiet);
    }
    #[cfg(not(feature = "formal"))]
    if cli.formal {
        eprintln!("Formal verification not available: compile with --features formal");
        return Err(maria::error::SimError::new(None, "formal feature not enabled"));
    }

    if cli.compile_only {
        if !cli.quiet {
            println!("Compile-only mode: skipping simulation");
        }
        return Ok(());
    }

    // ── Setup simulation ──
    let debug_mode = if cli.deep_debug {
        DebugMode::DeepDebug
    } else if cli.debug
        || cli.step
        || !cli.break_cycle.is_empty()
        || !cli.break_change.is_empty()
        || !cli.break_eq.is_empty()
        || !cli.watch.is_empty()
    {
        DebugMode::Debug
    } else {
        DebugMode::Normal
    };

    // Set X-propagation mode from CLI
    if let Some(mode) = maria::simulator::types::XPropagationMode::from_str(&cli.xprop) {
        maria::simulator::value::set_xprop_mode(mode);
        if !cli.quiet {
            println!("X-propagation mode: {}", mode.as_str());
        }
    } else {
        return Err(SimError::new(None, format!("invalid --xprop '{}': use optimistic, pessimistic, or x-anywhere", cli.xprop)));
    }

    let mut engine = SimulationEngine::new(ir_design, cli.max_time);

    // ── SDF Annotation (applies timing delays from Standard Delay Format file) ──
    if let Some(ref sdf_path) = cli.sdf {
        let sdf_data = maria::simulator::sdf::SdfData::parse_file(sdf_path)
            .map_err(|e| SimError::new(None, format!("SDF parse failed: {}", e)))?;
        engine.annotate_sdf(&sdf_data)?;
        if !cli.quiet {
            println!("SDF annotation loaded from '{}'", sdf_path);
        }
    }

    engine.debug_mode = debug_mode;
    engine.snapshot_interval = cli.snap_interval;
    engine.use_packed_eval = cli.packed;
    engine.use_dag_parallel = cli.parallel;
    engine.use_cycle_fusion = cli.cycle_fusion;
    engine.use_timing_wheel = cli.use_timing_wheel;

    // Configure signal history disk spill
    if let Some(ref spill_path) = cli.signal_history_spill {
        engine.signal_history.enable_spill(std::path::PathBuf::from(spill_path));
        if !cli.quiet {
            println!("Signal history spill to '{}'", spill_path);
        }
    }

    // ── UPF Power Intent (power-aware simulation) ──
    if let Some(ref upf_path) = cli.upf {
        match maria::simulator::upf::PowerIntent::parse_file(upf_path) {
            Ok(mut power_intent) => {
                power_intent.build_signal_mapping(&engine.design.top.signals);
                if !cli.quiet {
                    println!("UPF power intent loaded from '{}' ({} domains, {} supply nets)",
                        upf_path, power_intent.domains.len(), power_intent.supply_nets.len());
                }
                engine.power_intent = Some(power_intent);
            }
            Err(e) => {
                eprintln!("warning: UPF parse failed: {}", e);
            }
        }
    }

    // Load DPI shared libraries
    #[cfg(feature = "dpi")]
    if !cli.dpi_libs.is_empty() {
        use maria::simulator::dpi::DpiEngine;
        #[allow(unused_imports)]
        use std::sync::Mutex;
        fn get_dpi_engine() -> &'static Mutex<Option<DpiEngine>> {
            use std::sync::OnceLock;
            static DPI: OnceLock<Mutex<Option<DpiEngine>>> = OnceLock::new();
            DPI.get_or_init(|| Mutex::new(Some(DpiEngine::new())))
        }
        if let Ok(mut guard) = get_dpi_engine().lock() {
            if let Some(ref mut eng) = *guard {
                for lib_path in &cli.dpi_libs {
                    match eng.load_library(lib_path) {
                        Ok(_) => {
                            if !cli.quiet {
                                println!("  DPI library loaded: {}", lib_path);
                            }
                        }
                        Err(e) => {
                            eprintln!("warning: failed to load DPI library '{}': {}", lib_path, e);
                        }
                    }
                }
            }
        }
    }

    for pa in &cli.plusargs {
        if let Some((key, val)) = pa.split_once('=') {
            engine.plusargs.insert(key.to_string(), val.to_string());
        } else {
            engine.plusargs.insert(pa.clone(), String::new());
        }
    }

    for c in &cli.break_cycle {
        engine.breakpoints.push(Breakpoint::Cycle(*c));
        if !cli.quiet {
            println!("  breakpoint: cycle {}", c);
        }
    }
    for name in &cli.break_change {
        engine
            .breakpoints
            .push(Breakpoint::SignalChange(name.clone()));
        if !cli.quiet {
            println!("  breakpoint: change {}", name);
        }
    }
    for eq in &cli.break_eq {
        if let Some((name, val_hex)) = eq.split_once('=') {
            if let Ok(val) = u64::from_str_radix(
                val_hex.trim_start_matches("0x").trim_start_matches("0X"),
                16,
            ) {
                let w = engine
                    .design
                    .top
                    .signals
                    .iter()
                    .find(|s| s.name == name)
                    .map(|s| s.width)
                    .unwrap_or(32);
                engine.breakpoints.push(Breakpoint::SignalEq(
                    name.to_string(),
                    LogicVec::from_u64(val, w),
                ));
                if !cli.quiet {
                    println!("  breakpoint: {} == 0x{:X}", name, val);
                }
            }
        }
    }
    for name in &cli.watch {
        engine.watchpoints.push(Watchpoint::Signal(name.clone()));
        if !cli.quiet {
            println!("  watchpoint: {}", name);
        }
    }

    let vcd_path = cli
        .output
        .unwrap_or_else(|| format!("{}.vcd", &engine.design.top.name.to_string()));
    let mut vcd = VcdWriter::new(&vcd_path, &engine.design)
        .map_err(|e| SimError::new(None, format!("VCD creation failed: {}", e)))?;
    if cli.waveform_stream {
        vcd.stream_flush_interval = 1;
        if !cli.quiet {
            println!("Waveform streaming enabled (flush every time step)");
        }
    }
    engine.set_vcd(vcd);

    // CSV waveform setup (fast path)
    if let Some(ref csv_path) = cli.waveform_csv {
        let csv = maria::waveform::CsvWaveWriter::new(csv_path, &engine.design)
            .map_err(|e| SimError::new(None, format!("CSV creation failed: {}", e)))?;
        engine.set_csv(csv);
        if !cli.quiet {
            println!("CSV waveform: {}", csv_path);
        }
    }

    // Signal statistics setup
    if cli.signal_stats.is_some() {
        let stats = maria::waveform::SignalStats::new(&engine.design);
        engine.set_signal_stats(stats);
    }

    let mut debugger = Debugger::new(engine);

    if cli.print_tree {
        println!("\n{}", debugger.hierarchy_tree());
    }

    if cli.step && debug_mode != DebugMode::Normal {
        if !cli.quiet {
            println!("\nStep mode: running one cycle...");
        }
        debugger.step_cycle()?;
        if !cli.quiet {
            println!("{}\n", debugger.print_state_summary());
        }
        if !debugger.engine.event_log.is_empty() && !cli.quiet {
            println!("{}", debugger.print_event_log());
        }
    } else {
        if !cli.quiet {
            println!("\nStarting simulation (max time={}, vcd={})", cli.max_time, vcd_path);
        }
        debugger.run()?;
    }

    if !cli.quiet {
        println!("\nSimulation completed at time {}", debugger.engine.state.time);
    }

    // Flush runtime diagnostics (warnings, etc.)
    emit_diags(&debugger.engine.flush_diagnostics());

    if debug_mode != DebugMode::Normal && !cli.quiet {
        if debugger.engine.paused {
            println!("(debugger paused)");
        }
        if !debugger.engine.event_log.is_empty() {
            println!("\nDebug events:\n{}", debugger.print_event_log());
        }
    }

    if cli.print_state {
        println!("\n{}", debugger.print_all_signals());
    }
    for name in &cli.print_signal {
        println!("  {}", debugger.print_signal(name));
    }
    for name in &cli.timeline {
        println!("\n{}", debugger.timeline(name, cli.timeline_len));
    }

    if !cli.quiet {
        println!("VCD waveform written to '{}'", vcd_path);
    }

    // ── CSV close ──
    let _ = debugger.engine.close_csv();

    // ── Signal statistics ──
    if let Some(ref stats_path) = cli.signal_stats {
        let path = if stats_path.is_empty() {
            format!("{}.stats.txt", debugger.engine.design.top.name)
        } else {
            stats_path.clone()
        };
        if let Some(ref stats) = debugger.engine.signal_stats {
            if let Err(e) = stats.write_to_file(&path) {
                eprintln!("warning: signal stats write failed: {}", e);
            } else if !cli.quiet {
                println!("Signal statistics written to '{}'", path);
            }
        }
    }

    // ── GTKWave save file ──
    if let Some(ref gtkw_path) = cli.gtkw {
        let path = if gtkw_path.is_empty() {
            format!("{}.gtkw", debugger.engine.design.top.name)
        } else {
            gtkw_path.clone()
        };
        match maria::waveform::save_gtkw(&path, &vcd_path, &debugger.engine.design) {
            Ok(()) => {
                if !cli.quiet {
                    println!("GTKWave save file written to '{}'", path);
                }
            }
            Err(e) => eprintln!("warning: GTKW save failed: {}", e),
        }
    }

    // ── HTML waveform viewer ──
    if let Some(ref html_path) = cli.waveform_html_viewer {
        let csv_ref = cli.waveform_csv.as_deref().unwrap_or("output.csv");
        let path = if html_path.is_empty() {
            format!("{}.html", debugger.engine.design.top.name)
        } else {
            html_path.clone()
        };
        match maria::waveform::save_html_viewer(&path, csv_ref, &debugger.engine.design) {
            Ok(()) => {
                if !cli.quiet {
                    println!("HTML waveform viewer written to '{}'", path);
                }
            }
            Err(e) => eprintln!("warning: HTML viewer failed: {}", e),
        }
    }

    // Signal history stats
    if !cli.quiet && cli.signal_history_spill.is_some() {
        let stats = debugger.engine.signal_history.stats();
        println!("Signal history: {} mem entries, {} spilled to disk",
            stats.total_memory_entries, stats.total_spilled_entries);
    }

    // Save coverage database if requested
    if let Some(ref covdb_path) = cli.coverage_ucdb {
        let mut covdb = maria::simulator::coverage_db::CoverageDatabase::with_path(covdb_path);
        covdb.merge_from_engine(&debugger.engine);
        if let Err(e) = covdb.save() {
            eprintln!("warning: coverage DB save failed: {}", e);
        } else if !cli.quiet {
            println!("Coverage database saved to '{}'", covdb_path);
        }

        // Coverage threshold check
        if let Some(threshold) = cli.coverage_threshold {
            let stats = debugger.engine.coverage_stats();
            let branch_pct = stats.get("branch_percent").copied().unwrap_or(0.0);
            if branch_pct < threshold {
                let msg = format!("COVERAGE FAILED: branch coverage {:.1}% < threshold {:.1}%", branch_pct, threshold);
                eprintln!("warning: {}", msg);
                return Err(SimError::new(None, msg));
            } else if !cli.quiet {
                println!("Coverage threshold: {:.1}% >= {:.1}% ✅", branch_pct, threshold);
            }
        }
    }

    // Export HTML coverage report
    if let Some(ref html_path) = cli.coverage_html {
        let mut covdb = maria::simulator::coverage_db::CoverageDatabase::new();
        covdb.merge_from_engine(&debugger.engine);
        if let Err(e) = covdb.export_html(html_path) {
            eprintln!("warning: HTML coverage report failed: {}", e);
        } else if !cli.quiet {
            println!("HTML coverage report written to '{}'", html_path);
        }
    }

    // Save checkpoint to file if requested
    if let Some(save_path) = &cli.save {
        let _ = debugger.engine.signal_history.flush();
        let path = std::path::Path::new(save_path);
        debugger.engine.save_checkpoint(path)
            .map_err(|e| SimError::new(None, format!("checkpoint save failed: {}", e)))?;
        if !cli.quiet {
            println!("Checkpoint saved to '{}'", save_path);
        }
    }

    Ok(())
}

