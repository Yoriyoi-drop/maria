//! Estimasi area (SYNTHESIS.md §17 — phase 5).
//!
//! Area dalam satuan **unit area** (bukan µm² — teknologi nyata via Liberty
//! di fase 6-7). Model deterministik & transparan:
//!
//! ```text
//! LUT6     = 1.00 unit
//! CARRY4   = 2.00 unit   (per slice 4-bit)
//! FF bit   = 0.50 unit
//! Buffer   = 0.10 unit
//! sel lain = 0.25 unit
//! ```
//!
//! `area.rpt` = resource count + total unit area.

use maria_netlist::cell::CellKind;
use maria_netlist::net::Netlist;

/// Hasil estimasi area.
#[derive(Debug, Clone, Default)]
pub struct AreaReport {
    pub lut: usize,
    pub carry4: usize,
    pub ff: usize,
    pub buf: usize,
    pub other: usize,
    /// Total unit area.
    pub area_units: f64,
}

/// Estimasi area atas netlist (resource-based).
pub fn estimate_area(nl: &Netlist) -> AreaReport {
    let mut r = AreaReport::default();
    let mut units = 0.0;
    for c in &nl.cells {
        match &c.kind {
            CellKind::Lut { .. } => {
                r.lut += 1;
                units += 1.0;
            }
            CellKind::Carry4 => {
                // Slice-equivalent: satu sel CARRY4 bit-vector = ceil(width/4)
                // slice (konsisten dgn `tech_map` carry4_count).
                let slices = (c.width.max(1) + 3) / 4;
                r.carry4 += slices;
                units += 2.0 * slices as f64;
            }
            CellKind::Dff | CellKind::DffE | CellKind::DffR { .. } | CellKind::DffRE { .. } => {
                r.ff += c.width.max(1);
                units += 0.5 * c.width.max(1) as f64;
            }
            CellKind::Buffer => {
                r.buf += 1;
                units += 0.10;
            }
            _ => {
                r.other += 1;
                units += 0.25;
            }
        }
    }
    r.area_units = units;
    r
}

/// Render report teks (area.rpt / stdout).
pub fn render_area_report(r: &AreaReport) -> String {
    let mut s = String::new();
    s.push_str("── Area Report (estimate)\n");
    s.push_str(&format!("  LUT6        {:>6}\n", r.lut));
    s.push_str(&format!("  CARRY4      {:>6}\n", r.carry4));
    s.push_str(&format!("  FF bits     {:>6}\n", r.ff));
    s.push_str(&format!("  Buffer      {:>6}\n", r.buf));
    s.push_str(&format!("  other       {:>6}\n", r.other));
    s.push_str(&format!("  ──────────────────\n"));
    s.push_str(&format!("  area        {:>8.2} units\n", r.area_units));
    s.push_str("  (model: LUT=1.0, CARRY4=2.0, FF=0.5, BUF=0.1 — teknologi\n");
    s.push_str("   nyata via Liberty .lib menyusul fase 6-7)\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use maria_core::intern::Symbol;
    use maria_netlist::cell::{CellInstance, PinConn};
    use maria_netlist::net::Netlist;

    #[test]
    fn area_of_lut_ff_netlist() {
        let mut nl = Netlist::new(Symbol::intern("top"));
        let a = nl.add_net(Symbol::intern("a"), 1);
        let q = nl.add_net(Symbol::intern("q"), 1);
        let clk = nl.add_net(Symbol::intern("clk"), 1);
        let mut l = CellInstance::new(Symbol::intern("u0"), CellKind::Lut { init: 0x1 }, 1);
        l.inputs = vec![PinConn {
            net: a,
            pin: "i0".into(),
            bit: None,
        }];
        l.outputs = vec![PinConn {
            net: a,
            pin: "o".into(),
            bit: None,
        }];
        let mut ff = CellInstance::new(Symbol::intern("q_reg"), CellKind::Dff, 1);
        ff.inputs = vec![
            PinConn {
                net: clk,
                pin: "c".into(),
                bit: None,
            },
            PinConn {
                net: a,
                pin: "d".into(),
                bit: None,
            },
        ];
        ff.outputs = vec![PinConn {
            net: q,
            pin: "q".into(),
            bit: None,
        }];
        nl.add_cell(l);
        nl.add_cell(ff);
        let r = estimate_area(&nl);
        assert_eq!(r.lut, 1);
        assert_eq!(r.ff, 1);
        assert!((r.area_units - 1.5).abs() < 1e-9, "1 LUT + 1 FF = 1.5");
    }
}
