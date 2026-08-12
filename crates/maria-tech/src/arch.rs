//! Arsitektur teknologi — abstraksi device untuk synthesis (SYNTHESIS.md §12).
//!
//! Maria tidak menganggap semua hardware punya gate yang sama. Teknologi nyata
//! (LUT width, carry chain, nama sel, area) hanya lewat trait [`TechArch`].
//! Back-end bawaan: `generic` (LUT6 + CARRY4 + FF) dan `fpga` (fpga-x7, sel
//! Xilinx-style). `asic`/`custom` menyusul di phase 6/7 (Liberty `.lib`).
//!
//! ```text
//! Teknologi
//! ├── cells        (LUT6, CARRY4, FDRE, ...)
//! ├── lut_inputs   (6 input per LUT)
//! ├── area         (LUT/CARRY/FF dalam satuan device)
//! └── constraints  (BUFG, IO, dst. — phase 8)
//! ```

/// Arsitektur teknologi abstrak.
///
/// Trait ini adalah SEGALANYA yang boleh diketahui core synthesis tentang
/// device. Tidak ada logika device di dalam `maria-synth`/`maria-sir` —
/// mapper hanya memakai `lut_inputs()` dan nama sel.
pub trait TechArch {
    /// Nama arsitektur (untuk report): `generic`, `fpga-x7`.
    fn name(&self) -> &'static str;

    /// Jumlah input per LUT (LUT6 → 6).
    fn lut_inputs(&self) -> usize;

    /// Apakah device punya carry chain eksplisit (CARRY4 pada FPGA).
    fn has_carry_chain(&self) -> bool;

    /// Nama sel LUT untuk `k` input (mapping memilih LUT terkecil yang muat).
    fn lut_cell_name(&self, inputs: usize) -> String;

    /// Nama sel carry chain (None bila tidak ada).
    fn carry_cell_name(&self) -> Option<String>;

    /// Nama sel FF (Xilinx: FDRE; generic: FF).
    fn ff_cell_name(&self) -> String;

    /// Estimasi area 1 LUT.
    fn lut_area(&self) -> f64;

    /// Estimasi area 1 sel carry chain.
    fn carry_area(&self) -> f64;

    /// Estimasi area 1 FF.
    fn ff_area(&self) -> f64;
}
