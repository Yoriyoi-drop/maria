//! VPI Callback System — Register and dispatch simulation event callbacks.
//!
//! Supports: cbStartOfSimulation, cbEndOfSimulation, cbReadWriteSynch,
//! cbReadOnlySynch, cbValueChange, cbAfterDelay, cbNextSimTime.

use super::types::*;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Registered VPI callback
struct RegisteredCallback {
    /// Handle unik (bukan posisi di Vec!) — lihat ROUND 36: handle lama =
    /// `registry.len()` (POSISI) → `remove(idx)` menggeser elemen berikutnya
    /// sehingga handle kedua jadi STALE (remove balik 0 padahal masih ada).
    id: u64,
    data: t_cb_data,
}

/// Global callback registry
static VPI_CALLBACKS: Mutex<Vec<RegisteredCallback>> = Mutex::new(Vec::new());
static NEXT_CB_ID: AtomicU64 = AtomicU64::new(1);

/// vpi_register_cb(cb_data_p) — register a callback for a simulation event.
pub fn vpi_register_cb(cb_data_p: &t_cb_data) -> vpiHandle {
    let id = NEXT_CB_ID.fetch_add(1, Ordering::SeqCst);
    let cb = RegisteredCallback {
        id,
        data: cb_data_p.clone(),
    };
    let mut registry = VPI_CALLBACKS.lock().unwrap();
    registry.push(cb);
    vpiHandle {
        ptr: id as *mut std::ffi::c_void,
    }
}

/// vpi_remove_cb(cb_handle) — remove a registered callback.
pub fn vpi_remove_cb(cb_handle: vpiHandle) -> i32 {
    if cb_handle.is_null() {
        return 0;
    }
    let id = cb_handle.ptr as u64;
    let mut registry = VPI_CALLBACKS.lock().unwrap();
    if let Some(pos) = registry.iter().position(|cb| cb.id == id) {
        registry.remove(pos);
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
    pub id: u64,
    pub data: t_cb_data,
}

/// Get a snapshot of the callback registry (for vpi_get_cb_info access).
pub fn get_callback_registry() -> Vec<CallbackEntry> {
    let registry = VPI_CALLBACKS.lock().unwrap();
    registry.iter().map(|cb| CallbackEntry {
        id: cb.id,
        data: cb.data.clone(),
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI32, Ordering};

    // VPI_CALLBACKS adalah global static — serialkan test agar tidak saling
    // menghapus callback saat berjalan paralel (pola sama dgn handle.rs).
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static FIRED: AtomicI32 = AtomicI32::new(0);

    extern "C" fn stub_cb(_data: *mut t_cb_data) -> i32 {
        FIRED.fetch_add(1, Ordering::SeqCst);
        0
    }

    fn make_cb(reason: i32) -> t_cb_data {
        t_cb_data {
            reason,
            cb_rtn: Some(stub_cb),
            user_data: std::ptr::null_mut(),
            time: std::ptr::null_mut(),
            value: std::ptr::null_mut(),
            index: 0,
            obj: vpiHandle::NULL,
            obj_type: 0,
        }
    }

    #[test]
    fn test_vpi_callback_register_fire_remove() {
        let _g = TEST_LOCK.lock().unwrap();
        clear_all_callbacks();
        FIRED.store(0, Ordering::SeqCst);
        let h = vpi_register_cb(&make_cb(cbStartOfSimulation));
        assert!(h.is_valid(), "handle callback harus valid");
        dispatch_start_of_simulation();
        assert_eq!(FIRED.load(Ordering::SeqCst), 1, "callback terpanggil sekali");
        assert_eq!(vpi_remove_cb(h), 1, "remove sukses");
        dispatch_start_of_simulation();
        assert_eq!(FIRED.load(Ordering::SeqCst), 1, "setelah remove tidak terpanggil lagi");
        assert_eq!(vpi_remove_cb(h), 0, "remove ganda → 0");
        clear_all_callbacks();
    }

    #[test]
    fn test_vpi_callback_reason_filter() {
        let _g = TEST_LOCK.lock().unwrap();
        clear_all_callbacks();
        FIRED.store(0, Ordering::SeqCst);
        // register utk cbEndOfSimulation — fire StartOfSimulation tak boleh memanggil
        let h = vpi_register_cb(&make_cb(cbEndOfSimulation));
        dispatch_start_of_simulation();
        assert_eq!(FIRED.load(Ordering::SeqCst), 0, "reason berbeda tak dipanggil");
        dispatch_end_of_simulation();
        assert_eq!(FIRED.load(Ordering::SeqCst), 1, "reason cocok dipanggil");
        assert_eq!(vpi_remove_cb(h), 1);
        clear_all_callbacks();
    }

    #[test]
    fn test_vpi_callback_multiple_fire_all() {
        let _g = TEST_LOCK.lock().unwrap();
        clear_all_callbacks();
        FIRED.store(0, Ordering::SeqCst);
        let h1 = vpi_register_cb(&make_cb(cbReadWriteSynch));
        let h2 = vpi_register_cb(&make_cb(cbReadWriteSynch));
        dispatch_read_write_synch();
        assert_eq!(FIRED.load(Ordering::SeqCst), 2, "dua callback pd reason sama");
        assert_eq!(vpi_remove_cb(h1), 1);
        assert_eq!(vpi_remove_cb(h2), 1);
        clear_all_callbacks();
    }

    #[test]
    fn test_vpi_callback_remove_null() {
        let _g = TEST_LOCK.lock().unwrap();
        clear_all_callbacks();
        assert_eq!(vpi_remove_cb(vpiHandle::NULL), 0, "null handle → 0");
        clear_all_callbacks();
    }

    #[test]
    fn test_vpi_callback_remove_middle_keeps_later_handle() {
        // ROUND 36: regresi bug handle POSISI — register 3 callback, hapus
        // yang tengah, lalu hapus yang ketiga. Handle posisi: setelah
        // remove(posisi 1), elemen 2 menggeser ke posisi 1 → handle ketiga
        // (posisi 3) jadi STALE → remove balik 0. Handle ID unik → tetap 1.
        let _g = TEST_LOCK.lock().unwrap();
        clear_all_callbacks();
        FIRED.store(0, Ordering::SeqCst);
        let h1 = vpi_register_cb(&make_cb(cbReadOnlySynch));
        let h2 = vpi_register_cb(&make_cb(cbReadOnlySynch));
        let h3 = vpi_register_cb(&make_cb(cbReadOnlySynch));
        assert_eq!(vpi_remove_cb(h2), 1, "hapus tengah sukses");
        assert_eq!(vpi_remove_cb(h1), 1, "hapus pertama sukses");
        assert_eq!(vpi_remove_cb(h3), 1, "handle ketiga tetap valid walau posisi bergeser");
        dispatch_read_only_synch();
        assert_eq!(FIRED.load(Ordering::SeqCst), 0, "semua callback sudah dihapus");
        clear_all_callbacks();
    }
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
