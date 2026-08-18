//! CLI argument definitions for Maria RTL Simulator.
//! Separated from main.rs for clarity.

use clap::Parser as ClapParser;
use clap::Subcommand;

/// Subcommands `maria`.
#[derive(Subcommand, Clone)]
pub enum MariaCmd {
    /// Bersihkan database MICD (.maria/database)
    Clean,

    // ── Tools terminal (tools.md) ──
    /// minspect — inspeksi struktur project (stats/modules/hierarchy/...)
    #[command(alias = "minspect")]
    Inspect(MinspectArgs),
    /// mlint — static RTL linter (unused signal, width, latch, loop, FSM)
    #[command(alias = "mlint")]
    Lint(MlintArgs),
    /// melab — standalone elaborator (parameter resolve, generate, hierarchy)
    #[command(alias = "melab")]
    Elab(MelabArgs),
    /// msim — simulator (VCD/FST/wave/assertion/coverage)
    #[command(alias = "msim")]
    Sim(MsimArgs),
    /// mcov — coverage analyzer → coverage.json / coverage.html
    #[command(alias = "mcov")]
    Cov(McovArgs),
    /// mwave — wave utility (merge/export/filter VCD)
    #[command(alias = "mwave")]
    Wave(MwaveArgs),
    /// mfmt — formatter Verilog/SystemVerilog
    #[command(alias = "mfmt")]
    Fmt(MfmtArgs),
    /// mgen — generate SystemVerilog (.sv/.svh) dari Maria HDL (.mv)
    #[command(alias = "mgen")]
    Gen(MgenArgs),
    /// mprof — performance profiler pipeline (lexer→parser→elab→sim)
    #[command(alias = "mprof")]
    Prof(MprofArgs),
    /// mcheck — project health checker (missing file, circular include, deps)
    #[command(alias = "mcheck")]
    Check(McheckArgs),
    /// mbench — benchmark tool (compile speed, memori, CPU, throughput)
    #[command(alias = "mbench")]
    Bench(MbenchArgs),
    /// synth — Maria synthesis (RTL → SIR → netlist gate-level, SYNTHESIS.md)
    /// Nama lama: `msynth` (alias)
    #[command(name = "synth", alias = "msynth")]
    Synth(SynthArgs),

    // ── Emulator (EMULATOR.md) — Hardware-Software Emulator ──
    /// emu — Maria emulator; R0: MHIR extraction (device/register/clock/
    /// reset/memory + back-pointer) dan memory map dump
    #[command(name = "emu")]
    Emu(EmuArgs),
}

/// minspect — Maria Inspect.
#[derive(clap::Args, Clone)]
pub struct MinspectArgs {
    /// Target input: file .sv, direktori, atau file list .f
    /// Subcommand output (stats/modules/hierarchy/...) boleh diletakkan
    /// di posisi pertama (mis. `minspect stats rtl/`).
    #[arg(required = true)]
    pub targets: Vec<String>,

    /// Add include search path
    #[arg(short = 'I', long = "incdir", num_args = 1)]
    pub incdirs: Vec<String>,

    /// Define preprocessor macro
    #[arg(short = 'D', long = "define", num_args = 1)]
    pub defines: Vec<String>,

    /// Top module name
    #[arg(short = 't', long = "top")]
    pub top: Option<String>,

    /// Output sebagai JSON
    #[arg(long)]
    pub json: bool,
}

/// mlint — Static RTL Linter.
#[derive(clap::Args, Clone)]
pub struct MlintArgs {
    /// Target input: file .sv, direktori, atau file list .f
    #[arg(required = true)]
    pub targets: Vec<String>,

    /// Tambahkan include search path
    #[arg(short = 'I', long = "incdir", num_args = 1)]
    pub incdirs: Vec<String>,

    /// Define preprocessor macro
    #[arg(short = 'D', long = "define", num_args = 1)]
    pub defines: Vec<String>,

    /// Aktifkan semua check
    #[arg(long)]
    pub all: bool,

    /// Check: unused signal
    #[arg(long)]
    pub unused: bool,

    /// Check: width mismatch
    #[arg(long)]
    pub width: bool,

    /// Check: latch detection
    #[arg(long)]
    pub latch: bool,

    /// Check: combinational loop
    #[arg(long)]
    pub loop_check: bool,

    /// Check: FSM state register
    #[arg(long)]
    pub fsm: bool,

