//! VHPI — VHDL Procedural Interface (IEEE 1076-2008 Annex C).
//!
//! Adapter ABI-compatible di atas simulation kernel Maria (arsitektur
//! masukan user poin 1 & 4). Library VHDL eksternal memanggil fungsi
//! `vhpi_*` standar dan menganggap Maria simulator VHDL-kompatibel.
//!
//! Scope implementasi: object model (design unit/architecture/signal/port/
//! process/instance), handle-by-name, iterate/scan, get/put value (logic,
//! int, real, string), callback (start/end sim, time step, value change,
//! read-write/read-only synch), control (stop/finish), release handle.
//! VHDL *language parsing* di luar scope — Maria adalah simulator
//! SystemVerilog; VHPI di sini menyediakan ABI layer untuk integrasi
//! toolchain VHDL (mixed-language) sesuai IEEE 1076-2008.

pub mod api;
pub mod callback;
pub mod handle;
pub mod iterator;
pub mod loader;
pub mod object;
pub mod value;

pub use handle::{VhpiHandle, VhpiObjectKind};
pub use object::{vhpi_get, vhpi_get_str, vhpi_handle_by_name, vhpi_is_defined};

/// LOCK TEST BERSAMA (fix flake ROUND 106): registry/callback VHPI adalah
/// global static — test di handle/object/callback/iterator yang saling
/// `clear_all` harus serial satu sama lain, bukan hanya sesama file.
/// Lock ini HANYA untuk #[cfg(test)] lintas modul vhpi.
#[cfg(test)]
pub(crate) static VHPI_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
