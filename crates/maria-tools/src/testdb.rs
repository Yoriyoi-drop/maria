//! ENT-06: Test Result Database — JSON persistence for tracking test results.
//!
//! Menyimpan hasil test dalam format JSON untuk analisis trend, regression
//! detection, dan reporting. File: `.maria/test-results.json`
//!
//! Fitur:
//! - Simpan hasil test per-run (timestamp, pass/fail/skip, duration, test names)
//! - Load history untuk trend analysis (pass rate, duration trend)
//! - Export untuk CI/CD integration
//! - Compare runs untuk regression detection

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Satu test result (per-function).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub duration_ms: u64,
    pub message: Option<String>,
}

/// Hasil satu test run (kumpulan test results).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRun {
    pub timestamp: u64,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub duration_ms: u64,
    pub results: Vec<TestResult>,
    pub metadata: HashMap<String, String>,
}

/// Database untuk menyimpan history test runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResultDb {
    pub runs: Vec<TestRun>,
    pub max_runs: usize,
}

impl Default for TestResultDb {
    fn default() -> Self {
        TestResultDb {
            runs: Vec::new(),
            max_runs: 100, // simpan 100 run terakhir
        }
    }
}

impl TestResultDb {
    /// Load database dari file, atau buat baru bila tidak ada.
    pub fn load(path: &Path) -> Self {
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(path) {
                if let Ok(db) = serde_json::from_str::<TestResultDb>(&content) {
                    return db;
                }
            }
        }
        Self::default()
    }

    /// Simpan database ke file.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json =
            serde_json::to_string_pretty(self).map_err(|e| format!("gagal serialize: {}", e))?;
        // Atomic write: tulis ke temp file, lalu rename
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &json).map_err(|e| format!("gagal tulis {}: {}", tmp.display(), e))?;
        std::fs::rename(&tmp, path)
            .map_err(|e| format!("gagal rename {} → {}: {}", tmp.display(), path.display(), e))
    }

    /// Tambahkan satu test run ke database.
    pub fn add_run(&mut self, run: TestRun) {
        self.runs.push(run);
        // Batasi jumlah run tersimpan
        if self.runs.len() > self.max_runs {
            let drain_count = self.runs.len() - self.max_runs;
            self.runs.drain(..drain_count);
        }
    }

    /// Buat TestRun baru dari data mentah.
    pub fn create_run(results: Vec<TestResult>, metadata: HashMap<String, String>) -> TestRun {
        let total = results.len();
        let passed = results.iter().filter(|r| r.passed).count();
        let failed = results.iter().filter(|r| !r.passed).count();
        let skipped = 0; // TODO: tambah skip tracking
        let duration_ms = results.iter().map(|r| r.duration_ms).sum();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        TestRun {
            timestamp,
            total,
            passed,
            failed,
            skipped,
            duration_ms,
            results,
            metadata,
        }
    }

    /// Hitung pass rate dari run terakhir.
    pub fn last_pass_rate(&self) -> f64 {
        self.runs
            .last()
            .map(|r| {
                if r.total > 0 {
                    r.passed as f64 / r.total as f64
                } else {
                    0.0
                }
            })
            .unwrap_or(0.0)
    }

    /// Cari regression: test yang pass di run sebelumnya tapi fail di run ini.
    pub fn detect_regression(&self) -> Vec<String> {
        if self.runs.len() < 2 {
            return Vec::new();
        }
        let current = &self.runs[self.runs.len() - 1];
        let previous = &self.runs[self.runs.len() - 2];

        let prev_pass: std::collections::HashSet<&str> = previous
            .results
            .iter()
            .filter(|r| r.passed)
            .map(|r| r.name.as_str())
            .collect();

        current
            .results
            .iter()
            .filter(|r| !r.passed && prev_pass.contains(r.name.as_str()))
            .map(|r| r.name.clone())
            .collect()
    }

    /// Hitung average pass rate dari N run terakhir.
    pub fn avg_pass_rate(&self, n: usize) -> f64 {
        let recent: Vec<&TestRun> = self.runs.iter().rev().take(n).collect();
        if recent.is_empty() {
            return 0.0;
        }
        let total_passed: usize = recent.iter().map(|r| r.passed).sum();
        let total_tests: usize = recent.iter().map(|r| r.total).sum();
        if total_tests > 0 {
            total_passed as f64 / total_tests as f64
        } else {
            0.0
        }
    }

    /// Export summary sebagai JSON string.
    pub fn export_summary(&self) -> String {
        let summary = serde_json::json!({
            "total_runs": self.runs.len(),
            "last_pass_rate": format!("{:.1}%", self.last_pass_rate() * 100.0),
            "avg_pass_rate_5": format!("{:.1}%", self.avg_pass_rate(5) * 100.0),
            "regression": self.detect_regression(),
            "last_run": self.runs.last().map(|r| serde_json::json!({
                "timestamp": r.timestamp,
                "total": r.total,
                "passed": r.passed,
                "failed": r.failed,
                "duration_ms": r.duration_ms,
            })),
        });
        serde_json::to_string_pretty(&summary).unwrap_or_default()
    }
}

