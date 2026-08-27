//! ELAB-09: Instance Array with Parameter Override.
//!
//! Supports SystemVerilog generate-based instance arrays with
//! per-element parameter overrides.
//!
//! Example:
//! ```systemverilog
//! # Full array (no override)
//! sub_module u_array [7:0] (.clk(clk), .data(data));
//!
//! # With parameter override per element
//! sub_module u_array [3:0] #(
//!     .WIDTH(8),    // all elements
//!     .DEPTH(2)     // all elements
//! ) (.clk(clk), .data(data));
//!
//! # With per-element override via generate
//! for (genvar i = 0; i < 4; i++) begin : gen
//!     sub_module u (.WIDTH(8 + i*4));
//! end
//! ```

use serde::{Deserialize, Serialize};

/// Instance array definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceArray {
    pub module_name: String,
    pub instance_name: String,
    pub low_index: i64,
    pub high_index: i64,
    pub default_params: Vec<ParamOverride>,
    pub per_element_overrides: Vec<ElementOverride>,
}

/// Parameter override.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamOverride {
    pub name: String,
    pub value: String,
}

/// Per-element parameter override (for generate loops).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementOverride {
    pub index: i64,
    pub overrides: Vec<ParamOverride>,
}

impl InstanceArray {
    /// Number of instances.
    pub fn count(&self) -> usize {
        ((self.high_index - self.low_index).abs() + 1) as usize
    }

    /// Get parameters for a specific element index.
    pub fn params_for(&self, index: i64) -> Vec<ParamOverride> {
        let mut params = self.default_params.clone();

        // Apply per-element overrides
        for override_entry in &self.per_element_overrides {
            if override_entry.index == index {
                for po in &override_entry.overrides {
                    // Replace or add parameter
                    if let Some(existing) = params.iter_mut().find(|p| p.name == po.name) {
                        existing.value = po.value.clone();
                    } else {
                        params.push(po.clone());
                    }
                }
            }
        }

        params
    }

    /// Generate expanded instance list.
    pub fn expand(&self) -> Vec<ExpandedInstance> {
        (self.low_index..=self.high_index)
            .map(|i| ExpandedInstance {
                index: i,
                name: format!("{}[{}]", self.instance_name, i),
                module: self.module_name.clone(),
                params: self.params_for(i),
            })
            .collect()
    }

    /// Summary.
    pub fn summary(&self) -> String {
        format!(
            "InstanceArray: {} {} [{}:{}] ({} instances)",
            self.module_name,
            self.instance_name,
            self.low_index,
            self.high_index,
            self.count(),
        )
    }
}

/// Expanded instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpandedInstance {
    pub index: i64,
    pub name: String,
    pub module: String,
    pub params: Vec<ParamOverride>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_array() {
        let arr = InstanceArray {
            module_name: "sub".into(),
            instance_name: "u_arr".into(),
            low_index: 0,
            high_index: 7,
            default_params: vec![ParamOverride { name: "WIDTH".into(), value: "8".into() }],
            per_element_overrides: vec![],
        };
        assert_eq!(arr.count(), 8);
        assert_eq!(arr.params_for(0).len(), 1);
    }

    #[test]
    fn test_per_element_override() {
        let arr = InstanceArray {
            module_name: "sub".into(),
            instance_name: "u_arr".into(),
            low_index: 0,
            high_index: 3,
            default_params: vec![ParamOverride { name: "WIDTH".into(), value: "8".into() }],
            per_element_overrides: vec![
                ElementOverride {
                    index: 2,
                    overrides: vec![ParamOverride { name: "WIDTH".into(), value: "16".into() }],
                },
            ],
        };

        let p0 = arr.params_for(0);
        assert_eq!(p0[0].value, "8");

        let p2 = arr.params_for(2);
        assert_eq!(p2[0].value, "16");
    }

    #[test]
    fn test_expand() {
        let arr = InstanceArray {
            module_name: "sub".into(),
            instance_name: "u_arr".into(),
            low_index: 0,
            high_index: 2,
            default_params: vec![],
            per_element_overrides: vec![],
        };
        let expanded = arr.expand();
        assert_eq!(expanded.len(), 3);
        assert_eq!(expanded[0].name, "u_arr[0]");
        assert_eq!(expanded[2].name, "u_arr[2]");
    }

    #[test]
    fn test_summary() {
        let arr = InstanceArray {
            module_name: "sub".into(),
            instance_name: "u_arr".into(),
            low_index: 0,
            high_index: 7,
            default_params: vec![],
            per_element_overrides: vec![],
        };
        let s = arr.summary();
        assert!(s.contains("sub"));
        assert!(s.contains("8 instances"));
    }
}