    /// Suppress output sukses
    #[arg(short = 'q', long)]
    pub quiet: bool,
}

/// melab — Standalone Elaborator.
#[derive(clap::Args, Clone)]
pub struct MelabArgs {
    /// Input file .sv (bisa lebih dari satu)
    #[arg(required = true)]
    pub files: Vec<String>,

    /// Top module name (default: module pertama)
    #[arg(short = 't', long = "top")]
    pub top: Option<String>,

    /// Cetak hierarchy tree
    #[arg(long)]
    pub tree: bool,

    /// Cetak parameter ter-resolve per module
    #[arg(long)]
    pub params: bool,

    /// Cetak sinyal per module
    #[arg(long)]
    pub signals: bool,

    /// SIM-22: analisis reset-domain crossing (RDC)
    #[arg(long = "reset-domain")]
    pub reset_domain: bool,

    /// Baca hasil elaborasi dari cache pipeline (db.md "5. elaborate/",
    /// "16. generate/") tanpa menjalankan elaborator — instance + port
    /// binding + parameter override + proses + net resolution + blok generate.
    #[arg(long = "from-cache")]
    pub from_cache: bool,

    /// Tambahkan include search path (identitas project cache)
    #[arg(short = 'I', long = "incdir", num_args = 1)]
    pub incdirs: Vec<String>,

    /// Define preprocessor macro (identitas project cache)
    #[arg(short = 'D', long = "define", num_args = 1)]
    pub defines: Vec<String>,
}

/// msim — Simulator.
#[derive(clap::Args, Clone)]
pub struct MsimArgs {
    /// Input file .sv (bisa lebih dari satu)
    #[arg(required = true)]
    pub files: Vec<String>,

    /// Maximum simulation time (default: unlimited — run until $finish/$fatal)
    #[arg(short = 'T', long = "max-time", alias = "time", value_name = "NS")]
    pub max_time: Option<u64>,

    /// Force simulation + VCD even when elaboration errors/skipped modules exist
    /// (default: Maria refuses to simulate until the design elaborates cleanly)
    #[arg(long = "force-sim")]
    pub force_sim: bool,

    /// Top module name
    #[arg(short = 't', long = "top")]
    pub top: Option<String>,

    /// VCD/FST output file
    #[arg(short = 'o', long = "output")]
    pub output: Option<String>,

    /// Tulis FST waveform juga
    #[arg(long)]
    pub fst: bool,

    /// Cetak ringkasan assertion setelah simulasi
    #[arg(long)]
    pub assertions: bool,

    /// Cetak ringkasan coverage setelah simulasi
    #[arg(long)]
    pub coverage: bool,

    /// Tambahkan include search path
    #[arg(short = 'I', long = "incdir", num_args = 1)]
    pub incdirs: Vec<String>,

    /// Define preprocessor macro
    #[arg(short = 'D', long = "define", num_args = 1)]
    pub defines: Vec<String>,
}

/// mcov — Coverage Analyzer.
#[derive(clap::Args, Clone)]
pub struct McovArgs {
    /// Input file .sv (bisa lebih dari satu)
    #[arg(required = true)]
    pub files: Vec<String>,

    /// Maximum simulation time (default: unlimited — run until $finish/$fatal)
    #[arg(short = 'T', long = "max-time", alias = "time", value_name = "NS")]
    pub max_time: Option<u64>,

    /// Force simulation + VCD even when elaboration errors/skipped modules exist
    /// (default: Maria refuses to simulate until the design elaborates cleanly)
    #[arg(long = "force-sim")]
    pub force_sim: bool,

    /// Top module name
    #[arg(short = 't', long = "top")]
    pub top: Option<String>,

    /// Prefix output (default: <top>). Menghasilkan <prefix>.coverage.json
    /// dan <prefix>.coverage.html
    #[arg(short = 'o', long = "output")]
    pub output: Option<String>,

    /// Tulis coverage.json
    #[arg(long)]
    pub json: bool,

    /// Tulis coverage.html
    #[arg(long)]
    pub html: bool,

    /// Threshold branch coverage (%) — exit error bila di bawah
    #[arg(long)]
    pub threshold: Option<f64>,

    /// Tambahkan include search path
    #[arg(short = 'I', long = "incdir", num_args = 1)]
    pub incdirs: Vec<String>,

