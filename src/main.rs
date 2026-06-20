use std::process;
use clap::Parser as ClapParser;

use maria::parser::lexer::Lexer;
use maria::parser::parser::Parser;
use maria::parser::preprocessor::Preprocessor;
use maria::elaboration::Elaborator;
use maria::simulator::SimulationEngine;
use maria::waveform::VcdWriter;
use maria::error::SimError;
use maria::read_project_file;

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

    /// Dump all signal values at each timestep
    #[arg(long = "dump-all")]
    dump_all: bool,

    /// Print tokens before parsing
    #[arg(long = "tokens")]
    print_tokens: bool,

    /// Print AST after parsing
    #[arg(long = "ast")]
    print_ast: bool,
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

    let mut parser = Parser::new(tokens);
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

    let mut engine = SimulationEngine::new(ir_design, cli.max_time);
    let vcd_path = cli.output
        .unwrap_or_else(|| format!("{}.vcd", engine.design.top.name));

    let vcd = VcdWriter::new(&vcd_path, &engine.design)
        .map_err(|e| SimError::new(None, format!("VCD creation failed: {}", e)))?;
    engine.set_vcd(vcd);

    println!("\nStarting simulation (max time={}, vcd={})", cli.max_time, vcd_path);
    engine.run()?;

    println!("\nSimulation completed at time {}", engine.state.time);
    println!("VCD waveform written to '{}'", vcd_path);

    Ok(())
}
