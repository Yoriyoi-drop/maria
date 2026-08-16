//! Maria emulator — Hardware-Software Emulator (lihat EMULATOR.md).
//!
//! R0: **MHIR (Maria Hardware IR)** — ekstraksi struktur hardware dari
//! `IrDesign` (clock/reset/register/memory/device) lengkap dengan back-pointer
//! ke source RTL, plus dump text (memory map / MHIR). Di atas MHIR inilah
//! Machine Engine (CPU interpreter/JIT, Device ABI, co-simulation) dibangun
//! pada fase berikutnya.
//!
//! Pipeline: `IrDesign` (maria-ir) → `mhir::extract` → `MhirDesign` → `dump`.
//! Aturan 1 file = 1 tanggung jawab: types / backptr / extract / dump terpisah.

pub mod config;
pub mod cpu;
pub mod dump;
pub mod elf;
pub mod machine;
pub mod mem;
pub mod mhir;
