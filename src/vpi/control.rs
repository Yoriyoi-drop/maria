//! VPI Control Functions — vpi_control, vpi_get_time.
//!
//! Provides simulation control (stop, finish, reset) and time access.

use super::types::*;

/// vpi_control(operation, ...) — control simulation execution.
pub fn vpi_control(operation: i32, _arg1: i32, _arg2: i32) -> i32 {
    match operation {
        vpiStop => {
            super::with_vpi_engine(|engine| {
                engine.running = false;
                engine.paused = true;
            });
            1
        }
        vpiFinish => {
            super::with_vpi_engine(|engine| {
                engine.running = false;
            });
            1
        }
        vpiReset => {
            // Reset simulation is complex — not fully implemented
            0
        }
        vpiSetInteractiveScope => {
            // Not implemented (interactive mode)
            0
        }
        _ => 0,
    }
}

/// vpi_get_time(handle, time_p) — get the current simulation time.
pub fn vpi_get_time(_handle: vpiHandle, time_p: &mut t_vpi_time) -> i32 {
    super::with_vpi_engine(|engine| {
        let t = engine.state.time;
        time_p.ttype = 0; // Scaled real time
        time_p.low = t as u32;
        time_p.high = (t >> 32) as u32;
        time_p.real = t as f64;
        1
    }).unwrap_or(0)
}

/// vpi_get_cb_info(cb_handle, cb_data_p) — get info about a registered callback.
pub fn vpi_get_cb_info(_cb_handle: vpiHandle, _cb_data_p: &mut t_cb_data) -> i32 {
    // Not fully implemented — returns 0 to indicate the callback info is not available
    0
}
