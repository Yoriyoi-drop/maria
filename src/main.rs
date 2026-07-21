use std::process;
use std::path::PathBuf;
use clap::Parser as ClapParser;

use maria::parser::lexer::Lexer;
use maria::parser::parser::Parser;
use maria::parser::preprocessor::Preprocessor;
use maria::elaboration::Elaborator;
use maria::simulator::SimulationEngine;
use maria::simulator::DebugMode;
use maria::simulator::Breakpoint;
use maria::simulator::Watchpoint;
use maria::waveform::VcdWriter;
use maria::ir::LogicVec;
use maria::error::{SimError, ErrorContext};
use maria::read_project_file;
use maria::debugger::Debugger;

#[derive(ClapParser)]
#[command(name = "maria", about = "RTL Simulator untuk SystemVerilog")]
struct Cli {
    /// Input SystemVerilog file(s) — last is top module
    #[arg(required_unless_present = "start", required_unless_present = "filelist")]
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
    // Auto-detect include paths:
    //   1. Walk up from each source, at each ancestor scan subdirs for .svh/.sv files
    //   2. Scan each source dir's own subtree (depth ≤ 3) for SV include files
    let mut auto_seen = std::collections::HashSet::new();
    fn scan_for_includes(dir: &std::path::Path, seen: &mut std::collections::HashSet<PathBuf>, pp: &mut Preprocessor, depth: usize) {
        if depth > 3 { return; }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                if let Ok(ft) = entry.file_type() {
                    let path = entry.path();
                    if ft.is_dir() {
                        scan_for_includes(&path, seen, pp, depth + 1);
                    } else if ft.is_file() {
                        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                        if (ext == "svh" || ext == "sv") && seen.insert(path.clone()) {
                            if let Some(parent) = path.parent() {
                                pp.add_search_path(parent.to_str().unwrap());
                            }
                        }
                    }
                }
            }
        }
    }
    let mut src_dirs = std::collections::HashSet::new();
    for src in &sources {
        if let Some(dir) = std::path::Path::new(src).parent() {
            let canonical = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
            if !src_dirs.insert(canonical.clone()) { continue; }
            // Walk up from source dir, at each level scan ALL subdirs for SV files
            let mut anc = Some(canonical.clone());
            while let Some(ref d) = anc {
                if let Ok(entries) = std::fs::read_dir(d) {
                    for entry in entries.flatten() {
                        if let Ok(ft) = entry.file_type() {
                            if ft.is_dir() {
                                let subdir = entry.path();
                                if auto_seen.insert(subdir.clone()) {
                                    if let Ok(sub_entries) = std::fs::read_dir(&subdir) {
                                        let has_sv = sub_entries.flatten().any(|e| {
                                            let p = e.path();
                                            let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("");
                                            ext == "svh" || ext == "sv"
                                        });
                                        if has_sv {
                                            base_pp.add_search_path(subdir.to_str().unwrap());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                anc = d.parent().map(|p| p.to_path_buf());
            }
        }
    }
    // Also scan each unique source dir's own subtree (depth ≤ 2) for SV files
    for src_dir in src_dirs.iter() {
        scan_for_includes(src_dir, &mut auto_seen, &mut base_pp, 0);
    }

    // Combine all sources
    let mut combined = String::new();
    let mut design_timescale = None;
    for path in &sources {
        let mut pp = base_pp.clone();
        let processed = pp.preprocess_file(path)
            .map_err(|e| SimError::new(None, format!("preprocessor '{}': {}", path, e)))?;
        if pp.timescale.is_some() {
            design_timescale = pp.timescale.clone();
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

    if cli.print_ast {
        println!("{:#?}", design);
    }

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
                                let combined_lib = format!("`line 1 \"{}\"\n{}", path.display(), processed);
                                let mut lexer = Lexer::new(&combined_lib);
                                let mut lib_tokens = Vec::new();
                                loop {
                                    let (tok, line, col) = lexer.next_token();
                                    if tok == maria::parser::lexer::Token::Eof { break; }
                                    lib_tokens.push((tok, line, col));
                                }
                                let mut parser = Parser::new(lib_tokens, path.to_str().unwrap_or("<lib>"));
                                parser = parser.with_source_lines(&combined_lib);
                                match parser.parse_design() {
                                    Ok(lib_design) => {
                                        for m in lib_design.modules {
                                            if !design.modules.iter().any(|dm| dm.name == m.name) {
                                                design.modules.push(m);
                                            }
                                        }
                                    }
                                    Err(e) => eprintln!("warning: library file '{}' parse error: {}", path.display(), e),
                                }
                            }
                            Err(e) => eprintln!("warning: library file '{}' preprocess error: {}", path.display(), e),
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
                    if tok == maria::parser::lexer::Token::Eof { break; }
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
            Err(e) => eprintln!("warning: library file '{}' preprocess error: {}", libfile, e),
        }
    }

    if design.modules.is_empty() {
        return Err(SimError::new(None, "no modules found in design"));
    }

    let top_name = cli.top.as_deref();
    println!("Compiling design ({} files sources)...", sources.len());
    let mut elaborator = Elaborator::new(design);
    let mut ir_design = elaborator.elaborate(top_name)?;
    ir_design.timescale = ts_for_ir;

    println!("Module '{}': {} signals, {} processes",
        ir_design.top.name,
        ir_design.top.signals.len(),
        ir_design.top.processes.len());

    // ── Setup ──
    let debug_mode = if cli.deep_debug {
        DebugMode::DeepDebug
    } else if cli.debug || cli.step || !cli.break_cycle.is_empty()
        || !cli.break_change.is_empty() || !cli.break_eq.is_empty()
        || !cli.watch.is_empty() {
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
        println!("  breakpoint: cycle {}", c);
    }
    for name in &cli.break_change {
        engine.breakpoints.push(Breakpoint::SignalChange(name.clone()));
        println!("  breakpoint: change {}", name);
    }
    for eq in &cli.break_eq {
        if let Some((name, val_hex)) = eq.split_once('=') {
            if let Ok(val) = u64::from_str_radix(val_hex.trim_start_matches("0x").trim_start_matches("0X"), 16) {
                let w = engine.design.top.signals.iter()
                    .find(|s| s.name == name).map(|s| s.width).unwrap_or(32);
                engine.breakpoints.push(Breakpoint::SignalEq(name.to_string(), LogicVec::from_u64(val, w)));
                println!("  breakpoint: {} == 0x{:X}", name, val);
            }
        }
    }
    // Apply watchpoints
    for name in &cli.watch {
        engine.watchpoints.push(Watchpoint::Signal(name.clone()));
        println!("  watchpoint: {}", name);
    }

    // VCD setup
    let vcd_path = cli.output
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
        println!("\nStep mode: running one cycle...");
        debugger.step_cycle()?;
        println!("{}\n", debugger.print_state_summary());
        if !debugger.engine.event_log.is_empty() {
            println!("{}", debugger.print_event_log());
        }
    } else {
        println!("\nStarting simulation (max time={}, vcd={})", cli.max_time, vcd_path);
        debugger.run()?;
    }

    // ── Post-simulation output ──
    println!("\nSimulation completed at time {}", debugger.engine.state.time);

    if debug_mode != DebugMode::Normal {
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
        if let (Ok(addr), Ok(len)) = (u64::from_str_radix(cli.mem[0].trim_start_matches("0x").trim_start_matches("0X"), 16), cli.mem[1].parse::<usize>()) {
            println!("\n{}", debugger.memory_inspect(addr, len));
        }
    }

    println!("VCD waveform written to '{}'", vcd_path);

    // UCIS coverage export
    if let Some(ref ucis_path) = cli.coverage_ucis {
        let path = if ucis_path.is_empty() {
            format!("{}.ucis.xml", debugger.engine.design.top.name)
        } else {
            ucis_path.clone()
        };
        match debugger.engine.export_coverage_ucis(&path) {
            Ok(()) => println!("UCIS coverage written to '{}'", path),
            Err(e) => eprintln!("UCIS export failed: {}", e),
        }
    }

    Ok(())
}
