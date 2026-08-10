//! VPI System Task/Function Registration.
//!
//! Allows external C code to register custom system tasks and functions
//! via vpi_register_systf.

use super::types::*;
use std::sync::Mutex;

/// Registered system task/function
pub(crate) struct RegisteredSystf {
    pub data: s_vpi_systf_data,
}

/// Global registry for system tasks/functions
static VPI_SYSTFS: Mutex<Vec<RegisteredSystf>> = Mutex::new(Vec::new());

/// vpi_register_systf(systf_data_p) — register a C system task/function.
pub fn vpi_register_systf(systf_data_p: &s_vpi_systf_data) -> vpiHandle {
    let entry = RegisteredSystf {
        data: systf_data_p.clone(),
    };
    let mut registry = VPI_SYSTFS.lock().unwrap();
    registry.push(entry);
    let idx = registry.len();
    vpiHandle {
        ptr: idx as *mut std::ffi::c_void,
    }
}

/// vpi_remove_systf(systf_handle) — remove a registered system task/function.
pub fn vpi_remove_systf(systf_handle: vpiHandle) -> i32 {
    if systf_handle.is_null() {
        return 0;
    }
    let idx = systf_handle.ptr as usize - 1;
    let mut registry = VPI_SYSTFS.lock().unwrap();
    if idx < registry.len() {
        registry.remove(idx);
        1
    } else {
        0
    }
}

/// Find and call a registered system task/function by name.
/// Clone the registry FIRST, drop the lock, then call the C function.
/// This prevents deadlocks if the C function tries to register/unregister
/// another systf during its execution.
/// Returns true if found and called.
pub fn call_registered_systf(name: &str, is_function: bool) -> bool {
    let snapshot: Vec<s_vpi_systf_data> = {
        let registry = VPI_SYSTFS.lock().unwrap();
        registry.iter()
            .filter_map(|entry| {
                if entry.data.tfname.is_null() {
                    return None;
                }
                let tfname = unsafe {
                    match std::ffi::CStr::from_ptr(entry.data.tfname).to_str() {
                        Ok(s) => s.to_string(),
                        Err(_) => return None,
                    }
                };
                if tfname != name {
                    return None;
                }
                let expected_type = if is_function { vpiSysFunc } else { vpiSysTask };
                if entry.data.task_function_type != expected_type
                    && entry.data.task_function_type != vpiSystfFuncInt
                {
                    return None;
                }
                Some(entry.data.clone())
            })
            .collect()
    };
    for entry in snapshot {
        if let Some(calltf) = entry.calltf {
            let mut dummy: i32 = 0;
            let ptr = &mut dummy as *mut i32 as *mut std::ffi::c_void;
            unsafe {
                calltf(ptr);
            }
            return true;
        }
    }
    false
}

/// Call the compile-time function for all registered systfs.
/// Clone the registry FIRST, drop the lock, then call each compile function.
pub fn compile_all_systfs() {
    let snapshot: Vec<s_vpi_systf_data> = {
        let registry = VPI_SYSTFS.lock().unwrap();
        registry.iter().map(|entry| entry.data.clone()).collect()
    };
    for entry in snapshot {
        if let Some(compiletf) = entry.compiletf {
            let mut dummy: i32 = 0;
            let ptr = &mut dummy as *mut i32 as *mut std::ffi::c_void;
            unsafe { compiletf(ptr); }
        }
    }
}

/// Clear all registered system tasks/functions.
pub fn clear_all_systfs() {
    let mut registry = VPI_SYSTFS.lock().unwrap();
    registry.clear();
}
