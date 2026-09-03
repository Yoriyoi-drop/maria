//! Adaptive Mutation Scheduler — dynamic mutation operator probabilities.
//!
//! Inspired by MOpt (Paper #7): instead of fixed mutation probabilities,
//! adaptively adjust which mutation operators get more "tries" based on
//! their effectiveness at finding new coverage or bugs.
//!
//! Concept:
//! ```text
//! Mutation Operators
//!  ├─ AST replace      → probability p1
//!  ├─ subtree delete   → probability p2
//!  ├─ expression mutate → probability p3
//!  ├─ width mutate     → probability p4
//!  ├─ type mutate      → probability p5
//!  ├─ generate mutate  → probability p6
//!  └─ timing mutate    → probability p7
//!
//!        ↓ (feedback: new coverage? new bug?)
//!
//! Reward/Score per operator
//!        ↓
//! Scheduler
//!        ↓
//! Updated probabilities (PSO-inspired)
//! ```

use std::collections::HashMap;

/// Mutation operator types — each maps to a specific kind of AST mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MutationOp {
    /// Replace entire expression node with a new random one.
    AstReplace,
    /// Delete a subtree (replace with literal 0).
    SubtreeDelete,
    /// Duplicate a subtree (wrap in unary or binary).
    SubtreeDuplicate,
    /// Swap two subtrees within the same expression.
    SubtreeSwap,
    /// Mutate leaf values (literals, variables, indices).
    LeafMutate,
    /// Change bit width to a boundary value.
    WidthMutate,
    /// Change operator type (e.g., Add → Sub).
    OperatorSwap,
    /// Inject X/Z literal for 4-state testing.
    XInject,
}

impl MutationOp {
    pub fn all() -> &'static [MutationOp] {
        &[
            MutationOp::AstReplace,
            MutationOp::SubtreeDelete,
            MutationOp::SubtreeDuplicate,
            MutationOp::SubtreeSwap,
            MutationOp::LeafMutate,
            MutationOp::WidthMutate,
            MutationOp::OperatorSwap,
            MutationOp::XInject,
        ]
    }

    pub fn name(self) -> &'static str {
        match self {
            MutationOp::AstReplace => "ast_replace",
            MutationOp::SubtreeDelete => "subtree_delete",
            MutationOp::SubtreeDuplicate => "subtree_duplicate",
            MutationOp::SubtreeSwap => "subtree_swap",
            MutationOp::LeafMutate => "leaf_mutate",
            MutationOp::WidthMutate => "width_mutate",
            MutationOp::OperatorSwap => "operator_swap",
            MutationOp::XInject => "x_inject",
        }
    }
}

/// Per-operator statistics for adaptive scheduling.
#[derive(Debug, Clone)]
struct OpStats {
    /// Number of times this operator was selected.
    selects: u64,
    /// Number of times this operator produced new coverage.
    successes: u64,
    /// Number of times this operator found a bug.
    bug_finds: u64,
    /// Current probability weight (adapted over time).
    weight: f64,
}

impl OpStats {
    fn new(initial_weight: f64) -> Self {
        OpStats {
            selects: 0,
            successes: 0,
            bug_finds: 0,
            weight: initial_weight,
        }
    }

    /// Compute fitness score: (successes + 2*bug_finds) / (selects + 1)
    /// The +1 prevents division by zero and provides a smoothing effect.
    fn fitness(&self) -> f64 {
        (self.successes as f64 + 2.0 * self.bug_finds as f64) / (self.selects as f64 + 1.0)
    }
}

/// Adaptive Mutation Scheduler using PSO-inspired weight adaptation.
///
/// After each mutation, the scheduler observes whether it produced new
/// coverage or found a bug, and adjusts operator probabilities accordingly.
pub struct AdaptiveMutationScheduler {
    stats: HashMap<MutationOp, OpStats>,
    /// Global iteration counter.
    pub total_iterations: u64,
    /// Learning rate for weight updates (higher = more responsive).
    learning_rate: f64,
    /// Minimum weight for any operator (prevents starvation).
    min_weight: f64,
    /// Maximum weight for any operator (prevents domination).
    max_weight: f64,
}

impl AdaptiveMutationScheduler {
    pub fn new() -> Self {
        let mut stats = HashMap::new();
        // Initial uniform weights
        let initial_weight = 1.0 / MutationOp::all().len() as f64;
        for &op in MutationOp::all() {
            stats.insert(op, OpStats::new(initial_weight));
        }
        AdaptiveMutationScheduler {
            stats,
            total_iterations: 0,
            learning_rate: 0.1,
            min_weight: 0.02,
            max_weight: 0.5,
        }
    }

    /// Select the next mutation operator based on current weights.
    pub fn select_op(&mut self, rng: &mut fastrand::Rng) -> MutationOp {
        let ops = MutationOp::all();
        let total_weight: f64 = ops.iter().map(|op| self.stats[op].weight).sum();

        // Weighted random selection
        let r = rng.f64() * total_weight;
        let mut cumulative = 0.0;
        for &op in ops {
            cumulative += self.stats[&op].weight;
            if r <= cumulative {
                self.stats.get_mut(&op).unwrap().selects += 1;
                self.total_iterations += 1;
                return op;
            }
        }
        // Fallback (should not happen due to floating point)
        let last = ops[ops.len() - 1];
        self.stats.get_mut(&last).unwrap().selects += 1;
        self.total_iterations += 1;
        last
    }

