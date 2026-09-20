//! Arsitektur generic (SYNTHESIS.md §12) — LUT6 + CARRY4 + FF.
//!
//! Back-end paling sederhana yang bisa menjalankan seluruh flow synthesis:
//! semua logika → LUT6 (6 input, init 64-bit), adder → carry chain CARRY4
//! (4 bit per slice), register → FF. Nama sel tanpa vendor (netlist
//! technology-agnostic, bisa di-diff/commit).

use crate::arch::TechArch;

/// Arsitektur generic.
#[derive(Debug, Clone, Copy)]
pub struct GenericArch;

impl TechArch for GenericArch {
    fn name(&self) -> &'static str {
        "generic"
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
        "FF".into()
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
    fn generic_arch_defaults() {
        let a = GenericArch;
        assert_eq!(a.name(), "generic");
        assert_eq!(a.lut_inputs(), 6);
        assert!(a.has_carry_chain());
        assert_eq!(a.lut_cell_name(3), "LUT3");
        assert_eq!(a.lut_cell_name(6), "LUT6");
        assert_eq!(a.lut_cell_name(9), "LUT6", "clamp ke LUT6");
        assert_eq!(a.carry_cell_name().as_deref(), Some("CARRY4"));
    }
}
