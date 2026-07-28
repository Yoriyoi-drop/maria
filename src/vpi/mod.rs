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
use std::sync::Mutex;

/// Wrapper to make *mut SimulationEngine Send (needed for Mutex).
/// Safety: VPI engine is set/get from the same thread.
pub(crate) struct EnginePtr(pub *mut SimulationEngine);
unsafe impl Send for EnginePtr {}
unsafe impl Sync for EnginePtr {}

/// Global VPI state: a pointer to the current SimulationEngine.
/// VPI callbacks and system tasks need access to the engine.
static VPI_ENGINE: Mutex<Option<EnginePtr>> = Mutex::new(None);

/// Register the current simulation engine for VPI access.
pub fn set_vpi_engine(engine: &mut SimulationEngine) {
    let ptr = EnginePtr(engine as *mut SimulationEngine);
    *VPI_ENGINE.lock().unwrap() = Some(ptr);
}

/// Clear the VPI engine reference (called at end of simulation).
pub fn clear_vpi_engine() {
    *VPI_ENGINE.lock().unwrap() = None;
}

/// Get a mutable reference to the current VPI engine.
/// Returns None if no engine is registered.
pub fn with_vpi_engine<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut SimulationEngine) -> R,
{
    let guard = VPI_ENGINE.lock().unwrap();
    guard.as_ref().and_then(|engine_ptr| {
        let engine = unsafe { engine_ptr.0.as_mut()? };
        Some(f(engine))
    })
}
