use clap::Parser as ClapParser;
use std::path::PathBuf;
use std::process;

use maria::frontend::CompileSession;
use maria::SessionConfig;
use maria::debugger::Debugger;
use maria::elaboration::Elaborator;
use maria::error::{ErrorContext, SimError};
use maria::ir::LogicVec;
use maria::parser::lexer::Lexer;
use maria::parser::parser::Parser;
use maria::parser::preprocessor::Preprocessor;
use maria::read_project_file;
use maria::simulator::Breakpoint;
use maria::simulator::DebugMode;
use maria::simulator::SimulationEngine;
use maria::simulator::Watchpoint;
use maria::waveform::VcdWriter;
use rayon::prelude::*;

#[derive(ClapParser)]
#[command(name = "maria", about = "RTL Simulator untuk SystemVerilog")]
struct Cli {
    /// Input SystemVerilog file(s) — last is top module
    #[arg(
        required_unless_present = "start",
        required_unless_present = "filelist"
    )]
    files: Vec<String>,

    /// Top module name (default: first module)
    #[arg(short = 't', long = "top")]
    top: Option<String>,

    /// Maximum simulation time
    #[arg(short = 'T', long = "time", default_value = "1000")]
    max_time: u64,

    /// VCD output file (default: <module>.vcd)
    #[arg(short = 'o', long = "output")]
    output: Option<String>,

    /// Start from .maria project file (lists .sv files to compile)
    #[arg(long = "start")]
    start: bool,

    /// Add include search path
    #[arg(short = 'I', long = "incdir", num_args = 1)]
    incdirs: Vec<String>,

    /// Define preprocessor macro (NAME or NAME=VALUE)
    #[arg(short = 'D', long = "define", num_args = 1)]
    defines: Vec<String>,

    /// Read file list from file
    #[arg(short = 'f', long = "filelist")]
    filelist: Option<String>,

    /// Pass plusarg (NAME=VALUE)
    #[arg(long = "plusarg", num_args = 1)]
    plusargs: Vec<String>,

    /// Dump all signal values at each timestep
    #[arg(long = "dump-all")]
    dump_all: bool,

    /// Print tokens before parsing
    #[arg(long = "tokens")]
    print_tokens: bool,

    /// Print AST after parsing
    #[arg(long = "ast")]
    print_ast: bool,

    // ── Debug flags ──
    /// Enable debug mode (pause at breakpoints/watchpoints)
    #[arg(long = "debug")]
    debug: bool,

    /// Enable deep debug mode (with snapshot for reverse debugging)
    #[arg(long = "deep-debug")]
    deep_debug: bool,

    /// Single-step mode: run one cycle then pause
    #[arg(long = "step")]
    step: bool,

    /// Set breakpoint on cycle number
    #[arg(long = "break-cycle")]
    break_cycle: Vec<u64>,

    /// Set breakpoint on signal change (NAME)
    #[arg(long = "break-change")]
    break_change: Vec<String>,

    /// Set breakpoint on signal equality: NAME=VALUE (hex)
    #[arg(long = "break-eq")]
    break_eq: Vec<String>,

    /// Set watchpoint on signal name
    #[arg(long = "watch")]
    watch: Vec<String>,

    /// Print hierarchy tree after elaboration
    #[arg(long = "tree")]
    print_tree: bool,

    /// Print signal value after simulation
    #[arg(long = "print-signal")]
    print_signal: Vec<String>,

    /// Print all signal values after simulation
    #[arg(long = "print-state")]
    print_state: bool,

    /// Print timeline for signal after simulation
    #[arg(long = "timeline")]
    timeline: Vec<String>,

    /// Inspect memory at address with length
    #[arg(long = "mem", num_args = 2)]
    mem: Vec<String>,

    /// Snapshot interval for reverse debug (default: 1000)
    #[arg(long = "snap-interval", default_value = "1000")]
    snap_interval: u64,

    /// Print timeline entries count
    #[arg(long = "timeline-len", default_value = "20")]
    timeline_len: usize,

    /// Export coverage to UCIS XML file (default: <module>.ucis.xml)
    #[arg(long = "coverage-ucis")]
    coverage_ucis: Option<String>,

    /// Library directory to search for missing modules (-y <dir>)
    #[arg(short = 'y', long = "libdir", num_args = 1)]
    libdirs: Vec<String>,

    /// Library file containing one or more modules (-v <file>)
    #[arg(short = 'v', long = "libfile", num_args = 1)]
    libfiles: Vec<String>,

    /// Suppress preprocessor warnings (missing include files, etc.)
    #[arg(short = 'q', long = "quiet")]
    quiet: bool,

    /// Compile-only mode: parse + elaborate, skip simulation & VCD
    #[arg(long = "compile-only")]
    compile_only: bool,

    /// Use fast parallel pipeline (CompileSession + FastLexer)
    #[arg(long = "fast")]
    fast: bool,

    /// Use legacy lexer (char-based, default with new pipeline)
    #[arg(long = "legacy-lexer")]
    legacy_lexer: bool,

    /// Cache stats (show AST/HIR cache hit rates after run)
    #[arg(long = "cache-stats")]
    cache_stats: bool,

    /// Save checksums to file for change detection across runs
    #[arg(long = "checksum-file")]
    checksum_file: Option<String>,

    /// Enable profiling (show phase timings and counters)
    #[arg(long = "profile")]
    profile: bool,

    /// Force full recompile (ignore cache)
    #[arg(long = "recompile")]
    recompile: bool,

    /// Use lazy elaboration (HIR-based, on-demand)
    #[arg(long = "lazy")]
    lazy: bool,
}

