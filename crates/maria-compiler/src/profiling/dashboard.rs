//! Performance monitoring dashboard untuk simulasi runtime (SIM-25).
//!
//! Mengumpulkan metrik simulasi yang berjalan (delta cycles, events processed,
//! time steps, NBA commits, trigger sensitive processes) dan menghasilkan
//! laporan throughput/kecepatan simulasi — dipakai CLI `--perf-dashboard`.

use std::time::Instant;

/// Metrik performa simulasi runtime yang dikumpulkan oleh engine.
///
/// Field-field ini di-increment di dalam event loop (`SimulationEngine::run`).
/// Kecepatan/throughput dihitung saat laporan dibuat terhadap `start`.
#[derive(Debug, Default)]
pub struct SimPerfCounters {
    /// Jumlah delta cycle yang dieksekusi (total seluruh time step).
    pub delta_cycles: u64,
    /// Jumlah time step yang dilalui (simulasi time units).
    pub time_steps: u64,
    /// Jumlah event yang di-drain & diproses dari antrian region.
    pub events_processed: u64,
    /// Jumlah commit non-blocking assignment (NBA).
    pub nba_commits: u64,
    /// Jumlah trigger sensitive processes (always @(...) / initial sensitive).
    pub sensitive_triggers: u64,
    /// Jumlah process yang benar-benar dievaluasi (EvalProcess diproses).
    pub processes_evaluated: u64,
}

impl SimPerfCounters {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Dashboard performa: membungkus counter + pengukur waktu wall-clock.
#[derive(Debug)]
pub struct PerfDashboard {
    pub counters: SimPerfCounters,
    start: Instant,
}

impl Default for PerfDashboard {
    fn default() -> Self {
        Self::new()
    }
}

impl PerfDashboard {
    pub fn new() -> Self {
        PerfDashboard {
            counters: SimPerfCounters::new(),
            start: Instant::now(),
        }
    }

    /// Wall-clock elapsed sejak dashboard dibuat (engine dijalankan).
    pub fn elapsed(&self) -> std::time::Duration {
        self.start.elapsed()
    }

    /// Rata-rata event per delta cycle (0 jika belum ada delta).
    pub fn events_per_delta(&self) -> f64 {
        if self.counters.delta_cycles == 0 {
            0.0
        } else {
            self.counters.events_processed as f64 / self.counters.delta_cycles as f64
        }
    }

    /// Throughput event per detik wall-clock.
    pub fn events_per_sec(&self) -> f64 {
        let secs = self.elapsed().as_secs_f64();
        if secs <= 0.0 {
            0.0
        } else {
            self.counters.events_processed as f64 / secs
        }
    }

    /// Delta cycles per detik wall-clock.
    pub fn deltas_per_sec(&self) -> f64 {
        let secs = self.elapsed().as_secs_f64();
        if secs <= 0.0 {
            0.0
        } else {
            self.counters.delta_cycles as f64 / secs
        }
    }
}

impl std::fmt::Display for PerfDashboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "═══ Simulation Performance Dashboard ═══")?;
        writeln!(
            f,
            "Wall-clock:        {:>8.2} s",
            self.elapsed().as_secs_f64()
        )?;
        writeln!(
            f,
            "Simulated time:    {:>8} time units",
            self.counters.time_steps
        )?;
        writeln!(
            f,
            "Delta cycles:      {:>8} (avg {:.1}/time step)",
            self.counters.delta_cycles,
            if self.counters.time_steps == 0 {
                0.0
            } else {
                self.counters.delta_cycles as f64 / self.counters.time_steps as f64
            }
        )?;
        writeln!(
            f,
            "Events processed:  {:>8} (avg {:.1}/delta)",
            self.counters.events_processed,
            self.events_per_delta()
        )?;
        writeln!(f, "NBA commits:       {:>8}", self.counters.nba_commits)?;
        writeln!(
            f,
            "Sensitive triggers:{:>8}",
            self.counters.sensitive_triggers
        )?;
        writeln!(
            f,
            "Processes evaluated:{:>7}",
            self.counters.processes_evaluated
        )?;
        writeln!(f)?;
        writeln!(
            f,
            "Throughput:        {:>8.0} events/s | {:.0} deltas/s",
            self.events_per_sec(),
            self.deltas_per_sec()
        )?;
        Ok(())
    }
}

// ─── ENT-07: Comprehensive Metrics System ───

/// Metrik kompilasi yang dikumpulkan selama pipeline compile.
#[derive(Debug, Default, Clone)]
pub struct CompileMetrics {
    pub files_processed: usize,
    pub modules_found: usize,
    pub parse_time_ms: u64,
    pub elaborate_time_ms: u64,
    pub optimize_time_ms: u64,
    pub total_time_ms: u64,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub errors: usize,
    pub warnings: usize,
}

impl CompileMetrics {
    pub fn new() -> Self { Self::default() }

    /// Cache hit rate (0.0..=1.0).
    pub fn cache_hit_rate(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 { 0.0 } else { self.cache_hits as f64 / total as f64 }
    }
}

impl std::fmt::Display for CompileMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "═══ Compile Metrics ═══")?;
        writeln!(f, "Files:     {:>8}", self.files_processed)?;
        writeln!(f, "Modules:   {:>8}", self.modules_found)?;
        writeln!(f, "Parse:     {:>8} ms", self.parse_time_ms)?;
        writeln!(f, "Elab:      {:>8} ms", self.elaborate_time_ms)?;
        writeln!(f, "Optimize:  {:>8} ms", self.optimize_time_ms)?;
        writeln!(f, "Total:     {:>8} ms", self.total_time_ms)?;
        writeln!(f, "Cache:     {:>5.1}% hit ({}/{})", self.cache_hit_rate() * 100.0, self.cache_hits, self.cache_hits + self.cache_misses)?;
        writeln!(f, "Errors:    {:>8}", self.errors)?;
        writeln!(f, "Warnings:  {:>8}", self.warnings)?;
        Ok(())
    }
}