    /// Define preprocessor macro
    #[arg(short = 'D', long = "define", num_args = 1)]
    pub defines: Vec<String>,
}

/// mwave — Wave Utility.
#[derive(clap::Args, Clone)]
pub struct MwaveArgs {
    /// Subcommand: merge, export, filter
    #[command(subcommand)]
    pub cmd: MwaveCmd,
}

/// mwave subcommand.
#[derive(Subcommand, Clone)]
pub enum MwaveCmd {
    /// Gabungkan beberapa VCD menjadi satu (offset waktu otomatis kumulatif)
    Merge {
        /// VCD input (2+)
        #[arg(required = true)]
        inputs: Vec<String>,
        /// Output VCD
        #[arg(short = 'o', long = "output")]
        output: Option<String>,
    },
    /// Ekspor VCD ke format lain (csv/txt)
    Export {
        /// VCD input
        #[arg(required = true)]
        input: String,
        /// Format output: csv (default) | txt
        #[arg(short = 'f', long = "format", default_value = "csv")]
        format: String,
        /// Output file
        #[arg(short = 'o', long = "output")]
        output: Option<String>,
    },
    /// Filter subset sinyal dari VCD
    Filter {
        /// VCD input
        #[arg(required = true)]
        input: String,
        /// Sinyal yang dipertahankan (koma/space terpisah)
        #[arg(required = true)]
        signals: Vec<String>,
        /// Output VCD
        #[arg(short = 'o', long = "output")]
        output: Option<String>,
    },
    /// Bandingkan dua VCD (perbedaan nilai per signal)
    Compare {
        /// VCD pertama
        #[arg(required = true)]
        a: String,
        /// VCD kedua
        #[arg(required = true)]
        b: String,
    },
    /// Cari sinyal by pola wildcard (* dan ?)
    Search {
        /// VCD input
        #[arg(required = true)]
        input: String,
        /// Pola (koma/space terpisah, dukung * dan ?)
        #[arg(required = true)]
        patterns: Vec<String>,
    },
    /// Index hierarki scope + sinyal
    Tree {
        /// VCD input
        #[arg(required = true)]
        input: String,
    },
    /// Statistik aktivitas per sinyal (toggle, transitions, activity%)
    Stats {
        /// VCD input
        #[arg(required = true)]
        input: String,
    },
}

/// mfmt — Formatter.
#[derive(clap::Args, Clone)]
pub struct MfmtArgs {
    /// File .sv untuk diformat
    #[arg(required = true)]
    pub files: Vec<String>,

    /// Tulis kembali ke file (tanpa ini hanya ke stdout)
    #[arg(short = 'i', long)]
    pub inplace: bool,

    /// Lebar indentasi (default: 4)
    #[arg(long, default_value = "4")]
    pub indent: usize,

    /// Periksa saja — report file yang berbeda format (exit 1 bila ada)
    #[arg(long)]
    pub check: bool,
}

/// mgen — Generator SystemVerilog dari Maria HDL (.mv).
#[derive(clap::Args, Clone)]
pub struct MgenArgs {
    /// Input: file .mv atau direktori (recursive scan *.mv)
    #[arg(required = true)]
    pub targets: Vec<String>,

    /// Direktori output (default: di samping file input)
    #[arg(short = 'o', long = "output")]
    pub output: Option<String>,

    /// Print .sv ke stdout (debug, satu file saja)
    #[arg(long)]
    pub stdout: bool,

    /// Verifikasi output up-to-date — exit 1 bila ada file yang beda (CI)
    #[arg(long)]
    pub check: bool,

    /// Hanya generate .svh
    #[arg(long = "svh-only")]
    pub svh_only: bool,

    /// Lewati type-check (E2001–E2007) — untuk konstruk eksternal yang
    /// belum dipahami checker
    #[arg(long)]
    pub no_check: bool,

    /// Hanya generate .sv
    #[arg(long = "sv-only")]
    pub sv_only: bool,

    /// Report per-file yang diproses
    #[arg(long)]
    pub verbose: bool,
}

/// mprof — Performance Profiler.
#[derive(clap::Args, Clone)]
pub struct MprofArgs {
    /// Target input: file .sv, direktori, atau file list .f
    #[arg(required = true)]
    pub targets: Vec<String>,

    /// Maximum simulation time (default: unlimited — run until $finish/$fatal)
    #[arg(short = 'T', long = "max-time", alias = "time", value_name = "NS")]
    pub max_time: Option<u64>,

