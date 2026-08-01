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
        writeln!(f, "Wall-clock:        {:>8.2} s", self.elapsed().as_secs_f64())?;
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
        writeln!(
            f,
            "NBA commits:       {:>8}",
            self.counters.nba_commits
        )?;
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
}
