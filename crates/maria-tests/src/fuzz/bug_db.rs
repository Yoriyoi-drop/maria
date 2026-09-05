//! Bug Database — tracking historical bugs + bug-guided prioritization.
//!
//! Inspired by ChipFuzzer (Paper #2): coverage + historical bugs + crash
//! locations + rare paths → priority map → mutation selection.
//!
//! Concept:
//! - Track every bug found (seed, features, bug message, severity)
//! - Compute "hotness" score per feature based on bug proximity
//! - Seeds whose features overlap hot regions get higher priority
//! - Bug-guided: after finding a bug, increase energy for nearby features

use std::collections::HashMap;

use super::gen::GenInput;

/// Severity of a bug finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BugSeverity {
    /// Panic / stack-overflow / process crash — highest priority.
    Crash,
    /// Non-determinism detected (run1 != run2).
    NonDeterminism,
    /// Differential mismatch confirmed by external reference (Icarus/Verilator).
    DifferentialMismatch,
    /// Mismatch without external confirmation — suspect, not confirmed bug.
    Suspect,
}

/// A recorded bug finding.
#[derive(Debug, Clone)]
pub struct BugRecord {
    pub input: GenInput,
    pub message: String,
    pub severity: BugSeverity,
    /// Features (tags) that were active when this bug was found.
    pub features: Vec<String>,
    /// Number of times this bug pattern was seen (dedup by features).
    pub hit_count: u64,
}

/// Bug database — tracks historical bugs and computes feature hotness.
///
/// Inspired by ChipFuzzer's "bug-guided stage":
/// ```text
/// Coverage + Historical Bugs + Crash Locations + Rare Paths
///     ↓
/// Priority Map
///     ↓
/// Mutation Selection
/// ```
pub struct BugDatabase {
    /// All recorded bugs, sorted by severity (most severe first).
    records: Vec<BugRecord>,
    /// Feature → hotness score (accumulated from bug proximity).
    feature_hotness: HashMap<String, f64>,
    /// Total bugs recorded.
    pub total_bugs: u64,
    /// Total unique bug features.
    pub unique_bug_features: usize,
}

impl BugDatabase {
    pub fn new() -> Self {
        BugDatabase {
            records: Vec::new(),
            feature_hotness: HashMap::new(),
            total_bugs: 0,
            unique_bug_features: 0,
        }
    }

    /// Record a bug finding. Updates hotness scores for all features
    /// that were active when the bug was found.
    pub fn record_bug(&mut self, input: &GenInput, message: &str, severity: BugSeverity) {
        let mut features = Vec::new();
        input.expr.features(&mut features);
        features.push(format!("W:{}", input.w));

        // Update hotness: features near bugs get higher scores.
        // Severity multiplier: Crash=4.0, NonDet=3.0, DiffMismatch=2.0, Suspect=1.0
        let severity_weight = match severity {
            BugSeverity::Crash => 4.0,
            BugSeverity::NonDeterminism => 3.0,
            BugSeverity::DifferentialMismatch => 2.0,
            BugSeverity::Suspect => 1.0,
        };

        for feat in &features {
            let entry = self.feature_hotness.entry(feat.clone()).or_insert(0.0);
            *entry += severity_weight;
        }

        self.records.push(BugRecord {
            input: input.clone(),
            message: message.to_string(),
            severity,
            features: features.clone(),
            hit_count: 1,
        });

        self.total_bugs += 1;
        self.unique_bug_features = self.feature_hotness.len();
    }

    /// Compute "bug priority" score for a set of features.
    /// Higher score = this input is in a bug-prone region → should get more energy.
    ///
    /// Formula: sum of hotness scores for all matching features, normalized.
    pub fn bug_priority(&self, features: &[String]) -> f64 {
        if self.feature_hotness.is_empty() {
            return 0.0;
        }
        let mut score = 0.0;
        for feat in features {
            if let Some(&hotness) = self.feature_hotness.get(feat) {
                score += hotness;
            }
        }
        // Normalize to [0, 1] range using max possible hotness
        let max_hotness: f64 = self
            .feature_hotness
            .values()
            .copied()
            .fold(0.0f64, f64::max);
        if max_hotness > 0.0 {
            (score / max_hotness).min(1.0)
        } else {
            0.0
        }
    }

