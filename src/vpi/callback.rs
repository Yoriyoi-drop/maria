//! VPI Callback System — Register and dispatch simulation event callbacks.
//!
//! Supports: cbStartOfSimulation, cbEndOfSimulation, cbReadWriteSynch,
//! cbReadOnlySynch, cbValueChange, cbAfterDelay, cbNextSimTime.

use super::types::*;
use std::sync::Mutex;

/// Registered VPI callback
struct RegisteredCallback {
    data: t_cb_data,
}

/// Global callback registry
static VPI_CALLBACKS: Mutex<Vec<RegisteredCallback>> = Mutex::new(Vec::new());

/// vpi_register_cb(cb_data_p) — register a callback for a simulation event.
pub fn vpi_register_cb(cb_data_p: &t_cb_data) -> vpiHandle {
    let cb = RegisteredCallback {
        data: cb_data_p.clone(),
    };
    let mut registry = VPI_CALLBACKS.lock().unwrap();
    registry.push(cb);
    // Return a handle to the callback (for vpi_remove_cb)
    let idx = registry.len();
    vpiHandle {
        ptr: idx as *mut std::ffi::c_void,
    }
}

/// vpi_remove_cb(cb_handle) — remove a registered callback.
pub fn vpi_remove_cb(cb_handle: vpiHandle) -> i32 {
    if cb_handle.is_null() {
        return 0;
    }
    let idx = cb_handle.ptr as usize - 1;
    let mut registry = VPI_CALLBACKS.lock().unwrap();
    if idx < registry.len() {
        registry.remove(idx);
        1
    } else {
        0
    }
}

/// Fire all callbacks matching a specific reason code.
/// Clone the registry FIRST, drop the lock, then call callbacks.
/// This prevents deadlocks if callbacks call vpi_register_cb/vpi_remove_cb.
pub fn fire_callbacks(reason: i32) {
    let snapshot: Vec<t_cb_data> = {
        let registry = VPI_CALLBACKS.lock().unwrap();
        registry.iter()
            .filter(|cb| cb.data.reason == reason)
            .map(|cb| cb.data.clone())
            .collect()
    };
    for mut cb_data in snapshot {
        if let Some(cb_fn) = cb_data.cb_rtn {
            unsafe {
                cb_fn(&mut cb_data);
            }
        }
    }
}

/// Fire all value-change callbacks for a given signal.
/// Clone matching callbacks FIRST, drop the lock, then call each.
pub fn fire_value_change_callbacks(sig_name: &str, _old_val: &t_vpi_value, _new_val: &t_vpi_value) {
    let matching: Vec<t_cb_data> = {
        let registry = VPI_CALLBACKS.lock().unwrap();
        registry.iter()
            .filter(|cb| {
                if cb.data.reason != cbValueChange {
                    return false;
                }
                if !cb.data.obj.is_null() {
                    let obj_name = super::handle::vpi_get_str(super::types::vpiName, cb.data.obj);
                    if !obj_name.is_null() {
                        let name = unsafe { super::types::cstr_to_str(obj_name) };
                        return name == sig_name;
                    }
                }
                false
            })
            .map(|cb| cb.data.clone())
            .collect()
    };
    for mut cb_data in matching {
        if let Some(cb_fn) = cb_data.cb_rtn {
            unsafe {
                cb_fn(&mut cb_data);
            }
        }
    }
}

/// Remove all callbacks (called at end of simulation).
pub fn clear_all_callbacks() {
    let mut registry = VPI_CALLBACKS.lock().unwrap();
    registry.clear();
}

/// Public struct to expose RegisteredCallback data for vpi_get_cb_info
#[derive(Debug, Clone)]
pub struct CallbackEntry {
    pub data: t_cb_data,
}

/// Get a snapshot of the callback registry (for vpi_get_cb_info access).
pub fn get_callback_registry() -> Vec<CallbackEntry> {
    let registry = VPI_CALLBACKS.lock().unwrap();
    registry.iter().map(|cb| CallbackEntry {
        data: cb.data.clone(),
    }).collect()
}

// ─── Callback Dispatch Points ───

/// Called at the start of simulation (time 0 initialization).
pub fn dispatch_start_of_simulation() {
    fire_callbacks(cbStartOfSimulation);
}

/// Called at the end of simulation ($finish).
pub fn dispatch_end_of_simulation() {
    fire_callbacks(cbEndOfSimulation);
}

/// Called after each time step (read-write synchronization point).
pub fn dispatch_read_write_synch() {
    fire_callbacks(cbReadWriteSynch);
}

/// Called after all events for a time step are processed.
pub fn dispatch_read_only_synch() {
    fire_callbacks(cbReadOnlySynch);
}