fn main() {
    let cli = Cli::parse();

    let result = run(cli);
    if let Err(e) = result {
        let ctx = ErrorContext::new();
        eprint!("{}", e.format_with_context(&ctx));
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
    // Auto-detect include paths: consolidated single-pass scan
    // Walk up from each source dir's ancestors, recursively scan for SV files (depth ≤ 4)
    let mut seen_dirs = std::collections::HashSet::new();
    let mut src_dirs = std::collections::HashSet::new();
    for src in &sources {
        if let Some(dir) = std::path::Path::new(src).parent() {
            let canonical = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
            src_dirs.insert(canonical);
        }
    }
    // Recursively scan subdirectories for SV files (max depth 4), add parent dirs to search paths
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

    // ── Fast pipeline via CompileSession (skip legacy pipeline entirely) ──
    if cli.fast {
        return run_fast(cli, None);
    }

    // Combine all sources (parallel preprocessing for many files)
    let mut combined = String::new();
    let mut design_timescale = None;

    // Preprocess files in parallel using rayon
    let pp_for_parallel = &base_pp;
    let pp_results: Vec<Result<(String, Option<(String, String)>), String>> = sources
        .par_iter()
        .map(|path| {
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

    if tokens.is_empty() {
        return Err(SimError::new(None, "no tokens found (empty source?)"));
    }

    let first_source = sources.first().map(|s| s.as_str()).unwrap_or("<unknown>");
    let mut parser = Parser::new(tokens, first_source).with_source_lines(&combined);
    let mut design = parser.parse_design()?;
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
    let mut elaborator = Elaborator::new(design);
    let mut ir_design = elaborator.elaborate(top_name)?;
    ir_design.timescale = ts_for_ir;

    if !cli.quiet {
        println!(
            "Module '{}': {} signals, {} processes",
            ir_design.top.name,
            ir_design.top.signals.len(),
            ir_design.top.processes.len()
        );
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

    let mut engine = SimulationEngine::new(ir_design, cli.max_time);
    engine.debug_mode = debug_mode;
    engine.snapshot_interval = cli.snap_interval;

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
    let vcd = VcdWriter::new(&vcd_path, &engine.design)
        .map_err(|e| SimError::new(None, format!("VCD creation failed: {}", e)))?;
    engine.set_vcd(vcd);

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

    Ok(())
}

/// Run compilation + simulation using the new parallel pipeline (CompileSession + FastLexer).
fn run_fast(cli: Cli, _timescale: Option<(String, String)>) -> Result<(), SimError> {
    let sources: Vec<PathBuf> = if cli.start {
        read_project_file(".maria")?
            .into_iter()
            .map(PathBuf::from)
            .collect()
    } else {
        cli.files.iter().map(PathBuf::from).collect()
    };

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

    // ── Lazy mode: compile-only, skip full elaboration ──
    if cli.lazy && cli.compile_only {
        let (design, hir_count, index_len) = session.compile_lazy_only()?;
        if !cli.quiet {
            session.print_timing();
            println!("Modules indexed: {}", index_len);
            println!("Lazy-elaborated modules (HIR): {}", hir_count);
            if let Some(top) = &session.config.top_module {
                println!("HIR query ready: session.elaborate_lazy_module(...)");
            }
        }
        if cli.print_ast {
            println!("{:#?}", design);
        }
        return Ok(());
    }

    // ── Full pipeline: compile + elaborate (use compile_and_elaborate when --lazy) ──
    let top_name = cli.top.as_deref();

    let (design, ir_design, index_len) = if cli.lazy {
        // Use integrated compile + elaborate with lazy pre-population
        let (design, ir_design, index_len) = session.compile_and_elaborate(top_name)?;
        if !cli.quiet {
            session.print_timing();
            println!("Modules indexed: {}, lazy HIR modules: {}", index_len, session.lazy_elaborated_count());
        }
        (design, ir_design, index_len)
    } else if cli.recompile {
        if !cli.quiet { eprintln!("Forcing full recompile..."); }
        let all_sources: Vec<PathBuf> = session.config.sources.clone();
        let (design, module_index) = session.compile_incremental(&all_sources)?;
        let index_len = module_index.len();
        if !cli.quiet { session.print_timing(); }
        (design.clone(), Elaborator::new(design).elaborate(top_name)?, index_len)
    } else {
        let (design, module_index) = session.compile()?;
        let index_len = module_index.len();
        if !cli.quiet { session.print_timing(); }
        (design.clone(), Elaborator::new(design).elaborate(top_name)?, index_len)
    };

    if !cli.quiet {
        println!("Modules indexed: {}", index_len);
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

    let mut engine = SimulationEngine::new(ir_design, cli.max_time);
    engine.debug_mode = debug_mode;
    engine.snapshot_interval = cli.snap_interval;

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
    let vcd = VcdWriter::new(&vcd_path, &engine.design)
        .map_err(|e| SimError::new(None, format!("VCD creation failed: {}", e)))?;
    engine.set_vcd(vcd);

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

    Ok(())
}