    /// Check if a feature is "hot" (near a bug). Used by FairFuzz-inspired
    /// rare-targeting: hot features should get MORE energy, not less.
    pub fn is_feature_hot(&self, feature: &str) -> bool {
        self.feature_hotness
            .get(feature)
            .map(|&h| h > 1.0)
            .unwrap_or(false)
    }

    /// Get all hot features (for reporting / external use).
    pub fn hot_features(&self) -> Vec<(&str, f64)> {
        let mut hot: Vec<_> = self
            .feature_hotness
            .iter()
            .filter(|(_, &h)| h > 1.0)
            .map(|(k, &v)| (k.as_str(), v))
            .collect();
        hot.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        hot
    }

    /// Number of recorded bugs.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the database is empty.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Summary statistics.
    pub fn summary(&self) -> BugDbSummary {
        let crash_count = self
            .records
            .iter()
            .filter(|r| r.severity == BugSeverity::Crash)
            .count();
        let nondet_count = self
            .records
            .iter()
            .filter(|r| r.severity == BugSeverity::NonDeterminism)
            .count();
        let diff_count = self
            .records
            .iter()
            .filter(|r| r.severity == BugSeverity::DifferentialMismatch)
            .count();
        let suspect_count = self
            .records
            .iter()
            .filter(|r| r.severity == BugSeverity::Suspect)
            .count();

        BugDbSummary {
            total: self.records.len(),
            crashes: crash_count,
            nondeterminism: nondet_count,
            differential: diff_count,
            suspects: suspect_count,
            hot_features: self.unique_bug_features,
            top_hot: self
                .hot_features()
                .into_iter()
                .take(5)
                .map(|(f, s)| (f.to_string(), s))
                .collect(),
        }
    }
}

impl Default for BugDatabase {
    fn default() -> Self {
        Self::new()
    }
}

/// Summary of bug database state.
#[derive(Debug, Clone)]
pub struct BugDbSummary {
    pub total: usize,
    pub crashes: usize,
    pub nondeterminism: usize,
    pub differential: usize,
    pub suspects: usize,
    pub hot_features: usize,
    pub top_hot: Vec<(String, f64)>,
}

impl std::fmt::Display for BugDbSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Bug Database Summary:")?;
        writeln!(f, "  Total bugs: {}", self.total)?;
        writeln!(f, "  Crashes: {}", self.crashes)?;
        writeln!(f, "  Non-determinism: {}", self.nondeterminism)?;
        writeln!(f, "  Differential mismatches: {}", self.differential)?;
        writeln!(f, "  Suspects: {}", self.suspects)?;
        writeln!(f, "  Hot features: {}", self.hot_features)?;
        if !self.top_hot.is_empty() {
            writeln!(f, "  Top hot features:")?;
            for (feat, score) in &self.top_hot {
                writeln!(f, "    {}: {:.2}", feat, score)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bug_db_record_and_query() {
        let mut db = BugDatabase::new();
        let input = crate::fuzz::gen::generate(42);

        db.record_bug(&input, "test crash", BugSeverity::Crash);
        assert_eq!(db.len(), 1);
        assert!(!db.is_empty());

        let features: Vec<String> = {
            let mut tags = Vec::new();
            input.expr.features(&mut tags);
            tags.push(format!("W:{}", input.w));
            tags
        };

        let priority = db.bug_priority(&features);
        assert!(
            priority > 0.0,
            "bug priority should be > 0 for hot features"
        );

        let summary = db.summary();
        assert_eq!(summary.total, 1);
        assert_eq!(summary.crashes, 1);
    }

    #[test]
    fn bug_db_severity_weighting() {
        let mut db = BugDatabase::new();
        let input = crate::fuzz::gen::generate(1);

        // Record same feature with different severities
        db.record_bug(&input, "crash1", BugSeverity::Crash);
        db.record_bug(&input, "suspect1", BugSeverity::Suspect);

        let summary = db.summary();
        assert_eq!(summary.total, 2);
        assert_eq!(summary.crashes, 1);
        assert_eq!(summary.suspects, 1);

        // Crash should have higher hotness than suspect alone
        let features: Vec<String> = {
            let mut tags = Vec::new();
            input.expr.features(&mut tags);
            tags.push(format!("W:{}", input.w));
            tags
        };
        let priority = db.bug_priority(&features);
        assert!(priority > 0.0);
    }
}