    /// Force simulation + VCD even when elaboration errors/skipped modules exist
    /// (default: Maria refuses to simulate until the design elaborates cleanly)
    #[arg(long = "force-sim")]
    pub force_sim: bool,

    /// Top module name
    #[arg(short = 't', long = "top")]
    pub top: Option<String>,

    /// Baca profil build terakhir dari cache pipeline (db.md "20. profile/") —
    /// tanpa compile/simulasi. Menampilkan bottleneck + rekomendasi.
    #[arg(long = "cached")]
    pub cached: bool,

    /// Tambahkan include search path
    #[arg(short = 'I', long = "incdir", num_args = 1)]
    pub incdirs: Vec<String>,

    /// Define preprocessor macro
    #[arg(short = 'D', long = "define", num_args = 1)]
    pub defines: Vec<String>,
}

/// mcheck — Project Health Checker.
#[derive(clap::Args, Clone)]
pub struct McheckArgs {
    /// Target input: file .sv, direktori, atau file list .f
    #[arg(required = true)]
    pub targets: Vec<String>,

    /// Aktifkan semua check
    #[arg(long)]
    pub all: bool,

    /// Check: missing file (`include / file list)
    #[arg(long)]
    pub missing: bool,

    /// Check: circular include
    #[arg(long)]
    pub circular: bool,

    /// Check: unresolved dependency module
    #[arg(long)]
    pub deps: bool,

    /// Check: module instantiation cycle
    #[arg(long)]
    pub cycles: bool,

    /// Check: inkonsistensi timescale
    #[arg(long)]
    pub timescale: bool,

    /// bandingkan AST file pertama dgn file kedua (structural
    /// diff utk regression) — `mcheck a.sv --ast-diff b.sv`.
    #[arg(long = "ast-diff")]
    pub ast_diff: Option<String>,
}

/// mbench — Benchmark Tool.
#[derive(clap::Args, Clone)]
pub struct MbenchArgs {
    /// Target input: file .sv, direktori, atau file list .f
    #[arg(required = true)]
    pub targets: Vec<String>,

    /// Jumlah run (default: 3)
    #[arg(short = 'n', long, default_value = "3")]
    pub runs: usize,

    /// Tambahkan include search path
    #[arg(short = 'I', long = "incdir", num_args = 1)]
    pub incdirs: Vec<String>,

    /// Define preprocessor macro
    #[arg(short = 'D', long = "define", num_args = 1)]
    pub defines: Vec<String>,
}

/// synth — Maria Synthesis.
#[derive(clap::Args, Clone)]
pub struct SynthArgs {
    /// Target input: file .sv, direktori, atau file list (.f/.maria)
    #[arg(required = true)]
    pub targets: Vec<String>,

    /// Top module name
    #[arg(short = 't', long = "top")]
    pub top: Option<String>,

    /// Prefix output (default: nama top) → <prefix>.mvnet dst.
    #[arg(short = 'o', long = "output")]
    pub output: Option<String>,

    /// Hanya analisis sintesizability (SYN-1..9), tanpa netlist.
    #[arg(long = "check-only")]
    pub check_only: bool,

    /// Device target: fpga-x7 (default) | generic
    #[arg(long, default_value = "fpga-x7")]
    pub device: String,

    /// Preset pipeline synthesis: generic | fpga (default) | asic | custom.
    /// Phase 2: memilih pipeline pass optimizer (mapping menyusul).
    #[arg(long, default_value = "fpga")]
    pub preset: String,

    /// Tulis netlist `.mvnet`
    #[arg(long = "emit-mvnet")]
    pub emit_mvnet: bool,

    /// Tulis dump SIR (Synthesis Intermediate Representation, node-based)
    /// ke stdout — debugging fase RTL→SIR (SYNTHESIS.md §3).
    #[arg(long = "dump-sir")]
    pub dump_sir: bool,

    /// Tulis dump SIR SETELAH pass optimizer (const fold/DCE/CSE/mux/arith)
    #[arg(long = "dump-sir-opt")]
    pub dump_sir_opt: bool,

    /// Dump netlist generik hasil mapping SIR (netlist.v + .mvnet) ke stdout
    /// — debugging fase SIR→netlist (SYNTHESIS.md §13, phase 3).
    #[arg(long = "dump-netlist")]
    pub dump_netlist: bool,