/// Default path untuk test result database.
pub fn default_db_path() -> PathBuf {
    PathBuf::from(".maria/test-results.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_db() -> (PathBuf, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test-results.json");
        (path, dir)
    }

    #[test]
    fn test_db_create_and_load() {
        let (path, _dir) = tmp_db();
        let mut db = TestResultDb::default();
        let results = vec![
            TestResult {
                name: "test_a".into(),
                passed: true,
                duration_ms: 10,
                message: None,
            },
            TestResult {
                name: "test_b".into(),
                passed: false,
                duration_ms: 5,
                message: Some("fail".into()),
            },
        ];
        let run = TestResultDb::create_run(results, HashMap::new());
        db.add_run(run);
        db.save(&path).unwrap();

        let loaded = TestResultDb::load(&path);
        assert_eq!(loaded.runs.len(), 1);
        assert_eq!(loaded.runs[0].total, 2);
        assert_eq!(loaded.runs[0].passed, 1);
        assert_eq!(loaded.runs[0].failed, 1);
    }

    #[test]
    fn test_regression_detection() {
        let mut db = TestResultDb::default();

        // Run 1: test_a pass, test_b pass
        let run1 = TestResultDb::create_run(
            vec![
                TestResult {
                    name: "test_a".into(),
                    passed: true,
                    duration_ms: 10,
                    message: None,
                },
                TestResult {
                    name: "test_b".into(),
                    passed: true,
                    duration_ms: 5,
                    message: None,
                },
            ],
            HashMap::new(),
        );
        db.add_run(run1);

        // Run 2: test_a pass, test_b FAIL (regression!)
        let run2 = TestResultDb::create_run(
            vec![
                TestResult {
                    name: "test_a".into(),
                    passed: true,
                    duration_ms: 10,
                    message: None,
                },
                TestResult {
                    name: "test_b".into(),
                    passed: false,
                    duration_ms: 5,
                    message: Some("broken".into()),
                },
            ],
            HashMap::new(),
        );
        db.add_run(run2);

        let regressions = db.detect_regression();
        assert_eq!(regressions.len(), 1);
        assert_eq!(regressions[0], "test_b");
    }

    #[test]
    fn test_pass_rate() {
        let mut db = TestResultDb::default();
        let run = TestResultDb::create_run(
            vec![
                TestResult {
                    name: "t1".into(),
                    passed: true,
                    duration_ms: 1,
                    message: None,
                },
                TestResult {
                    name: "t2".into(),
                    passed: true,
                    duration_ms: 1,
                    message: None,
                },
                TestResult {
                    name: "t3".into(),
                    passed: false,
                    duration_ms: 1,
                    message: None,
                },
            ],
            HashMap::new(),
        );
        db.add_run(run);
        let rate = db.last_pass_rate();
        assert!(
            (rate - 2.0 / 3.0).abs() < 0.001,
            "pass rate should be ~66.7%, got {}",
            rate
        );
    }

    #[test]
    fn test_max_runs_eviction() {
        let mut db = TestResultDb::default();
        db.max_runs = 3;
        for i in 0..5 {
            let run = TestResultDb::create_run(
                vec![TestResult {
                    name: format!("t{}", i),
                    passed: true,
                    duration_ms: 1,
                    message: None,
                }],
                HashMap::new(),
            );
            db.add_run(run);
        }
        assert_eq!(db.runs.len(), 3);
        // Run pertama harus sudah di-evict
        assert_eq!(db.runs[0].results[0].name, "t2");
    }
}
