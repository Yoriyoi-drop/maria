//! Signal Statistics — collect toggle counts, transitions per signal.
//!
//! Mencatat setiap perubahan signal selama simulasi dan menghasilkan
//! laporan statistik: toggle count, toggle rate, transitions, активности.
//!
//! Gunakan `--signal-stats <path>` untuk mengaktifkan.

use maria_ir::{IrDesign, LogicVal};

/// Per-signal statistics.
#[derive(Debug, Clone, Default)]
pub struct SignalStat {
    /// Signal name
    pub name: String,
    /// Total number of toggles (0→1, 1→0, etc.)
    pub toggle_count: u64,
    /// Last known value (for edge detection)
    pub last_value: Option<u64>,
    /// Time of first change
    pub first_change_time: u64,
    /// Time of last change
    pub last_change_time: u64,
    /// Rise count (0→1 or X→1)
    pub rise_count: u64,
    /// Fall count (1→0 or 1→X)
    pub fall_count: u64,
    /// Percentage of time signal was high (estimated)
    pub high_time: u64,
    /// Percentage of time signal was low (estimated)
    pub low_time: u64,
    /// Unknown/X states count
    pub x_count: u64,
    /// High-impedance count
    pub z_count: u64,
}

/// Signal statistics collector.
#[derive(Debug)]
pub struct SignalStats {
    /// Per-signal statistics keyed by signal index
    pub stats: Vec<SignalStat>,
    /// Total simulation time observed
    total_time: u64,
}

impl SignalStats {
    /// Create a new statistics collector from design signals.
    pub fn new(design: &IrDesign) -> Self {
        let stats: Vec<SignalStat> = design
            .top
            .signals
            .iter()
            .map(|sig| {
                let name = sig.name.to_string();
                SignalStat {
                    name,
                    ..Default::default()
                }
            })
            .collect();

        SignalStats {
            stats,
            total_time: 0,
        }
    }

    /// Record signal states at a given time step.
    /// Call this AFTER commit_changes() so we see the committed values.
    pub fn record(&mut self, time: u64, signals: &[maria_ir::LogicVec]) {
        self.total_time = time;

        for (i, sig_val) in signals.iter().enumerate() {
            if i >= self.stats.len() {
                break;
            }

            let u64_val = sig_val.to_u64();
            let stat = &mut self.stats[i];

            // Detect X or Z
            let has_x = sig_val.bits.iter().any(|b| matches!(b, LogicVal::X));
            let has_z = sig_val.bits.iter().any(|b| matches!(b, LogicVal::Z));

            if has_x {
                stat.x_count += 1;
            }
            if has_z {
                stat.z_count += 1;
            }

            // Detect toggle (value change)
            if let Some(prev) = stat.last_value {
                if prev != u64_val {
                    stat.toggle_count += 1;

                    if stat.first_change_time == 0 {
                        stat.first_change_time = time;
                    }
                    stat.last_change_time = time;

                    // Detect rise (was low, now high — for single-bit)
                    if sig_val.width == 1 {
                        if u64_val == 1 && prev == 0 {
                            stat.rise_count += 1;
                        } else if u64_val == 0 && prev == 1 {
                            stat.fall_count += 1;
                        }
                    }

                    // Track high/low time
                    if prev == 1 || (!has_x && !has_z && u64_val > 0) {
                        stat.high_time += 1;
                    } else if prev == 0 || u64_val == 0 {
                        stat.low_time += 1;
                    }
                }
            }

            stat.last_value = Some(u64_val);
        }
    }

    /// Generate a human-readable report.
    pub fn report(&self) -> String {
        let mut report = String::new();
        report.push_str("═══════════════════════════════════════════════\n");
        report.push_str("  Signal Statistics Report\n");
        report.push_str(&format!("  Total time: {} steps\n", self.total_time));
        report.push_str("═══════════════════════════════════════════════\n\n");

        report.push_str(&format!(
            "{:<30} {:>8} {:>10} {:>8} {:>8} {:>8} {:>8}\n",
            "Signal", "Toggles", "ToggleRate", "Rises", "Falls", "X_couunt", "Z_count"
        ));
        report.push_str(&"-".repeat(86));
        report.push('\n');

        // Sort by toggle count descending
        let mut sorted: Vec<&SignalStat> = self.stats.iter().collect();
        sorted.sort_by_key(|a| std::cmp::Reverse(a.toggle_count));

        let total_toggles: u64 = sorted.iter().map(|s| s.toggle_count).sum();

        for stat in &sorted {
            if stat.toggle_count == 0 && stat.x_count == 0 && stat.z_count == 0 {
                continue; // Skip inactive signals
            }

            let rate = if self.total_time > 0 {
                stat.toggle_count as f64 / self.total_time as f64
            } else {
                0.0
            };

            report.push_str(&format!(
                "{:<30} {:>8} {:>8.2}/st {:>8} {:>8} {:>8} {:>8}\n",
                truncate_str(&stat.name, 28),
                stat.toggle_count,
                rate,
                stat.rise_count,
                stat.fall_count,
                stat.x_count,
                stat.z_count,
            ));
        }

        report.push('\n');
        report.push_str(&format!("Total toggles across all signals: {}\n", total_toggles));
        report.push_str(&format!("Total active signals: {}\n", sorted.iter().filter(|s| s.toggle_count > 0).count()));
        report.push('\n');

        report
    }