/// Metrik coverage yang dikumpulkan selama simulasi.
#[derive(Debug, Default, Clone)]
pub struct CoverageMetrics {
    pub line_total: usize,
    pub line_hit: usize,
    pub toggle_total: usize,
    pub toggle_hit: usize,
    pub branch_total: usize,
    pub branch_hit: usize,
    pub fsm_total: usize,
    pub fsm_hit: usize,
    pub covergroup_total: usize,
    pub covergroup_hit: usize,
}

impl CoverageMetrics {
    pub fn new() -> Self { Self::default() }

    pub fn line_percent(&self) -> f64 {
        if self.line_total == 0 { 0.0 } else { self.line_hit as f64 / self.line_total as f64 * 100.0 }
    }
    pub fn toggle_percent(&self) -> f64 {
        if self.toggle_total == 0 { 0.0 } else { self.toggle_hit as f64 / self.toggle_total as f64 * 100.0 }
    }
    pub fn branch_percent(&self) -> f64 {
        if self.branch_total == 0 { 0.0 } else { self.branch_hit as f64 / self.branch_total as f64 * 100.0 }
    }
    pub fn overall_percent(&self) -> f64 {
        let total = self.line_total + self.toggle_total + self.branch_total + self.fsm_total;
        let hit = self.line_hit + self.toggle_hit + self.branch_hit + self.fsm_hit;
        if total == 0 { 0.0 } else { hit as f64 / total as f64 * 100.0 }
    }
}

impl std::fmt::Display for CoverageMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "═══ Coverage Metrics ═══")?;
        writeln!(f, "Line:      {:>5.1}% ({}/{})", self.line_percent(), self.line_hit, self.line_total)?;
        writeln!(f, "Toggle:    {:>5.1}% ({}/{})", self.toggle_percent(), self.toggle_hit, self.toggle_total)?;
        writeln!(f, "Branch:    {:>5.1}% ({}/{})", self.branch_percent(), self.branch_hit, self.branch_total)?;
        writeln!(f, "Overall:   {:>5.1}%", self.overall_percent())?;
        Ok(())
    }
}

/// Metrics gabungan untuk reporting lengkap.
#[derive(Debug, Default)]
pub struct MetricsReport {
    pub compile: CompileMetrics,
    pub simulation: PerfDashboard,
    pub coverage: CoverageMetrics,
}

impl MetricsReport {
    pub fn new() -> Self { Self::default() }

    /// Export sebagai JSON string.
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "compile": {
                "files": self.compile.files_processed,
                "modules": self.compile.modules_found,
                "parse_ms": self.compile.parse_time_ms,
                "elab_ms": self.compile.elaborate_time_ms,
                "total_ms": self.compile.total_time_ms,
                "cache_hit_rate": self.compile.cache_hit_rate(),
            },
            "simulation": {
                "delta_cycles": self.simulation.counters.delta_cycles,
                "events_per_sec": self.simulation.events_per_sec(),
                "deltas_per_sec": self.simulation.deltas_per_sec(),
            },
            "coverage": {
                "line_percent": self.coverage.line_percent(),
                "toggle_percent": self.coverage.toggle_percent(),
                "branch_percent": self.coverage.branch_percent(),
                "overall_percent": self.coverage.overall_percent(),
            },
        }).to_string()
    }
}

impl std::fmt::Display for MetricsReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{}", self.compile)?;
        writeln!(f, "{}", self.simulation)?;
        writeln!(f, "{}", self.coverage)
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dashboard_defaults() {
        let d = PerfDashboard::new();
        assert_eq!(d.counters.delta_cycles, 0);
        assert_eq!(d.counters.events_processed, 0);
        assert_eq!(d.events_per_delta(), 0.0);
    }

    #[test]
    fn test_dashboard_metrics() {
        let mut d = PerfDashboard::new();
        d.counters.delta_cycles = 10;
        d.counters.events_processed = 50;
        d.counters.time_steps = 5;
        assert!((d.events_per_delta() - 5.0).abs() < f64::EPSILON);
        assert_eq!(d.counters.nba_commits, 0);
    }

    #[test]
    fn test_dashboard_display() {
        let d = PerfDashboard::new();
        let s = format!("{}", d);
        assert!(s.contains("Simulation Performance Dashboard"));
        assert!(s.contains("Delta cycles"));
        assert!(s.contains("Throughput"));
    }

    #[test]
    fn test_compile_metrics() {
        let mut m = CompileMetrics::new();
        m.files_processed = 10;
        m.modules_found = 5;
        m.cache_hits = 3;
        m.cache_misses = 7;
        assert!((m.cache_hit_rate() - 0.3).abs() < f64::EPSILON);
        let s = format!("{}", m);
        assert!(s.contains("Compile Metrics"));
    }

    #[test]
    fn test_coverage_metrics() {
        let mut c = CoverageMetrics::new();
        c.line_total = 100;
        c.line_hit = 80;
        c.branch_total = 50;
        c.branch_hit = 40;
        assert!((c.line_percent() - 80.0).abs() < f64::EPSILON);
        assert!((c.branch_percent() - 80.0).abs() < f64::EPSILON);
        assert!((c.overall_percent() - 80.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_metrics_report_json() {
        let r = MetricsReport::new();
        let json = r.to_json();
        assert!(json.contains("compile"));
        assert!(json.contains("simulation"));
        assert!(json.contains("coverage"));
    }
}
