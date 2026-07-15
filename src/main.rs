use std::process;
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
use maria::error::SimError;
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
}

fn main() {
    let cli = Cli::parse();

    let result = run(cli);
    if let Err(e) = result {
        eprintln!("Error: {}", e);
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
    // Auto-detect include paths: walk up from each source looking for hw/ip/*/rtl
    for src in &sources {
        if let Some(dir) = std::path::Path::new(src).parent() {
            let mut candidate = Some(dir.to_path_buf());
            while let Some(d) = candidate {
                let rtl_dir = d.join("hw").join("ip").join("prim").join("rtl");
                if rtl_dir.join("prim_assert.sv").exists() {
                    base_pp.add_search_path(rtl_dir.to_str().unwrap());
                    break;
                }
                candidate = d.parent().map(|p| p.to_path_buf());
            }
        }
    }

    // Combine all sources
    let mut combined = String::new();
    for path in &sources {
        let mut pp = base_pp.clone();
        let processed = pp.preprocess_file(path)
            .map_err(|e| SimError::new(None, format!("preprocessor '{}': {}", path, e)))?;
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
    let mut parser = Parser::new(tokens, first_source);
    let design = parser.parse_design()?;

    if cli.print_ast {
        println!("{:#?}", design);
    }

    if design.modules.is_empty() {
        return Err(SimError::new(None, "no modules found in design"));
    }

    let top_name = cli.top.as_deref();
    println!("Compiling design ({} files sources)...", sources.len());
    let mut elaborator = Elaborator::new(design);
    let ir_design = elaborator.elaborate(top_name)?;

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
