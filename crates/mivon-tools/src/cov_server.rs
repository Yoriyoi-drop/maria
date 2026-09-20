//! ENT-27: Coverage Database Server — HTTP API for coverage data access.
//!
//! API for team-based coverage viewing and merging.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageEntry {
    pub id: String,
    pub name: String,
    pub coverage_type: String, // "line", "toggle", "assertion", "functional"
    pub total_bins: u64,
    pub covered_bins: u64,
    pub percentage: f64,
    pub file_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageReport {
    pub run_id: String,
    pub entries: Vec<CoverageEntry>,
    pub overall_percentage: f64,
    pub timestamp: String,
}

pub struct CoverageServer {
    reports: Mutex<HashMap<String, CoverageReport>>,
}

impl CoverageServer {
    pub fn new() -> Self {
        CoverageServer {
            reports: Mutex::new(HashMap::new()),
        }
    }

    pub fn add_report(&self, report: CoverageReport) {
        self.reports
            .lock()
            .unwrap()
            .insert(report.run_id.clone(), report);
    }

    pub fn list_reports(&self) -> Vec<CoverageReport> {
        self.reports.lock().unwrap().values().cloned().collect()
    }

    pub fn get_report(&self, run_id: &str) -> Option<CoverageReport> {
        self.reports.lock().unwrap().get(run_id).cloned()
    }

    pub fn get_entry(&self, run_id: &str, entry_id: &str) -> Option<CoverageEntry> {
        self.reports
            .lock()
            .unwrap()
            .get(run_id)?
            .entries
            .iter()
            .find(|e| e.id == entry_id)
            .cloned()
    }

    pub fn compare(&self, run_a: &str, run_b: &str) -> Option<Vec<(String, f64, f64)>> {
        let reports = self.reports.lock().unwrap();
        let a = reports.get(run_a)?;
        let b = reports.get(run_b)?;

        let mut diffs = Vec::new();
        for ea in &a.entries {
            if let Some(eb) = b.entries.iter().find(|e| e.id == ea.id) {
                if (ea.percentage - eb.percentage).abs() > 0.01 {
                    diffs.push((ea.name.clone(), ea.percentage, eb.percentage));
                }
            }
        }
        Some(diffs)
    }

    pub fn summary(&self) -> String {
        let reports = self.reports.lock().unwrap();
        let total_entries: usize = reports.values().map(|r| r.entries.len()).sum();
        format!(
            "CoverageServer: {} reports, {} total entries",
            reports.len(),
            total_entries,
        )
    }
}

impl Default for CoverageServer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_report(id: &str) -> CoverageReport {
        CoverageReport {
            run_id: id.to_string(),
            entries: vec![
                CoverageEntry {
                    id: "line1".into(),
                    name: "counter.sv".into(),
                    coverage_type: "line".into(),
                    total_bins: 100,
                    covered_bins: 80,
                    percentage: 80.0,
                    file_source: None,
                },
                CoverageEntry {
                    id: "tog1".into(),
                    name: "clk_toggle".into(),
                    coverage_type: "toggle".into(),
                    total_bins: 50,
                    covered_bins: 40,
                    percentage: 80.0,
                    file_source: None,
                },
            ],
            overall_percentage: 80.0,
            timestamp: "2026-08-27".into(),
        }
    }

    #[test]
    fn test_add_and_list() {
        let server = CoverageServer::new();
        server.add_report(make_report("run1"));
        server.add_report(make_report("run2"));
        assert_eq!(server.list_reports().len(), 2);
    }

    #[test]
    fn test_get_entry() {
        let server = CoverageServer::new();
        server.add_report(make_report("run1"));
        let entry = server.get_entry("run1", "line1").unwrap();
        assert_eq!(entry.percentage, 80.0);
    }

    #[test]
    fn test_compare() {
        let server = CoverageServer::new();
        let mut r1 = make_report("run1");
        let mut r2 = make_report("run2");
        r2.entries[0].percentage = 90.0; // improved
        server.add_report(r1);
        server.add_report(r2);

        let diffs = server.compare("run1", "run2").unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].0, "counter.sv");
    }

    #[test]
    fn test_summary() {
        let server = CoverageServer::new();
        server.add_report(make_report("r1"));
        let s = server.summary();
        assert!(s.contains("1 reports"));
        assert!(s.contains("2 total entries"));
    }
}
