//! ENT-33: Synthesis Tool Integration — report format for DC/Genus/Yosys.
//!
//! Generates synthesis-compatible reports and configuration files.

use serde::{Deserialize, Serialize};

/// Synthesis report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthReport {
    pub module_name: String,
    pub target_library: String,
    pub area: AreaReport,
    pub timing: TimingReport,
    pub power: PowerReport,
    pub cell_count: CellCount,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AreaReport {
    pub total_area: f64,
    pub cell_area: f64,
    pub wire_area: f64,
    pub fill_area: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingReport {
    pub wns: f64,
    pub tns: f64,
    pub fep: u32,
    pub clock_period: f64,
    pub clock_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerReport {
    pub total_power: f64,
    pub internal_power: f64,
    pub switching_power: f64,
    pub leakage_power: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellCount {
    pub total: u64,
    pub combinational: u64,
    pub sequential: u64,
    pub buffer: u64,
    pub inverter: u64,
}

impl SynthReport {
    /// Generate Yosys-compatible report.
    pub fn to_yosys_report(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("=== Yosys Synthesis Report: {} ===\n\n", self.module_name));
        out.push_str(&format!("Target library: {}\n\n", self.target_library));

        out.push_str("Area:\n");
        out.push_str(&format!("  Total:  {:.2}\n", self.area.total_area));
        out.push_str(&format!("  Cell:   {:.2}\n", self.area.cell_area));
        out.push_str(&format!("  Wire:   {:.2}\n", self.area.wire_area));

        out.push_str("\nTiming:\n");
        out.push_str(&format!("  Clock: {} ({:.2}ns)\n", self.timing.clock_name, self.timing.clock_period));
        out.push_str(&format!("  WNS:   {:.3}ns\n", self.timing.wns));
        out.push_str(&format!("  TNS:   {:.3}ns\n", self.timing.tns));

        out.push_str("\nPower:\n");
        out.push_str(&format!("  Total:     {:.3}mW\n", self.power.total_power));
        out.push_str(&format!("  Internal:  {:.3}mW\n", self.power.internal_power));
        out.push_str(&format!("  Switching: {:.3}mW\n", self.power.switching_power));
        out.push_str(&format!("  Leakage:   {:.3}mW\n", self.power.leakage_power));

        out.push_str("\nCell Count:\n");
        out.push_str(&format!("  Total:         {}\n", self.cell_count.total));
        out.push_str(&format!("  Combinational: {}\n", self.cell_count.combinational));
        out.push_str(&format!("  Sequential:    {}\n", self.cell_count.sequential));

        out
    }

    /// Generate Design Compiler command script.
    pub fn to_dc_script(&self) -> String {
        format!(
            "# Design Compiler synthesis script for {}\n\
             set target_library {lib}\n\
             read_verilog input.sv\n\
             link\n\
             compile\n\
             report_timing > timing.rpt\n\
             report_area > area.rpt\n\
             report_power > power.rpt\n\
             write -format verilog -hierarchy -output netlist.sv\n",
            self.module_name,
            lib = self.target_library,
        )
    }

    /// Generate Genus-compatible report.
    pub fn to_genus_report(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("Genus Synthesis Report: {}\n", self.module_name));
        out.push_str(&format!("WNS: {:.3} TNS: {:.3}\n", self.timing.wns, self.timing.tns));
        out.push_str(&format!("Area: {:.2} Power: {:.3}mW\n", self.area.total_area, self.power.total_power));
        out.push_str(&format!("Cells: {} (seq: {} comb: {})\n", self.cell_count.total, self.cell_count.sequential, self.cell_count.combinational));
        out
    }

    /// Check if design meets timing.
    pub fn meets_timing(&self) -> bool {
        self.timing.wns >= 0.0
    }

    /// Summary.
    pub fn summary(&self) -> String {
        format!(
            "{}: area={:.0}, WNS={:.3}, power={:.3}mW, cells={}",
            self.module_name,
            self.area.total_area,
            self.timing.wns,
            self.power.total_power,
            self.cell_count.total,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_report() -> SynthReport {
        SynthReport {
            module_name: "counter".into(),
            target_library: "typical.db".into(),
            area: AreaReport {
                total_area: 1234.5,
                cell_area: 1000.0,
                wire_area: 200.0,
                fill_area: 34.5,
            },
            timing: TimingReport {
                wns: -0.15,
                tns: -0.45,
                fep: 2,
                clock_period: 10.0,
                clock_name: "clk".into(),
            },
            power: PowerReport {
                total_power: 5.234,
                internal_power: 2.0,
                switching_power: 2.5,
                leakage_power: 0.734,
            },
            cell_count: CellCount {
                total: 500,
                combinational: 300,
                sequential: 150,
                buffer: 30,
                inverter: 20,
            },
        }
    }

    #[test]
    fn test_yosys_report() {
        let report = make_report();
        let text = report.to_yosys_report();
        assert!(text.contains("Yosys"));
        assert!(text.contains("counter"));
    }

    #[test]
    fn test_dc_script() {
        let report = make_report();
        let script = report.to_dc_script();
        assert!(script.contains("compile"));
        assert!(script.contains("typical.db"));
    }

    #[test]
    fn test_genus_report() {
        let report = make_report();
        let text = report.to_genus_report();
        assert!(text.contains("Genus"));
    }

    #[test]
    fn test_meets_timing() {
        let mut report = make_report();
        assert!(!report.meets_timing());
        report.timing.wns = 0.0;
        assert!(report.meets_timing());
        report.timing.wns = 0.5;
        assert!(report.meets_timing());
    }

    #[test]
    fn test_summary() {
        let report = make_report();
        let s = report.summary();
        assert!(s.contains("counter"));
        assert!(s.contains("500"));
    }
}
