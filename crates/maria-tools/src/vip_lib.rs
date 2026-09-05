//! ENT-25: Standard Verification IP (VIP) Library.
//!
//! Koleksi reusable assertion templates dan checker untuk
//! common bus protocols. Setiap VIP adalah template SystemVerilog
//! yang bisa di-generate dan di-instantiate.
//!
//! Supported protocols:
//! - APB (AMBA APB3/APB4)
//! - AXI4-Lite
//! - AHB-Lite
//! - Simple handshake

use serde::{Deserialize, Serialize};

/// VIP template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VipTemplate {
    pub name: String,
    pub protocol: String,
    pub version: String,
    pub description: String,
    pub ports: Vec<VipPort>,
    pub assertions: Vec<VipAssertion>,
    pub parameters: Vec<VipParameter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VipPort {
    pub name: String,
    pub direction: String, // "input" or "output"
    pub width: u32,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VipAssertion {
    pub name: String,
    pub description: String,
    pub severity: String, // "error", "warning", "info"
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VipParameter {
    pub name: String,
    pub default: String,
    pub description: String,
}

impl VipTemplate {
    /// Generate SystemVerilog assertion module.
    pub fn generate_sv(&self) -> String {
        let mut out = String::new();

        out.push_str(&format!("// {} — {}\n", self.name, self.description));
        out.push_str("// Auto-generated verification IP\n\n");

        // Module declaration
        out.push_str(&format!("module {}_checker (\n", self.name));
        for (i, port) in self.ports.iter().enumerate() {
            let comma = if i + 1 < self.ports.len() { "," } else { "" };
            out.push_str(&format!(
                "    {} logic [{}:0] {}{}\n",
                port.direction,
                port.width - 1,
                port.name,
                comma,
            ));
        }
        out.push_str(");\n\n");

        // Parameters
        for param in &self.parameters {
            out.push_str(&format!(
                "    parameter {} = {};\n",
                param.name, param.default,
            ));
        }
        if !self.parameters.is_empty() {
            out.push('\n');
        }

        // Assertions
        for assertion in &self.assertions {
            out.push_str(&format!(
                "    // {}: {}\n",
                assertion.name, assertion.description
            ));
            match assertion.severity.as_str() {
                "error" => out.push_str(&format!(
                    "    assert property (@(posedge clk) {})\n",
                    assertion.body,
                )),
                "warning" => out.push_str(&format!(
                    "    assert property (@(posedge clk) {})\n",
                    assertion.body,
                )),
                _ => out.push_str(&format!(
                    "    assume property (@(posedge clk) {})\n",
                    assertion.body,
                )),
            }
            out.push_str(&format!(
                "        else $error(\"{}: assertion failed\");\n\n",
                assertion.name,
            ));
        }

        out.push_str("endmodule\n");
        out
    }
}

/// Generate APB3 checker VIP.
pub fn apb3_vip() -> VipTemplate {
    VipTemplate {
        name: "apb3".into(),
        protocol: "APB3 (AMBA APB)".into(),
        version: "3.0".into(),
        description: "AMBA APB3 protocol checker".into(),
        ports: vec![
            VipPort {
                name: "PCLK".into(),
                direction: "input".into(),
                width: 1,
                description: Some("Clock".into()),
            },
            VipPort {
                name: "PRESETn".into(),
                direction: "input".into(),
                width: 1,
                description: Some("Active-low reset".into()),
            },
            VipPort {
                name: "PSEL".into(),
                direction: "input".into(),
                width: 1,
                description: Some("Peripheral select".into()),
            },
            VipPort {
                name: "PENABLE".into(),
                direction: "input".into(),
                width: 1,
                description: Some("Enable".into()),
            },
            VipPort {
                name: "PWRITE".into(),
                direction: "input".into(),
                width: 1,
                description: Some("Write enable".into()),
            },
            VipPort {
                name: "PREADY".into(),
                direction: "input".into(),
                width: 1,
                description: Some("Ready".into()),
            },
            VipPort {
                name: "PSLVERR".into(),
                direction: "input".into(),
                width: 1,
                description: Some("Slave error".into()),
            },
        ],
        assertions: vec![
            VipAssertion {
                name: "APB_SETUP_PHASE".into(),
                description: "PENABLE must be 0 during setup phase".into(),
                severity: "error".into(),
                body: "$fell(PSEL) || PENABLE == 0".into(),
            },
            VipAssertion {
                name: "APB_ACCESS_PHASE".into(),
                description: "PENABLE must be 1 during access phase (after setup)".into(),
                severity: "error".into(),
                body: "$past(PSEL) && $past(PENABLE) == 0 || PENABLE == 1".into(),
            },
            VipAssertion {
                name: "APB_WRITE_STABLE".into(),
                description: "PADDR/PWDATA/PWRITE must be stable during setup+access".into(),
                severity: "error".into(),
                body: "PSEL && PENABLE |=> PADDR == $past(PADDR)".into(),
            },
            VipAssertion {
                name: "APB_NO_WAIT_STATES".into(),
                description: "APB3 slave must respond in 1 cycle (no wait states)".into(),
                severity: "warning".into(),
                body: "PSEL && PENABLE |=> PREADY".into(),
            },
        ],
        parameters: vec![
            VipParameter {
                name: "ADDR_WIDTH".into(),
                default: "32".into(),
                description: "Address bus width".into(),
            },
            VipParameter {
                name: "DATA_WIDTH".into(),
                default: "32".into(),
                description: "Data bus width".into(),
            },
        ],
    }
}

/// Generate AXI4-Lite checker VIP.
pub fn axi4lite_vip() -> VipTemplate {
    VipTemplate {
        name: "axi4lite".into(),
        protocol: "AXI4-Lite".into(),
        version: "4.0".into(),
        description: "AXI4-Lite protocol checker (simplified)".into(),
        ports: vec![
            VipPort {
                name: "ACLK".into(),
                direction: "input".into(),
                width: 1,
                description: None,
            },
            VipPort {
                name: "ARESETn".into(),
                direction: "input".into(),
                width: 1,
                description: None,
            },
            VipPort {
                name: "AWVALID".into(),
                direction: "input".into(),
                width: 1,
                description: None,
            },
            VipPort {
                name: "AWREADY".into(),
                direction: "input".into(),
                width: 1,
                description: None,
            },
            VipPort {
                name: "WVALID".into(),
                direction: "input".into(),
                width: 1,
                description: None,
            },
            VipPort {
                name: "WREADY".into(),
                direction: "input".into(),
                width: 1,
                description: None,
            },
            VipPort {
                name: "BVALID".into(),
                direction: "input".into(),
                width: 1,
                description: None,
            },
            VipPort {
                name: "BREADY".into(),
                direction: "input".into(),
                width: 1,
                description: None,
            },
            VipPort {
                name: "ARVALID".into(),
                direction: "input".into(),
                width: 1,
                description: None,
            },
            VipPort {
                name: "ARREADY".into(),
                direction: "input".into(),
                width: 1,
                description: None,
            },
            VipPort {
                name: "RVALID".into(),
                direction: "input".into(),
                width: 1,
                description: None,
            },
            VipPort {
                name: "RREADY".into(),
                direction: "input".into(),
                width: 1,
                description: None,
            },
        ],
        assertions: vec![
            VipAssertion {
                name: "AXI4L_AW_HANDSHAKE".into(),
                description: "Write address valid must stay high until ready".into(),
                severity: "error".into(),
                body: "AWVALID && !AWREADY |=> AWVALID".into(),
            },
            VipAssertion {
                name: "AXI4L_AR_HANDSHAKE".into(),
                description: "Read address valid must stay high until ready".into(),
                severity: "error".into(),
                body: "ARVALID && !ARREADY |=> ARVALID".into(),
            },
            VipAssertion {
                name: "AXI4L_W_HANDSHAKE".into(),
                description: "Write data valid must stay high until ready".into(),
                severity: "error".into(),
                body: "WVALID && !WREADY |=> WVALID".into(),
            },
            VipAssertion {
                name: "AXI4L_B_BEFORE_W".into(),
                description: "BVALID cannot be asserted before write handshake".into(),
                severity: "error".into(),
                body: "BVALID |-> $past(WVALID)".into(),
            },
            VipAssertion {
                name: "AXI4L_RESET".into(),
                description: "All valid signals must be deasserted during reset".into(),
                severity: "error".into(),
                body: "!ARESETn |-> !AWVALID && !WVALID && !ARVALID && !BVALID && !RVALID".into(),
            },
        ],
        parameters: vec![
            VipParameter {
                name: "ADDR_WIDTH".into(),
                default: "32".into(),
                description: "Address width".into(),
            },
            VipParameter {
                name: "DATA_WIDTH".into(),
                default: "32".into(),
                description: "Data width".into(),
            },
        ],
    }
}

/// Generate AHB-Lite checker VIP.
pub fn ahblite_vip() -> VipTemplate {
    VipTemplate {
        name: "ahblite".into(),
        protocol: "AHB-Lite".into(),
        version: "5.0".into(),
        description: "AMBA AHB-Lite protocol checker".into(),
        ports: vec![
            VipPort {
                name: "HCLK".into(),
                direction: "input".into(),
                width: 1,
                description: None,
            },
            VipPort {
                name: "HRESETn".into(),
                direction: "input".into(),
                width: 1,
                description: None,
            },
            VipPort {
                name: "HTRANS".into(),
                direction: "input".into(),
                width: 2,
                description: Some("Transfer type".into()),
            },
            VipPort {
                name: "HREADY".into(),
                direction: "input".into(),
                width: 1,
                description: None,
            },
            VipPort {
                name: "HRESP".into(),
                direction: "input".into(),
                width: 2,
                description: Some("Response".into()),
            },
        ],
        assertions: vec![
            VipAssertion {
                name: "AHB_IDLE_OR_NONSEQ".into(),
                description: "HTRANS must be IDLE, NONSEQ, or SEQ (no BUSY in burst)".into(),
                severity: "error".into(),
                body: "HTRANS inside {2'b00, 2'b10, 2'b11}".into(),
            },
            VipAssertion {
                name: "AHB_NO_BUSY".into(),
                description: "BUSY transfer must not follow SEQ or NONSEQ".into(),
                severity: "warning".into(),
                body: "$past(HTRANS) != 2'b01 || HTRANS != 2'b01".into(),
            },
            VipAssertion {
                name: "AHB_WAIT_STATES".into(),
                description: "HREADY low means slave inserting wait states".into(),
                severity: "info".into(),
                body: "1".into(),
            },
        ],
        parameters: vec![
            VipParameter {
                name: "ADDR_WIDTH".into(),
                default: "32".into(),
                description: "Address width".into(),
            },
            VipParameter {
                name: "DATA_WIDTH".into(),
                default: "32".into(),
                description: "Data width".into(),
            },
        ],
    }
}

/// Get all available VIP templates.
pub fn all_vips() -> Vec<VipTemplate> {
    vec![apb3_vip(), axi4lite_vip(), ahblite_vip()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apb3_vip() {
        let vip = apb3_vip();
        assert_eq!(vip.protocol, "APB3 (AMBA APB)");
        assert!(!vip.ports.is_empty());
        assert!(!vip.assertions.is_empty());
    }

    #[test]
    fn test_axi4lite_vip() {
        let vip = axi4lite_vip();
        assert_eq!(vip.name, "axi4lite");
        assert!(vip.ports.len() >= 8);
    }

    #[test]
    fn test_generate_sv() {
        let vip = apb3_vip();
        let sv = vip.generate_sv();
        assert!(sv.contains("module apb3_checker"));
        assert!(sv.contains("assert property"));
        assert!(sv.contains("endmodule"));
    }

    #[test]
    fn test_all_vips() {
        let vips = all_vips();
        assert!(vips.len() >= 3);
        for vip in &vips {
            let sv = vip.generate_sv();
            assert!(sv.contains("endmodule"));
        }
    }

    #[test]
    fn test_vip_parameters() {
        let vip = apb3_vip();
        assert!(!vip.parameters.is_empty());
        let sv = vip.generate_sv();
        assert!(sv.contains("parameter"));
    }
}
