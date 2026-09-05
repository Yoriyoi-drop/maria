//! ENT-32: STA Tool Integration — interface for PrimeTime/Tempus.
//!
//! Generates reports compatible with STA tools and parses
//! timing reports for feedback.

use serde::{Deserialize, Serialize};

/// Timing path report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingPath {
    pub start_point: String,
    pub end_point: String,
    pub delay: f64,
    pub slack: f64,
    pub required: f64,
    pub arrival: f64,
    pub path_type: String, // "setup" or "hold"
    pub is_critical: bool,
}

/// Clock definition for STA.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClockDef {
    pub name: String,
    pub period: f64,
    pub waveform: (f64, f64),
    pub source_port: String,
}

/// STA report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaReport {
    pub paths: Vec<TimingPath>,
    pub clocks: Vec<ClockDef>,
    pub summary: StaSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaSummary {
    pub total_paths: usize,
    pub violating_paths: usize,
    pub worst_slack: f64,
    pub best_slack: f64,
    pub total_negative_slack: f64,
}

impl StaReport {
    /// Generate PrimeTime-compatible report.
    pub fn to_primetime_report(&self) -> String {
        let mut out = String::new();
        out.push_str("Timing Report — PrimeTime Compatible\n");
        out.push_str("=====================================\n\n");

        out.push_str("Clock Information:\n");
        for clk in &self.clocks {
            out.push_str(&format!(
                "  Clock: {} Period: {}ns Source: {}\n",
                clk.name, clk.period, clk.source_port,
            ));
        }
        out.push_str("\n");

        out.push_str(&format!(
            "Summary: {} paths, {} violations, worst slack: {:.2}ns\n\n",
            self.summary.total_paths, self.summary.violating_paths, self.summary.worst_slack,
        ));

        out.push_str("Timing Paths:\n");
        for path in &self.paths {
            let marker = if path.is_critical {
                " *** CRITICAL ***"
            } else {
                ""
            };
            out.push_str(&format!(
                "  {} -> {} | Delay: {:.2} Slack: {:.2} ({}){}\n",
                path.start_point, path.end_point, path.delay, path.slack, path.path_type, marker,
            ));
        }

        out
    }

    /// Generate Tempus-compatible report.
    pub fn to_tempus_report(&self) -> String {
        let mut out = String::new();
        out.push_str("=== Cadence Tempus Timing Report ===\n\n");
        out.push_str(&format!(
            "WNS: {:.3}  TNS: {:.3}\n",
            self.summary.worst_slack, self.summary.total_negative_slack,
        ));
        out.push_str(&format!(
            "Failing Paths: {}/{}\n\n",
            self.summary.violating_paths, self.summary.total_paths,
        ));

        for (i, path) in self.paths.iter().enumerate() {
            out.push_str(&format!(
                "Path {}:\n  Start: {}\n  End: {}\n  Delay: {:.3}\n  Slack: {:.3}\n\n",
                i + 1,
                path.start_point,
                path.end_point,
                path.delay,
                path.slack,
            ));
        }

        out
    }

    /// Generate SDC for the clocks.
    pub fn to_sdc(&self) -> String {
        let mut out = String::new();
        for clk in &self.clocks {
            out.push_str(&format!(
                "create_clock -name {} -period {} -waveform {{{} {}}} [get_ports {}]\n",
                clk.name, clk.period, clk.waveform.0, clk.waveform.1, clk.source_port,
            ));
        }
        out
    }

    /// Compute summary from paths.
    pub fn compute_summary(paths: &[TimingPath]) -> StaSummary {
        let total = paths.len();
        let violating = paths.iter().filter(|p| p.slack < 0.0).count();
        let worst = paths.iter().map(|p| p.slack).fold(f64::INFINITY, f64::min);
        let best = paths
            .iter()
            .map(|p| p.slack)
            .fold(f64::NEG_INFINITY, f64::max);
        let tns: f64 = paths
            .iter()
            .filter(|p| p.slack < 0.0)
            .map(|p| p.slack)
            .sum();

        StaSummary {
            total_paths: total,
            violating_paths: violating,
            worst_slack: if worst == f64::INFINITY { 0.0 } else { worst },
            best_slack: if best == f64::NEG_INFINITY { 0.0 } else { best },
            total_negative_slack: tns,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_report() -> StaReport {
        let paths = vec![
            TimingPath {
                start_point: "u_reg/CK".into(),
                end_point: "u_reg/D".into(),
                delay: 2.5,
                slack: -0.3,
                required: 5.0,
                arrival: 5.3,
                path_type: "setup".into(),
                is_critical: true,
            },
            TimingPath {
                start_point: "u_comb/A".into(),
                end_point: "u_comb/Y".into(),
                delay: 1.2,
                slack: 1.5,
                required: 5.0,
                arrival: 3.5,
                path_type: "setup".into(),
                is_critical: false,
            },
        ];

        let summary = StaReport::compute_summary(&paths);
        StaReport {
            paths,
            clocks: vec![ClockDef {
                name: "clk".into(),
                period: 10.0,
                waveform: (0.0, 5.0),
                source_port: "clk".into(),
            }],
            summary,
        }
    }

    #[test]
    fn test_primetime_report() {
        let report = make_report();
        let text = report.to_primetime_report();
        assert!(text.contains("PrimeTime"));
        assert!(text.contains("clk"));
    }

    #[test]
    fn test_tempus_report() {
        let report = make_report();
        let text = report.to_tempus_report();
        assert!(text.contains("Tempus"));
        assert!(text.contains("WNS"));
    }

    #[test]
    fn test_sdc_export() {
        let report = make_report();
        let sdc = report.to_sdc();
        assert!(sdc.contains("create_clock"));
        assert!(sdc.contains("clk"));
    }

    #[test]
    fn test_compute_summary() {
        let report = make_report();
        assert_eq!(report.summary.total_paths, 2);
        assert_eq!(report.summary.violating_paths, 1);
        assert!(report.summary.worst_slack < 0.0);
    }
}
