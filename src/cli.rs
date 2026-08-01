//! CLI argument definitions for Maria RTL Simulator.
//! Separated from main.rs for clarity.

use clap::Parser as ClapParser;

#[derive(ClapParser)]
#[command(name = "maria", about = "RTL Simulator untuk SystemVerilog")]
pub struct Cli {
    /// Input SystemVerilog file(s) — last is top module
    #[arg(
        required_unless_present = "start",
        required_unless_present = "filelist",
        required_unless_present = "gui"
    )]
    pub files: Vec<String>,

    /// Top module name (default: first module)
    #[arg(short = 't', long = "top")]
    pub top: Option<String>,

    /// Maximum simulation time
    #[arg(short = 'T', long = "time", default_value = "1000")]
    pub max_time: u64,

    /// VCD/FST output file (default: <module>.vcd)
    #[arg(short = 'o', long = "output")]
    pub output: Option<String>,

    /// Enable waveform streaming to disk (flush after each time step)
    /// Allows external tools to read waveform during simulation.
    #[arg(long = "waveform-stream")]
    pub waveform_stream: bool,

    /// Start from .maria project file (lists .sv files to compile)
    #[arg(long = "start")]
    pub start: bool,

    /// Launch the native GUI (egui) — requires --features gui
    #[arg(long = "gui")]
    pub gui: bool,

    /// Add include search path
    #[arg(short = 'I', long = "incdir", num_args = 1)]
    pub incdirs: Vec<String>,

    /// Define preprocessor macro (NAME or NAME=VALUE)
    #[arg(short = 'D', long = "define", num_args = 1)]
    pub defines: Vec<String>,

    /// Read file list from file
    #[arg(short = 'f', long = "filelist")]
    pub filelist: Option<String>,

    /// Pass plusarg (NAME=VALUE)
    #[arg(long = "plusarg", num_args = 1)]
    pub plusargs: Vec<String>,

    /// Dump all signal values at each timestep
    #[arg(long = "dump-all")]
    pub dump_all: bool,

    /// Print tokens before parsing
    #[arg(long = "tokens")]
    pub print_tokens: bool,

    /// Print AST after parsing
    #[arg(long = "ast")]
    pub print_ast: bool,

    // ── Debug flags ──
    /// Enable debug mode (pause at breakpoints/watchpoints)
    #[arg(long = "debug")]
    pub debug: bool,

    /// Enable deep debug mode (with snapshot for reverse debugging)
    #[arg(long = "deep-debug")]
    pub deep_debug: bool,

    /// Single-step mode: run one cycle then pause
    #[arg(long = "step")]
    pub step: bool,

    /// Set breakpoint on cycle number
    #[arg(long = "break-cycle")]
    pub break_cycle: Vec<u64>,

    /// Set breakpoint on signal change (NAME)
    #[arg(long = "break-change")]
    pub break_change: Vec<String>,

    /// Set breakpoint on signal equality: NAME=VALUE (hex)
    #[arg(long = "break-eq")]
    pub break_eq: Vec<String>,

    /// Set watchpoint on signal name
    #[arg(long = "watch")]
    pub watch: Vec<String>,

    /// Print hierarchy tree after elaboration
    #[arg(long = "tree")]
    pub print_tree: bool,

    /// Print signal value after simulation
    #[arg(long = "print-signal")]
    pub print_signal: Vec<String>,

    /// Print all signal values after simulation
    #[arg(long = "print-state")]
    pub print_state: bool,

    /// Print timeline for signal after simulation
    #[arg(long = "timeline")]
    pub timeline: Vec<String>,

    /// Inspect memory at address with length
    #[arg(long = "mem", num_args = 2)]
    pub mem: Vec<String>,

    /// Snapshot interval for reverse debug (default: 1000)
    #[arg(long = "snap-interval", default_value = "1000")]
    pub snap_interval: u64,

    /// Print timeline entries count
    #[arg(long = "timeline-len", default_value = "20")]
    pub timeline_len: usize,

    /// Export coverage to UCIS XML file (default: <module>.ucis.xml)
    #[arg(long = "coverage-ucis")]
    pub coverage_ucis: Option<String>,

    /// Library directory to search for missing modules (-y <dir>)
    #[arg(short = 'y', long = "libdir", num_args = 1)]
    pub libdirs: Vec<String>,

    /// Library file containing one or more modules (-v <file>)
    #[arg(short = 'v', long = "libfile", num_args = 1)]
    pub libfiles: Vec<String>,

    /// Suppress preprocessor warnings (missing include files, etc.)
    #[arg(short = 'q', long = "quiet")]
    pub quiet: bool,

    /// X-propagation mode: optimistic, pessimistic, or x-anywhere
    #[arg(long = "xprop", default_value = "pessimistic")]
    pub xprop: String,

    /// Compile-only mode: parse + elaborate, skip simulation & VCD
    #[arg(long = "compile-only")]
    pub compile_only: bool,

    /// Use fast parallel pipeline (CompileSession + FastLexer)
    #[arg(long = "fast")]
    pub fast: bool,

    /// Use legacy lexer (char-based, default with new pipeline)
    #[arg(long = "legacy-lexer")]
    pub legacy_lexer: bool,

    /// Cache stats (show AST/HIR cache hit rates after run)
    #[arg(long = "cache-stats")]
    pub cache_stats: bool,

    /// Save checksums to file for change detection across runs
    #[arg(long = "checksum-file")]
    pub checksum_file: Option<String>,

    /// Enable profiling (show phase timings and counters)
    #[arg(long = "profile")]
    pub profile: bool,

    /// Print simulation performance dashboard (delta cycles, events, throughput)
    #[arg(long = "perf-dashboard")]
    pub perf_dashboard: bool,

    /// Force full recompile (ignore cache)
    #[arg(long = "recompile")]
    pub recompile: bool,

    /// Use lazy elaboration (HIR-based, on-demand)
    #[arg(long = "lazy")]
    pub lazy: bool,

    /// Use packed 4-state eval (SIMD-ready bitmask ops for bitwise operations)
    #[arg(long = "packed", short = 'P')]
    pub packed: bool,

    /// Use DAG-parallel process evaluation (parallel simulation via rayon)
    #[arg(long = "parallel", short = 'L')]
    pub parallel: bool,

    /// Use cycle-based simulation fusion (clock-gated domain fusion)
    #[arg(long = "cycle-fusion")]
    pub cycle_fusion: bool,

    /// Enable body-level MIR JIT (compiled-code simulation for combinational processes)
    #[arg(long = "jit-body")]
    pub jit_body: bool,

    /// Enable formal verification with Z3 (Bounded Model Checking)
    #[arg(long = "formal")]
    pub formal: bool,

    /// Maximum unrolling bound for BMC (default: 20)
    #[arg(long = "formal-bound", default_value = "20")]
    pub formal_bound: u64,

    /// DPI shared library to load (can be specified multiple times)
    #[arg(long = "dpi-lib", num_args = 1)]
    pub dpi_libs: Vec<String>,

    /// Save simulation checkpoint to file (after sim, or at --break-cycle)
    #[arg(long = "save")]
    pub save: Option<String>,

    /// Restore simulation checkpoint from file (overrides initial state)
    #[arg(long = "restore")]
    pub restore: Option<String>,

    /// Enable signal history disk spill (path to spill file)
    #[arg(long = "signal-history-spill")]
    pub signal_history_spill: Option<String>,

    /// Coverage database file path (UCDB binary format, auto-merge on load)
    #[arg(long = "coverage-ucdb")]
    pub coverage_ucdb: Option<String>,

    /// Run as LSP server (stdin/stdout JSON-RPC transport)
    #[arg(long = "lsp")]
    pub lsp: bool,

    /// SDF annotation file path (Standard Delay Format for gate-level timing)
    #[arg(long = "sdf")]
    pub sdf: Option<String>,

    /// Minimum branch coverage threshold (%). Exit with error if below.
    #[arg(long = "coverage-threshold")]
    pub coverage_threshold: Option<f64>,

    /// Export coverage report as HTML file
    #[arg(long = "coverage-html")]
    pub coverage_html: Option<String>,

    /// Co-simulation port (TCP) for VHDL/SystemVerilog co-simulation bridge
    #[arg(long = "cosim-port")]
    pub cosim_port: Option<u16>,

    /// Comma-separated list of signal names to expose for co-simulation
    #[arg(long = "cosim-signals")]
    pub cosim_signals: Option<String>,

    /// SDF timing mode: min, typ (default), or max
    #[arg(long = "timing-mode", default_value = "typ")]
    pub timing_mode: String,

    /// UPF (Unified Power Format) file for power-aware simulation
    #[arg(long = "upf")]
    pub upf: Option<String>,

    /// Remote cache directory (filesystem-based, shared across CI builds)
    /// Content-addressed: entries stored by checksum under this directory.
    /// Use a shared NFS volume or CI artifact mount for cross-build caching.
    #[arg(long = "cache-remote-dir")]
    pub cache_remote_dir: Option<String>,

    /// Remote cache sync mode: none, manual, read-through, write-through, or read-write
    /// Default: read-write (auto-sync on both miss and insert)
    #[arg(long = "cache-remote-sync", default_value = "read-write")]
    pub cache_remote_sync: String,

    /// Clear all cache (local + remote) before starting
    #[arg(long = "cache-clear")]
    pub cache_clear: bool,

    /// CSV waveform output file path (signal values as comma-separated values)
    /// Compatible with spreadsheet tools and the built-in HTML viewer.
    #[arg(long = "waveform-csv")]
    pub waveform_csv: Option<String>,

    /// Generate GTKWave save file (.gtkw) for the waveform output
    /// Automatically referenced to VCD/FST path. Pass path or just `--gtkw` for auto-name.
    #[arg(long = "gtkw")]
    pub gtkw: Option<String>,

    /// Write signal statistics report (toggle counts, transitions, activity)
    /// Useful for power estimation and signal activity analysis.
    #[arg(long = "signal-stats")]
    pub signal_stats: Option<String>,

    /// Generate standalone HTML waveform viewer (requires --waveform-csv)
    /// The HTML file includes inline CSS+JS, open in browser to view CSV data.
    #[arg(long = "waveform-html-viewer")]
    pub waveform_html_viewer: Option<String>,

    /// CDC (Clock-Domain Crossing) analysis report file
    /// Detects and reports unsynchronized signal crossings between clock domains.
    #[arg(long = "cdc-report")]
    pub cdc_report: Option<String>,

    /// Run as distributed master node (coordinate partitions)
    #[arg(long = "dist-master")]
    pub dist_master: bool,

    /// Run as distributed slave node (connect to master)
    #[arg(long = "dist-slave")]
    pub dist_slave: bool,

    /// Distributed simulation port (default: 9876)
    #[arg(long = "dist-port", default_value = "9876")]
    pub dist_port: u16,

    /// Number of partitions for distributed simulation (master)
    #[arg(long = "num-partitions", default_value = "1")]
    pub num_partitions: usize,

    /// Master host for distributed slave
    #[arg(long = "master-host", default_value = "127.0.0.1")]
    pub master_host: String,

    /// Use hierarchical timing wheel for O(1) event scheduling.
    /// Replaces Vec<Vec<RegionEvent>> with a 3-level timing wheel
    /// that eliminates O(E) event filtering per delta cycle.
    #[arg(long = "use-timing-wheel")]
    pub use_timing_wheel: bool,

    /// Glitch detection window (in time units). 0 = disabled (default).
    /// Detects A→B→A pulses where a signal reverts to its previous value
    /// within this window and reports a WR0302 warning.
    #[arg(long = "glitch-window", default_value = "0")]
    pub glitch_window: u64,
}
