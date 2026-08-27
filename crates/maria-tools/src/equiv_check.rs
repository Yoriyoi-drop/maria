//! COMP-16: Sequential Equivalence Checking — foundation.
//!
//! Provides basic structure for checking if two RTL designs are
//! functionally equivalent (combinational and sequential).

use serde::{Deserialize, Serialize};

/// Equivalence check result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EquivResult {
    pub equivalent: bool,
    pub method: String,
    pub counter_example: Option<CounterExample>,
    pub proof_time_ms: u64,
}

/// Counter-example for non-equivalent designs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterExample {
    pub inputs: Vec<String>,
    pub expected_outputs: Vec<String>,
    pub actual_outputs: Vec<String>,
    pub cycle: u64,
}

/// Equivalence checker.
pub struct EquivChecker {
    method: String,
}

impl EquivChecker {
    pub fn new(method: &str) -> Self {
        EquivChecker {
            method: method.to_string(),
        }
    }

    /// Check combinational equivalence of two module signal maps.
    pub fn check_combinational(
        &self,
        signal_map: &[(String, String)],
        golden_values: &[(String, Vec<u64>)],
        impl_values: &[(String, Vec<u64>)],
    ) -> EquivResult {
        let start = std::time::Instant::now();

        let mut equivalent = true;
        let mut counter = None;

        for (output_name, golden) in golden_values {
            if let Some((_, impl_vals)) = impl_values.iter().find(|(n, _)| n == output_name) {
                for (cycle, (g, i)) in golden.iter().zip(impl_vals.iter()).enumerate() {
                    if g != i {
                        equivalent = false;
                        counter = Some(CounterExample {
                            inputs: signal_map.iter().map(|(a, _)| a.clone()).collect(),
                            expected_outputs: golden.iter().map(|v| format!("{:x}", v)).collect(),
                            actual_outputs: impl_vals.iter().map(|v| format!("{:x}", v)).collect(),
                            cycle: cycle as u64,
                        });
                        break;
                    }
                }
            }
        }

        EquivResult {
            equivalent,
            method: self.method.clone(),
            counter_example: counter,
            proof_time_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// Check sequential equivalence (stub — needs formal engine).
    pub fn check_sequential(
        &self,
        _golden_ir: &str,
        _impl_ir: &str,
        _depth: u32,
    ) -> EquivResult {
        // Stub: returns inconclusive
        EquivResult {
            equivalent: false,
            method: format!("{} (sequential)", self.method),
            counter_example: None,
            proof_time_ms: 0,
        }
    }

    /// Summary.
    pub fn summary(&self) -> String {
        format!("EquivChecker: method={}", self.method)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_combinational_equivalent() {
        let checker = EquivChecker::new("miter");
        let golden = vec![
            ("y0".to_string(), vec![0, 1, 1, 0]),
            ("y1".to_string(), vec![1, 0, 1, 1]),
        ];
        let impl_vals = vec![
            ("y0".to_string(), vec![0, 1, 1, 0]),
            ("y1".to_string(), vec![1, 0, 1, 1]),
        ];
        let result = checker.check_combinational(&[], &golden, &impl_vals);
        assert!(result.equivalent);
    }

    #[test]
    fn test_combinational_not_equivalent() {
        let checker = EquivChecker::new("miter");
        let golden = vec![("y0".to_string(), vec![0, 1, 1, 0])];
        let impl_vals = vec![("y0".to_string(), vec![0, 1, 0, 0])];
        let result = checker.check_combinational(&[], &golden, &impl_vals);
        assert!(!result.equivalent);
        assert!(result.counter_example.is_some());
    }

    #[test]
    fn test_sequential_stub() {
        let checker = EquivChecker::new("miter");
        let result = checker.check_sequential("a", "b", 100);
        assert!(!result.equivalent);
        assert!(result.method.contains("sequential"));
    }

    #[test]
    fn test_summary() {
        let checker = EquivChecker::new("bit-blasting");
        assert!(checker.summary().contains("bit-blasting"));
    }
}
