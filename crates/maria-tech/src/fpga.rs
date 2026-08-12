//! Arsitektur fpga-x7 (SYNTHESIS.md §12) — Xilinx 7-series style.
//!
//! Sel: LUT6 (k=6), CARRY4 (4-bit carry chain), FDRE, BUFG, IBUF/OBUF.
//! Nama sel mengikuti konvensi Xilinx agar report familier (Vivado-style);
//! mapping logika TETAP sama — hanya penamaan/area yang berbeda dari generic.

use crate::arch::TechArch;

/// Arsitektur fpga-x7.
#[derive(Debug, Clone, Copy)]
pub struct FpgaX7Arch;

impl TechArch for FpgaX7Arch {
    fn name(&self) -> &'static str {
        "fpga-x7"
    }

    fn lut_inputs(&self) -> usize {
        6
    }

    fn has_carry_chain(&self) -> bool {
        true
    }

    fn lut_cell_name(&self, inputs: usize) -> String {
        let k = inputs.clamp(1, 6);
        format!("LUT{}", k)
    }

    fn carry_cell_name(&self) -> Option<String> {
        Some("CARRY4".into())
    }

    fn ff_cell_name(&self) -> String {
        "FDRE".into()
    }

    fn lut_area(&self) -> f64 {
        1.0
    }

    fn carry_area(&self) -> f64 {
        1.0
    }

    fn ff_area(&self) -> f64 {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fpga_arch_names() {
        let a = FpgaX7Arch;
        assert_eq!(a.name(), "fpga-x7");
        assert_eq!(a.ff_cell_name(), "FDRE");
        assert_eq!(a.carry_cell_name().as_deref(), Some("CARRY4"));
    }
}