    /// Report the outcome of a mutation. Call this after running the
    /// mutated input through the oracle.
    pub fn report_outcome(
        &mut self,
        op: MutationOp,
        new_coverage: bool,
        bug_found: bool,
    ) {
        if let Some(stats) = self.stats.get_mut(&op) {
            if new_coverage {
                stats.successes += 1;
            }
            if bug_found {
                stats.bug_finds += 1;
            }

            // PSO-inspired weight update:
            // - If success: increase weight proportionally to fitness
            // - If failure: decrease weight slightly
            let fitness = stats.fitness();
            let adjustment = if new_coverage || bug_found {
                self.learning_rate * fitness
            } else {
                -self.learning_rate * 0.01 // Small decay
            };

            stats.weight = (stats.weight + adjustment).clamp(self.min_weight, self.max_weight);
        }

        // Normalize all weights to sum to 1.0
        self.normalize_weights();
    }

    /// Normalize all operator weights to sum to 1.0.
    fn normalize_weights(&mut self) {
        let total: f64 = self.stats.values().map(|s| s.weight).sum();
        if total > 0.0 {
            for stats in self.stats.values_mut() {
                stats.weight /= total;
            }
        }
    }

    /// Get current probability distribution (for reporting).
    pub fn probabilities(&self) -> Vec<(MutationOp, f64)> {
        let mut probs: Vec<_> = self
            .stats
            .iter()
            .map(|(&op, stats)| (op, stats.weight))
            .collect();
        probs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        probs
    }

    /// Get fitness scores for all operators (for debugging).
    pub fn fitness_scores(&self) -> Vec<(MutationOp, f64)> {
        let mut scores: Vec<_> = self
            .stats
            .iter()
            .map(|(&op, stats)| (op, stats.fitness()))
            .collect();
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scores
    }

    /// Summary for logging.
    pub fn summary(&self) -> String {
        let probs = self.probabilities();
        let mut result = format!("AdaptiveMutationScheduler ({} iterations):\n", self.total_iterations);
        for (op, prob) in &probs {
            let stats = &self.stats[op];
            result.push_str(&format!(
                "  {:20} p={:.3}  selects={}  successes={}  bugs={}\n",
                op.name(),
                prob,
                stats.selects,
                stats.successes,
                stats.bug_finds
            ));
        }
        result
    }
}

impl Default for AdaptiveMutationScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_selects_all_operators() {
        let mut scheduler = AdaptiveMutationScheduler::new();
        let mut rng = fastrand::Rng::with_seed(42);
        let mut selected = std::collections::HashSet::new();

        // Run enough iterations to expect all operators to be selected
        for _ in 0..1000 {
            let op = scheduler.select_op(&mut rng);
            selected.insert(op);
        }

        // With 8 operators and uniform initial weights, all should be selected
        assert_eq!(
            selected.len(),
            MutationOp::all().len(),
            "all operators should be selected at least once"
        );
    }

    #[test]
    fn scheduler_adapts_to_success() {
        let mut scheduler = AdaptiveMutationScheduler::new();
        let mut rng = fastrand::Rng::with_seed(42);

        // First, get initial probabilities
        let initial_probs = scheduler.probabilities();
        let initial_weight_subtree_del = initial_probs
            .iter()
            .find(|(op, _)| *op == MutationOp::SubtreeDelete)
            .map(|(_, p)| *p)
            .unwrap();

        // Report many successes for SubtreeDelete
        for _ in 0..100 {
            scheduler.report_outcome(MutationOp::SubtreeDelete, true, false);
        }

        // After many successes, SubtreeDelete should have higher weight
        let adapted_probs = scheduler.probabilities();
        let adapted_weight_subtree_del = adapted_probs
            .iter()
            .find(|(op, _)| *op == MutationOp::SubtreeDelete)
            .map(|(_, p)| *p)
            .unwrap();

        assert!(
            adapted_weight_subtree_del > initial_weight_subtree_del,
            "SubtreeDelete weight should increase after successes: {} -> {}",
            initial_weight_subtree_del,
            adapted_weight_subtree_del
        );
    }

    #[test]
    fn scheduler_weights_sum_to_one() {
        let mut scheduler = AdaptiveMutationScheduler::new();
        let mut rng = fastrand::Rng::with_seed(42);

        for i in 0..50 {
            let op = scheduler.select_op(&mut rng);
            scheduler.report_outcome(op, i % 3 == 0, i % 10 == 0);
        }

        let total: f64 = scheduler
            .probabilities()
            .iter()
            .map(|(_, p)| p)
            .sum();
        assert!(
            (total - 1.0).abs() < 0.001,
            "probabilities should sum to 1.0, got {}",
            total
        );
    }
}
