//! FEAT-10: Coverage Closure Analytics — track test contributions to coverage.
//!
//! Maps which tests contribute to which coverage points,
//! identifies coverage gaps, and suggests tests to improve closure.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// A coverage point (line, branch, or toggle).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoveragePoint {
    pub id: String,
    pub point_type: PointType,
    pub file: String,
    pub line: u32,
    pub column: Option<u32>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PointType {
    Line,
    Branch,
    Toggle,
    FSM,
    Assertion,
    Expression,
}

/// Coverage contribution from a test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestContribution {
    pub test_name: String,
    pub covered_points: Vec<String>, // point IDs
    pub unique_points: Vec<String>,  // points only covered by this test
}

/// Coverage closure analytics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageClosure {
    pub points: Vec<CoveragePoint>,
    pub test_contributions: Vec<TestContribution>,
    pub uncovered_points: Vec<String>,
}

impl CoverageClosure {
    pub fn new() -> Self {
        CoverageClosure {
            points: Vec::new(),
            test_contributions: Vec::new(),
            uncovered_points: Vec::new(),
        }
    }

    /// Add coverage points.
    pub fn add_points(&mut self, points: Vec<CoveragePoint>) {
        self.points.extend(points);
    }

    /// Record test contribution.
    pub fn record_contribution(&mut self, test: &str, covered: Vec<String>) {
        self.test_contributions.push(TestContribution {
            test_name: test.to_string(),
            covered_points: covered.clone(),
            unique_points: Vec::new(),
        });
        // Recompute unique points
        self.recompute_unique();
    }

    fn recompute_unique(&mut self) {
        let _all_covered: HashMap<&str, usize> = HashMap::new();
        let mut point_count: HashMap<String, usize> = HashMap::new();
        for tc in &self.test_contributions {
            for pid in &tc.covered_points {
                *point_count.entry(pid.clone()).or_insert(0) += 1;
            }
        }
        for tc in &mut self.test_contributions {
            tc.unique_points = tc.covered_points.iter()
                .filter(|pid| point_count.get(pid.as_str()).copied().unwrap_or(0) == 1)
                .cloned()
                .collect();
        }
    }

    /// Compute overall coverage.
    pub fn overall_coverage(&self) -> f64 {
        if self.points.is_empty() { return 100.0; }
        let covered: HashSet<&str> = self.test_contributions.iter()
            .flat_map(|tc| tc.covered_points.iter().map(|s| s.as_str()))
            .collect();
        covered.len() as f64 / self.points.len() as f64 * 100.0
    }

    /// Find uncovered points.
    pub fn find_uncovered(&self) -> Vec<&CoveragePoint> {
        let covered: HashSet<&str> = self.test_contributions.iter()
            .flat_map(|tc| tc.covered_points.iter().map(|s| s.as_str()))
            .collect();
        self.points.iter()
            .filter(|p| !covered.contains(p.id.as_str()))
            .collect()
    }

    /// Find "critical" tests (that cover unique points).
    pub fn critical_tests(&self) -> Vec<&TestContribution> {
        self.test_contributions.iter()
            .filter(|tc| !tc.unique_points.is_empty())
            .collect()
    }

    /// Coverage by type.
    pub fn coverage_by_type(&self) -> BTreeMap<String, f64> {
        let covered: HashSet<&str> = self.test_contributions.iter()
            .flat_map(|tc| tc.covered_points.iter().map(|s| s.as_str()))
            .collect();
        let mut by_type: HashMap<String, (usize, usize)> = HashMap::new();
        for p in &self.points {
            let key = format!("{:?}", p.point_type);
            let entry = by_type.entry(key).or_insert((0, 0));
            entry.0 += 1;
            if covered.contains(p.id.as_str()) {
                entry.1 += 1;
            }
        }
        by_type.into_iter()
            .map(|(k, (total, cov))| (k, cov as f64 / total as f64 * 100.0))
            .collect()
    }

    /// Summary.
    pub fn summary(&self) -> String {
        let overall = self.overall_coverage();
        let uncovered = self.find_uncovered().len();
        let critical = self.critical_tests().len();
        format!(
            "CoverageClosure: {:.1}% overall, {} uncovered, {} critical tests, {} total points",
            overall, uncovered, critical, self.points.len(),
        )
    }

    /// Save to JSON.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())
    }

    /// Load from JSON.
    pub fn load(path: &Path) -> Result<Self, String> {
        let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&json).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_point(id: &str, pt: PointType) -> CoveragePoint {
        CoveragePoint {
            id: id.into(),
            point_type: pt,
            file: "test.sv".into(),
            line: 1,
            column: None,
            description: None,
        }
    }

    #[test]
    fn test_overall_coverage() {
        let mut cc = CoverageClosure::new();
        cc.add_points(vec![
            make_point("p1", PointType::Line),
            make_point("p2", PointType::Line),
            make_point("p3", PointType::Branch),
        ]);
        cc.record_contribution("test_a", vec!["p1".into()]);
        cc.record_contribution("test_b", vec!["p2".into()]);
        assert!((cc.overall_coverage() - 66.67).abs() < 0.1);
    }

    #[test]
    fn test_uncovered() {
        let mut cc = CoverageClosure::new();
        cc.add_points(vec![
            make_point("p1", PointType::Line),
            make_point("p2", PointType::Line),
        ]);
        cc.record_contribution("test_a", vec!["p1".into()]);
        let uncovered = cc.find_uncovered();
        assert_eq!(uncovered.len(), 1);
        assert_eq!(uncovered[0].id, "p2");
    }

    #[test]
    fn test_critical_tests() {
        let mut cc = CoverageClosure::new();
        cc.add_points(vec![
            make_point("p1", PointType::Line),
            make_point("p2", PointType::Line),
        ]);
        cc.record_contribution("test_a", vec!["p1".into()]);
        cc.record_contribution("test_b", vec!["p1".into(), "p2".into()]);
        let critical = cc.critical_tests();
        assert!(critical.iter().any(|tc| tc.test_name == "test_b"));
    }

    #[test]
    fn test_summary() {
        let mut cc = CoverageClosure::new();
        cc.add_points(vec![make_point("p1", PointType::Line)]);
        cc.record_contribution("test_a", vec!["p1".into()]);
        let s = cc.summary();
        assert!(s.contains("100.0%"));
    }
}