    /// Write report to a file.
    pub fn write_to_file(&self, path: &str) -> Result<(), String> {
        let report = self.report();
        std::fs::write(path, &report)
            .map_err(|e| format!("cannot write signal stats '{}': {}", path, e))?;
        Ok(())
    }
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("...{}", &s[s.len().saturating_sub(max_len - 3)..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maria_ir::{LogicVec, IrDesign, SignalInfo};

    fn make_design() -> IrDesign {
        let mut design = IrDesign::default();
        design.top.signals = vec![
            SignalInfo {
                name: maria_core::intern::Symbol::intern("clk"),
                width: 1,
                ..Default::default()
            },
            SignalInfo {
                name: maria_core::intern::Symbol::intern("data"),
                width: 8,
                ..Default::default()
            },
        ];
        design
    }

    #[test]
    fn test_signal_stats_new() {
        let design = make_design();
        let stats = SignalStats::new(&design);
        assert_eq!(stats.stats.len(), 2);
        assert_eq!(stats.stats[0].name, "clk");
        assert_eq!(stats.stats[1].name, "data");
    }

    #[test]
    fn test_signal_stats_record_toggle() {
        let design = make_design();
        let mut stats = SignalStats::new(&design);

        // Time 0: clk=0, data=0
        let t0 = vec![LogicVec::from_u64(0, 1), LogicVec::from_u64(0, 8)];
        stats.record(0, &t0);
        assert_eq!(stats.stats[0].toggle_count, 0); // No toggle yet (first sample)

        // Time 1: clk=1, data=42
        let t1 = vec![LogicVec::from_u64(1, 1), LogicVec::from_u64(42, 8)];
        stats.record(1, &t1);
        assert_eq!(stats.stats[0].toggle_count, 1); // clk 0→1
        assert_eq!(stats.stats[1].toggle_count, 1); // data 0→42

        // Time 2: clk=0, data=42 (no change)
        let t2 = vec![LogicVec::from_u64(0, 1), LogicVec::from_u64(42, 8)];
        stats.record(2, &t2);
        assert_eq!(stats.stats[0].toggle_count, 2); // clk 1→0
        assert_eq!(stats.stats[1].toggle_count, 1); // data unchanged
    }

    #[test]
    fn test_signal_stats_report() {
        let design = make_design();
        let mut stats = SignalStats::new(&design);

        stats.record(0, &[LogicVec::from_u64(0, 1), LogicVec::from_u64(0, 8)]);
        stats.record(1, &[LogicVec::from_u64(1, 1), LogicVec::from_u64(255, 8)]);

        let report = stats.report();
        assert!(report.contains("clk"), "report should contain clk");
        assert!(report.contains("data"), "report should contain data");
        assert!(report.contains("Toggles"), "report should have header");
        assert!(report.contains("Total toggles"), "report should have summary");
    }

    #[test]
    fn test_signal_stats_write_file() {
        use std::io::Write;
        let dir = std::env::temp_dir().join("maria_stats_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("stats.txt");

        let design = make_design();
        let mut stats = SignalStats::new(&design);
        stats.record(0, &[LogicVec::from_u64(0, 1), LogicVec::from_u64(0, 8)]);
        stats.record(1, &[LogicVec::from_u64(1, 1), LogicVec::from_u64(42, 8)]);

        stats.write_to_file(path.to_str().unwrap()).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("clk"));
        assert!(content.contains("Total toggles"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("short", 10), "short");
        let long = "a".repeat(30);
        assert_eq!(truncate_str(&long, 10).len(), 10);
        assert!(truncate_str(&long, 10).starts_with("..."));
    }
}