    /// Emisi netlist ke file: <prefix>.netlist.v + <prefix>.netlist.json +
    /// <prefix>.netlist.mvnet (prefix default: nama top).
    #[arg(long = "emit-netlist")]
    pub emit_netlist: bool,

    /// Tech mapping (phase 4): LUT cut + AIG dekomposisi + carry chain →
    /// <prefix>.tech.v/.json/.mvnet + report LUT/CARRY4/FF.
    #[arg(long = "tech-map")]
    pub tech_map: bool,

    /// Tulis report utilisasi ke file (tanpa ini: ke stdout)
    #[arg(long = "report-util")]
    pub report_util: Option<String>,

    /// File constraint `.mcs` (clock period, IO delay, false/multicycle path)
    /// — dipakai `--timing`. Default tanpa file: period 10ns, delay 0.
    #[arg(long = "constraint")]
    pub constraint: Option<String>,

    /// Static timing + area analysis (phase 5): hitung arrival/required/
    /// slack/WNS/TNS + critical path atas netlist (tech bila `--tech-map`, 
    /// else generic). Tulis `<prefix>.timing.rpt` + `<prefix>.area.rpt`.
    #[arg(long = "timing")]
    pub timing: bool,

    /// Tambahkan include search path
    #[arg(short = 'I', long = "incdir", num_args = 1)]
    pub incdirs: Vec<String>,

    /// Define preprocessor macro
    #[arg(short = 'D', long = "define", num_args = 1)]
    pub defines: Vec<String>,

    /// Suppress output
    #[arg(short = 'q', long)]
    pub quiet: bool,
}

/// emu — Maria emulator (EMULATOR.md). R0: MHIR extraction + memory map dump.
#[derive(clap::Args, Clone)]
pub struct EmuArgs {
    /// Target input: file .sv, direktori, atau file list (.f/.maria)
    /// (opsional bila `--boot-iso` dipakai — boot ISO tidak butuh RTL)
    pub targets: Vec<String>,

    /// Tambahkan include search path
    #[arg(short = 'I', long = "incdir", num_args = 1)]
    pub incdirs: Vec<String>,

    /// Define preprocessor macro
    #[arg(short = 'D', long = "define", num_args = 1)]
    pub defines: Vec<String>,

    /// Top module name
    #[arg(short = 't', long = "top")]
    pub top: Option<String>,

    /// File konfigurasi emulator TOML (`.meu`) — top/mode/accuracy/cpu/ram/
    /// devices/seed. File TERPISAH dari project `.maria` (MICD memakai
    /// ekstensi/direktori .maria — tidak boleh bentrok).
    #[arg(long = "config")]
    pub config: Option<String>,

    /// Cetak MHIR lengkap (device/register/clock/reset/memory + back-pointer)
    #[arg(long)]
    pub dump_mhir: bool,

    /// Cetak memory map (region ber-alamat; tanpa --addr, region TBD)
    #[arg(long)]
    pub dump_memory_map: bool,

    /// Assign alamat region: NAME=BASE:SIZE (hex, bisa diulang).
    /// Cocok dengan nama instance device, nama module device, atau nama
    /// memory — mis. `--addr u_uart=0x10000000:0x1000`.
    #[arg(long = "addr", value_name = "NAME=BASE:SIZE")]
    pub addr: Vec<String>,

    /// Muat kernel ELF ke memory map (butuh `[emu] ram` di project file .maria).
    /// Cetak entry point + isi region.
    #[arg(long = "load-elf", value_name = "PATH")]
    pub load_elf: Option<String>,

    /// Hex dump memori guest: ADDR:LEN (hex) — mis. `0x80000000:64`.
    #[arg(long = "dump-memory", value_name = "ADDR:LEN")]
    pub dump_memory: Option<String>,

    /// Jalankan CPU dari RTL (.sv/.v) — Direct RTL CPU (EMULATOR.md §7.2
    /// mode 3), BUKAN interpreter Rust. Flag diulang per file (wrapper + core,
    /// mis. `--rtl-cpu rv32_bus_wrapper.sv --rtl-cpu picorv32.v`). RTL wajib
    /// memenuhi kontrak bus picorv32-style: clk/resetn/mem_valid/mem_instr/
    /// mem_addr/mem_wdata/mem_wstrb/mem_ready/mem_rdata/trap.
    #[arg(long = "rtl-cpu", value_name = "FILE", action = clap::ArgAction::Append)]
    pub rtl_cpu: Vec<String>,

