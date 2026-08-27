//! Maria Config — loader file konfigurasi TOML (`configs/*.toml`).
//!
//! Pengaturan compiler utama (jobs, incremental, optimisasi, batas parser),
//! simulasi (max_time, watchdog, force_sim), waveform (vcd/fst/stream),
//! lint, coverage, debug, dan benchmark. Format mengikuti konvensi project
//! (banner `# ====`, section `[compiler]`/`[simulation]`, komentar).
//!
//! Dipakai via flag CLI `--config <path>`; tanpa flag, Maria memuat
//! `configs/compiler.toml` bila ada (default), lalu CLI override menang.

use serde::Deserialize;
use std::fs;
use std::path::Path;

/// Seluruh konfigurasi Maria. Field opsional — nilai yang tidak ada di file
/// dibiarkan `None` (fallback ke default CLI/kode).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MariaConfig {
    #[serde(default)]
    pub compiler: CompilerConfig,
    #[serde(default)]
    pub parse: ParseConfig,
    #[serde(default)]
    pub elaborate: ElaborateConfig,
    #[serde(default)]
    pub simulation: SimulationConfig,
    #[serde(default)]
    pub waveform: WaveformConfig,
    #[serde(default)]
    pub lint: LintConfig,
    #[serde(default)]
    pub coverage: CoverageConfig,
    #[serde(default)]
    pub debug: DebugConfig,
    #[serde(default)]
    pub benchmark: BenchmarkConfig,
    #[serde(default)]
    pub verify: VerifyConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CompilerConfig {
    /// Standard bahasa (2012 / 2017 / 2023). Default: 2012.
    pub edition: Option<String>,
    /// Target: native | jit | mir.
    pub target: Option<String>,
    /// Jumlah thread parallel (0 = auto / semua core).
    pub jobs: Option<usize>,
    /// Incremental compilation via MICD.
    pub incremental: Option<bool>,
    /// Level optimisasi IR: 0..=3.
    pub opt_level: Option<u8>,
    /// LTO: off | thin | fat.
    pub lto: Option<String>,
    /// Batas maksimum langkah parser (anti infinite loop).
    pub max_parse_steps: Option<usize>,
    /// Aktifkan cache MICD.
    pub cache: Option<bool>,
    /// Berhenti di error pertama.
    pub fail_fast: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ParseConfig {
    pub max_recursion_depth: Option<usize>,
    pub lenient: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ElaborateConfig {
    /// StrictSimulation | AnalysisRecovery
    pub mode: Option<String>,
    pub expand_all_generates: Option<bool>,
    pub unroll_const_loops: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SimulationConfig {
    /// Batas waktu simulasi (ns). None = unlimited (jalan sampai $finish).
    pub max_time: Option<u64>,
    pub force_sim: Option<bool>,
    /// Watchdog: peringatan bila tidak ada event selama N detik.
    pub watchdog_idle_seconds: Option<u64>,
    pub progress_every_ticks: Option<u64>,
    pub max_delta_per_step: Option<u64>,
    // ENT-20: Resource management
    /// Batas memori maksimum (MB). None = unlimited.
    pub memory_limit_mb: Option<u64>,
    /// Batas jumlah thread CPU. None = gunakan compiler.jobs.
    pub cpu_threads: Option<usize>,
    /// Timeout simulasi (detik). None = unlimited.
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WaveformConfig {
    pub vcd: Option<bool>,
    pub fst: Option<bool>,
    pub stream: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LintConfig {
    pub check_unused: Option<bool>,
    pub check_width: Option<bool>,
    pub check_latch: Option<bool>,
    pub check_combo_loop: Option<bool>,
    pub check_fsm: Option<bool>,
    pub check_case_priority: Option<bool>,
    pub check_partselect_range: Option<bool>,
    pub check_undefined_signal: Option<bool>,
    pub warnings_are_errors: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CoverageConfig {
    pub enable: Option<bool>,
    pub branch_threshold: Option<f64>,
    pub line_threshold: Option<f64>,
    pub json: Option<bool>,
    pub html: Option<bool>,
    pub ucis: Option<bool>,
    pub output_prefix: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DebugConfig {
    pub deep: Option<bool>,
    pub snapshot_interval: Option<u64>,
    pub break_cycle: Option<u64>,
    pub watch: Option<Vec<String>>,
    pub print_tree: Option<bool>,
    pub timeline: Option<Vec<String>>,
    pub timeline_len: Option<usize>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct BenchmarkConfig {
    pub repeats: Option<usize>,
    pub per_phase: Option<bool>,
    pub report_throughput: Option<bool>,
    pub report_memory: Option<bool>,
    pub fixed_seed: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct VerifyConfig {
    pub check_width: Option<bool>,
    pub check_port: Option<bool>,
}

impl MariaConfig {
    /// Muat config dari path file TOML. Error = file tidak terbaca ATAU
    /// TOML tidak valid (agar pengguna sadar config-nya salah sejak awal).
    pub fn load(path: &str) -> Result<MariaConfig, String> {
        let content = fs::read_to_string(path).map_err(|e| format!("config '{}': {}", path, e))?;
        toml::from_str(&content).map_err(|e| format!("config '{}' invalid TOML: {}", path, e))
    }

    /// Muat dari `--config` bila di-set; selain itu coba default
    /// `configs/compiler.toml` (jika ada). Tidak error bila default tidak ada.
    pub fn load_auto(explicit: Option<&str>) -> Result<MariaConfig, String> {
        if let Some(p) = explicit {
            return Self::load(p);
        }
        if Path::new("configs/compiler.toml").exists() {
            return Self::load("configs/compiler.toml");
        }
        Ok(MariaConfig::default())
    }

    // ENT-20: Resource management helpers

    /// Jumlah thread yang diizinkan (dari compiler.jobs atau simulation.cpu_threads).
    pub fn max_threads(&self) -> usize {
        self.simulation
            .cpu_threads
            .or(self.compiler.jobs)
            .unwrap_or_else(num_cpus::get)
    }

    /// Batas memori dalam MB (None = unlimited).
    pub fn memory_limit_mb(&self) -> Option<u64> {
        self.simulation.memory_limit_mb
    }

    /// Timeout simulasi dalam detik (None = unlimited).
    pub fn sim_timeout(&self) -> Option<u64> {
        self.simulation.timeout_seconds
    }

    /// Apakah incremental compilation aktif.
    pub fn incremental(&self) -> bool {
        self.compiler.incremental.unwrap_or(true)
    }

    /// Level optimisasi (0..=3).
    pub fn opt_level(&self) -> u8 {
        self.compiler.opt_level.unwrap_or(1)
    }
}
