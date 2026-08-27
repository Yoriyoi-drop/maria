//! ENT-34: Place & Route Tool Integration — ICC2/Innovus interface.
//!
//! Generates P&R-compatible reports and configuration for
//! design implementation flow.

use serde::{Deserialize, Serialize};

/// P&R report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrReport {
    pub design_name: String,
    pub tool: String, // "icc2" or "innovus"
    pub placement: PlacementReport,
    pub routing: RoutingReport,
    pub drc: DrcReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlacementReport {
    pub total_cells: u64,
    pub placed_cells: u64,
    pub utilisation: f64,
    pub congestion: f64,
    pub timing_slack: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingReport {
    pub total_nets: u64,
    pub routed_nets: u64,
    pub via_count: u64,
    pub wire_length: f64,
    pub detour_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrcReport {
    pub violations: u64,
    pub violation_types: Vec<(String, u64)>,
}

impl PrReport {
    /// Generate ICC2-compatible report.
    pub fn to_icc2_report(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ICC2 Place & Route Report: {}\n", self.design_name));
        out.push_str(&format!("Tool: {}\n\n", self.tool));

        out.push_str("Placement:\n");
        out.push_str(&format!("  Total cells:   {}\n", self.placement.total_cells));
        out.push_str(&format!("  Placed cells:  {}\n", self.placement.placed_cells));
        out.push_str(&format!("  Utilisation:   {:.1}%\n", self.placement.utilisation * 100.0));
        out.push_str(&format!("  Congestion:    {:.2}\n", self.placement.congestion));

        out.push_str("\nRouting:\n");
        out.push_str(&format!("  Total nets:    {}\n", self.routing.total_nets));
        out.push_str(&format!("  Routed nets:   {}\n", self.routing.routed_nets));
        out.push_str(&format!("  Wire length:   {:.0}\n", self.routing.wire_length));
        out.push_str(&format!("  Detour ratio:  {:.2}\n", self.routing.detour_ratio));

        out.push_str(&format!("\nDRC: {} violations\n", self.drc.violations));
        for (vtype, count) in &self.drc.violation_types {
            out.push_str(&format!("  {}: {}\n", vtype, count));
        }

        out
    }

    /// Generate Innovus-compatible report.
    pub fn to_innovus_report(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("=== Innovus P&R Report: {} ===\n\n", self.design_name));
        out.push_str(&format!(
            "Utilization: {:.1}% | Congestion: {:.2} | DRC: {}\n",
            self.placement.utilisation * 100.0,
            self.placement.congestion,
            self.drc.violations,
        ));
        out.push_str(&format!(
            "Nets: {}/{} routed | Wire: {:.0} | Detour: {:.2}\n",
            self.routing.routed_nets,
            self.routing.total_nets,
            self.routing.wire_length,
            self.routing.detour_ratio,
        ));
        out
    }

    /// Check if design passes DRC.
    pub fn passes_drc(&self) -> bool {
        self.drc.violations == 0
    }

    /// Summary.
    pub fn summary(&self) -> String {
        format!(
            "{}: util={:.0}% DRC={} wire={:.0}",
            self.design_name,
            self.placement.utilisation * 100.0,
            self.drc.violations,
            self.routing.wire_length,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_report() -> PrReport {
        PrReport {
            design_name: "top".into(),
            tool: "icc2".into(),
            placement: PlacementReport {
                total_cells: 10000,
                placed_cells: 9800,
                utilisation: 0.75,
                congestion: 0.8,
                timing_slack: -0.1,
            },
            routing: RoutingReport {
                total_nets: 5000,
                routed_nets: 4950,
                via_count: 20000,
                wire_length: 150000.0,
                detour_ratio: 1.2,
            },
            drc: DrcReport {
                violations: 3,
                violation_types: vec![("short".into(), 2), ("spacing".into(), 1)],
            },
        }
    }

    #[test]
    fn test_icc2_report() {
        let report = make_report();
        let text = report.to_icc2_report();
        assert!(text.contains("ICC2"));
        assert!(text.contains("75"));
    }

    #[test]
    fn test_innovus_report() {
        let report = make_report();
        let text = report.to_innovus_report();
        assert!(text.contains("Innovus"));
    }

    #[test]
    fn test_drc_check() {
        let mut report = make_report();
        assert!(!report.passes_drc());
        report.drc.violations = 0;
        assert!(report.passes_drc());
    }

    #[test]
    fn test_summary() {
        let report = make_report();
        let s = report.summary();
        assert!(s.contains("top"));
        assert!(s.contains("75%"));
    }
}
