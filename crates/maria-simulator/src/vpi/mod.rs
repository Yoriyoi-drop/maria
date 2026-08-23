//! VPI (Verilog Procedural Interface) — IEEE 1800-2012 Section 36.
//!
//! Implementasi VPI untuk Maria Simulator. Menyediakan akses ke objek desain,
//! signal, callback, dan kontrol simulasi dari C code via FFI.
//!
//! Coverage:
//! - vpi_handle / vpi_handle_by_name / vpi_iterate / vpi_scan
//! - vpi_get / vpi_get_str (properties)
//! - vpi_get_value / vpi_put_value
//! - vpi_get_time / vpi_get_cb_info
//! - vpi_register_cb / vpi_remove_cb
//! - vpi_register_systf / vpi_remove_systf
//! - vpi_control
//! - vpi_free_object / vpi_chk_error

pub mod ffi;
pub mod handle;
pub mod value;
pub mod callback;
pub mod control;
pub mod systf;
pub mod types;

use crate::simulator::engine::SimulationEngine;
thread_local! {
    /// Pointer ke `SimulationEngine` aktif, per-thread.
    /// Safety contract: engine VPI di-set/diakses dari thread simulasi yang
    /// sama (semua caller `with_vpi_engine` berjalan di thread `run()`).
    /// Thread-local (bukan global `Mutex`) mencegah deref pointer dangling
    /// saat beberapa simulasi berjalan paralel — engine thread lain yang
    /// sudah drop tidak pernah terlihat dari thread ini.
    static VPI_ENGINE: std::cell::Cell<*mut SimulationEngine> =
        const { std::cell::Cell::new(std::ptr::null_mut()) };
}

/// Register the current simulation engine for VPI access.
pub fn set_vpi_engine(engine: &mut SimulationEngine) {
    VPI_ENGINE.with(|e| e.set(engine));
}

/// Clear the VPI engine reference (called at end of simulation).
pub fn clear_vpi_engine() {
    VPI_ENGINE.with(|e| e.set(std::ptr::null_mut()));
}

/// Get a mutable reference to the current VPI engine.
/// Returns None if no engine is registered on this thread.
pub fn with_vpi_engine<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut SimulationEngine) -> R,
{
    VPI_ENGINE.with(|e| unsafe { e.get().as_mut() }.map(f))
}
