//! VPI System Task/Function Registration.
//!
//! Allows external C code to register custom system tasks and functions
//! via vpi_register_systf.
#![allow(non_upper_case_globals)]

use super::types::*;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Registered system task/function
pub(crate) struct RegisteredSystf {
    /// Handle unik (bukan posisi — ROUND 36: handle posisi membuat remove
    /// kedua STALE setelah elemen pertama dihapus).
    pub id: u64,
    pub data: s_vpi_systf_data,
}

/// Global registry for system tasks/functions
static VPI_SYSTFS: Mutex<Vec<RegisteredSystf>> = Mutex::new(Vec::new());
static NEXT_SYSTF_ID: AtomicU64 = AtomicU64::new(1);

/// vpi_register_systf(systf_data_p) — register a C system task/function.
pub fn vpi_register_systf(systf_data_p: &s_vpi_systf_data) -> vpiHandle {
    let id = NEXT_SYSTF_ID.fetch_add(1, Ordering::SeqCst);
    let entry = RegisteredSystf {
        id,
        data: systf_data_p.clone(),
    };
    let mut registry = VPI_SYSTFS.lock().unwrap();
    registry.push(entry);
    vpiHandle {
        ptr: id as *mut std::ffi::c_void,
    }
}

/// vpi_remove_systf(systf_handle) — remove a registered system task/function.
pub fn vpi_remove_systf(systf_handle: vpiHandle) -> i32 {
    if systf_handle.is_null() {
        return 0;
    }
    let id = systf_handle.ptr as u64;
    let mut registry = VPI_SYSTFS.lock().unwrap();
    if let Some(pos) = registry.iter().position(|e| e.id == id) {
        registry.remove(pos);
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
                // task_function_type memakai konstanta vpiSystf* (task/function/
                // func-int/dst), BUKAN vpiSysTask/vpiSysFunc (object type — bug
                // lama: task terdaftar dengan vpiSystfTask=1 tak pernah cocok
                // dengan vpiSysTask=12). Per IEEE 1800 §36.7:
                // vpiSystfTask=1, vpiSystfFunc=2, vpiSystfFuncInt=3, ...
                let type_ok = if is_function {
                    matches!(
                        entry.data.task_function_type,
                        vpiSystfFunc
                            | vpiSystfFuncInt
                            | vpiSystfFuncReal
                            | vpiSystfFuncStr
                            | vpiSystfFuncSized
                            | vpiSystfFuncTime
                    )
                } else {
                    entry.data.task_function_type == vpiSystfTask
                };
                if !type_ok {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::os::raw::c_char;
    use std::sync::atomic::{AtomicI32, Ordering};

    static CALLED: AtomicI32 = AtomicI32::new(0);

    extern "C" fn stub_calltf(_user_data: *mut std::ffi::c_void) -> i32 {
        CALLED.fetch_add(1, Ordering::SeqCst);
        0
    }

    fn make_systf(name: &str) -> (CString, s_vpi_systf_data) {
        let cname = CString::new(name).unwrap();
        let data = s_vpi_systf_data {
            task_function_type: vpiSystfTask,
            tfname: cname.as_ptr() as *mut c_char,
            calltf: Some(stub_calltf),
            compiletf: None,
            sizetf: None,
            user_data: std::ptr::null_mut(),
        };
        (cname, data)
    }

    /// LANG-46: VPI systf — vpi_register_systf + call_registered_systf +
    /// vpi_remove_systf bekerja end-to-end (registrasi, dispatch C callback,
    /// penghapusan). Sebelumnya tidak ada test sama sekali untuk modul vpi/.
    #[test]
    fn test_vpi_systf_register_call_remove() {
        let (_cname, data) = make_systf("$my_vpi_task");
        let h = vpi_register_systf(&data);
        assert!(h.is_valid(), "handle hasil registrasi harus valid");

        // Dispatch: engine memanggil berdasarkan nama → calltf stub dieksekusi.
        let found = call_registered_systf("$my_vpi_task", false);
        assert!(found, "systf terdaftar harus ditemukan");
        assert_eq!(CALLED.load(Ordering::SeqCst), 1, "calltf harus terpanggil sekali");

        // Nama yang tidak terdaftar → tidak ditemukan.
        assert!(!call_registered_systf("$unregistered_task", false));

        // Hapus → call berikutnya tidak ditemukan.
        assert_eq!(vpi_remove_systf(h), 1, "remove harus sukses");
        assert!(!call_registered_systf("$my_vpi_task", false));
        assert_eq!(CALLED.load(Ordering::SeqCst), 1, "calltf tidak boleh terpanggil lagi");

        // remove ganda → 0 (sudah tidak ada).
        assert_eq!(vpi_remove_systf(h), 0);
    }
}