    /// Top module CPU RTL (default: rv32_bus_wrapper)
    #[arg(long = "rtl-cpu-top", value_name = "TOP")]
    pub rtl_cpu_top: Option<String>,

    /// Jalankan mesin (CPU RTL + RAM) sampai trap / --max-steps
    #[arg(long)]
    pub run: bool,

    /// Batas langkah instruksi mesin (default 10000)
    #[arg(long = "max-steps", value_name = "N")]
    pub max_steps: Option<u64>,

    /// Boot ISO x86 real-mode (R6): muat MBR ke 0x7c00, jalankan interpreter
    /// x86 real-mode dengan ISO sebagai disk (INT 13h). Jalur boot BIOS:
    /// MBR (ISOLINUX hybrid) → INT 13h → El Torito → GRUB boot.img.
    /// Butuh `ram = { base, size }` di config .meu (minimal 0x0:0x100000).
    #[arg(long = "boot-iso", value_name = "PATH")]
    pub boot_iso: Option<String>,
}

#[derive(ClapParser, Clone)]
#[command(name = "maria", about = "RTL Simulator untuk SystemVerilog")]
pub struct Cli {
    /// Subcommand (opsional)
    #[command(subcommand)]
    pub cmd: Option<MariaCmd>,

    /// Input SystemVerilog file(s) — last is top module
    pub files: Vec<String>,

    /// Top module name (default: first module)
    #[arg(short = 't', long = "top")]
    pub top: Option<String>,

    /// Maximum simulation time (default: unlimited — run until $finish/$fatal)
    #[arg(short = 'T', long = "max-time", alias = "time", value_name = "NS")]
    pub max_time: Option<u64>,

    /// Force simulation + VCD even when elaboration errors/skipped modules exist
    /// (default: Maria refuses to simulate until the design elaborates cleanly)
    #[arg(long = "force-sim")]
    pub force_sim: bool,

    /// VCD/FST output file (default: <module>.vcd)
    #[arg(short = 'o', long = "output")]
    pub output: Option<String>,

    /// Enable waveform streaming to disk (flush after each time step)
    /// Allows external tools to read waveform during simulation.
    #[arg(long = "waveform-stream")]
    pub waveform_stream: bool,

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

    /// Load configuration from TOML file (configs/*.toml).
    /// Default: configs/compiler.toml bila ada.
    #[arg(long = "config")]
    pub config: Option<String>,

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

    /// Tampilkan Global Diagnostic Engine report (semua diagnostic lintas
    /// komponen, coverage posisi + registry error code) setelah run.
    #[arg(long = "gdiag")]
    pub gdiag: bool,

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

    /// VHPI (IEEE 1076-2008) shared library to load — ABI-compatible adapter
    /// (can be specified multiple times). Loading memakai feature "dpi"
    /// (libloading); tanpa feature → error jelas saat --vhpi dipakai.
    #[arg(long = "vhpi", num_args = 1)]
    pub vhpi_libs: Vec<String>,

    /// PLI (IEEE 1364) shared library to load — ABI-compatible adapter
    /// (can be specified multiple times).
    #[arg(long = "pli", num_args = 1)]
    pub pli_libs: Vec<String>,

    /// Save simulation checkpoint to file (after sim, or at --break-cycle)
    #[arg(long = "save")]
    pub save: Option<String>,

    /// Restore simulation checkpoint from file (overrides initial state)
    #[arg(long = "restore")]
    pub restore: Option<String>,

    /// SIM-18: auto-checkpoint (crash recovery) — simpan state tiap interval
    /// cycle ke file; run yang crash bisa di-resume dari titik terakhir.
    #[arg(long = "auto-checkpoint")]
    pub auto_checkpoint: Option<String>,

    /// Interval (cycle) auto-checkpoint (default 1000)
    #[arg(long = "checkpoint-interval", default_value_t = 1000)]
    pub checkpoint_interval: u64,

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

    /// (Internal) Mode elaborasi dari config TOML (`[elaborate] mode`).
    /// Tidak di-parse dari CLI — diisi oleh `apply_config_to_cli` di main.rs.
    #[arg(skip)]
    pub config_elab_mode: Option<String>,
}
