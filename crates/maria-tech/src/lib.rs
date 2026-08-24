//! Maria Technology Library — abstraksi device untuk synthesis (SYNTHESIS.md §12).
//!
//! Maria tidak menganggap semua hardware punya gate yang sama. Teknologi hanya
//! lewat trait [`arch::TechArch`]; tidak ada logika device di dalam core
//! synthesis. Back-end:
//!
//! | Back-end | Sel | Status |
//! |----------|-----|--------|
//! | `generic` | LUT6, CARRY4, FF | phase 4 (tech mapping generik) |
//! | `fpga-x7` | LUT6, CARRY4, FDRE | phase 4 (mapping + report) |
//! | `asic` | cell library dari `.lib` | phase 6/7 (Liberty parser) |
//! | `custom` | library hardware sendiri | menyusul |
//!
//! Pilih arsitektur di CLI: `maria synth --preset generic|fpga|asic|custom`.

pub mod arch;
pub mod fpga;
pub mod generic;
pub mod liberty;

pub use arch::TechArch;
pub use fpga::FpgaX7Arch;
pub use generic::GenericArch;
pub use liberty::{load_mdb, parse_liberty, save_mdb};
pub use liberty::{LibertyCell, LibertyLibrary, LibertyPin, PinDir, TimingArc};

/// Ambil arsitektur berdasarkan nama preset (bukan teknologi — mapping di
/// `maria-synth`). Preset `asic`/`custom` belum punya back-end → `None`.
pub fn arch_for(name: &str) -> Option<Box<dyn TechArch>> {
    match name {
        "generic" => Some(Box::new(GenericArch)),
        "fpga" => Some(Box::new(FpgaX7Arch)),
        _ => None,
    }
}

/// Versi library (untuk header report).
pub const VERSION: &str = "0.1.0";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arch_for_presets() {
        assert!(arch_for("generic").is_some());
        assert!(arch_for("fpga").is_some());
        assert!(arch_for("asic").is_none(), "asi c belum punya back-end");
        assert!(arch_for("custom").is_none());
    }
}
