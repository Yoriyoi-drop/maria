//! VHPI Callback System (IEEE 1076-2008 §C.5.8) — register/remove/dispatch.
//!
//! Callback VHPI masuk ke antrian `ForeignEvent` (foreign/mod.rs) dan
//! dieksekusi oleh scheduler Maria — bukan langsung dari thread library
//! (arsitektur masukan user poin 5).
//!
//! Handle callback memakai unique id (AtomicU64) — BUKAN posisi registry
//! (pelajaran ROUND 36: handle posisi STALE setelah remove menggeser elemen).
#![allow(non_upper_case_globals)]

use super::handle::VhpiHandle;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::sync::OnceLock;

// ─── Callback Reasons (IEEE 1076-2008) ───

pub const vhpiCbValueChange: i32 = 1;
pub const vhpiCbStartOfSimulation: i32 = 2;
pub const vhpiCbEndOfSimulation: i32 = 3;
pub const vhpiCbTimeStep: i32 = 4;
pub const vhpiCbAfterDelay: i32 = 5;
pub const vhpiCbReadWriteSynch: i32 = 6;
pub const vhpiCbReadOnlySynch: i32 = 7;
pub const vhpiCbNextTimeStep: i32 = 8;
pub const vhpiCbTerminate: i32 = 9;
pub const vhpiCbEnterInteractive: i32 = 10;

/// C-compatible struct callback data (pola vpi t_cb_data).
#[repr(C)]
#[derive(Debug, Clone)]
pub struct t_vhpi_cb_data {
    pub reason: i32,
    pub cb_rtn: Option<unsafe extern "C" fn(*mut t_vhpi_cb_data) -> i32>,
    pub user_data: *mut std::ffi::c_void,
    pub obj: VhpiHandle,
    pub time: *mut super::value::t_vhpi_time,
}

unsafe impl Send for t_vhpi_cb_data {}
unsafe impl Sync for t_vhpi_cb_data {}

pub(crate) struct RegisteredVhpiCb {
    pub id: u64,
    pub data: t_vhpi_cb_data,
}

fn vhpi_callbacks() -> &'static Mutex<Vec<RegisteredVhpiCb>> {
    static CB: OnceLock<Mutex<Vec<RegisteredVhpiCb>>> = OnceLock::new();
    CB.get_or_init(|| Mutex::new(Vec::new()))
}

static NEXT_CB_ID: AtomicU64 = AtomicU64::new(1);

/// vhpi_register_cb(cb_data_p) — daftarkan callback.
pub fn vhpi_register_cb(cb_data_p: &t_vhpi_cb_data) -> VhpiHandle {
    let id = NEXT_CB_ID.fetch_add(1, Ordering::SeqCst);
    vhpi_callbacks().lock().unwrap().push(RegisteredVhpiCb {
        id,
        data: cb_data_p.clone(),
    });
    VhpiHandle { ptr: id as *mut std::ffi::c_void }
}

/// vhpi_remove_cb(handle) — hapus callback by unique id.
pub fn vhpi_remove_cb(handle: VhpiHandle) -> i32 {
    if handle.is_null() { return 0; }
    let id = handle.ptr as u64;
    let mut reg = vhpi_callbacks().lock().unwrap();
    if let Some(pos) = reg.iter().position(|c| c.id == id) {
        reg.remove(pos);
        1
    } else {
        0
    }
}

/// Fire callback dengan reason tertentu. Clone registry dulu, drop lock,
/// lalu panggil — cegah deadlock bila callback register/remove lagi.
pub fn dispatch_callback(reason: i32) {
    let snapshot: Vec<t_vhpi_cb_data> = {
        let reg = vhpi_callbacks().lock().unwrap();
        reg.iter()
            .filter(|c| c.data.reason == reason)
            .map(|c| c.data.clone())
            .collect()
    };
    for data in snapshot {
        if let Some(cb) = data.cb_rtn {
            let mut d = data;
            unsafe { cb(&mut d); }
        }
    }
}

pub fn dispatch_start_of_simulation() {
    dispatch_callback(vhpiCbStartOfSimulation);
}

pub fn dispatch_end_of_simulation() {
    dispatch_callback(vhpiCbEndOfSimulation);
    dispatch_callback(vhpiCbTerminate);
}

pub fn dispatch_time_step() {
    dispatch_callback(vhpiCbTimeStep);
    dispatch_callback(vhpiCbNextTimeStep);
}

pub fn dispatch_synch() {
    dispatch_callback(vhpiCbReadWriteSynch);
    dispatch_callback(vhpiCbReadOnlySynch);
}

pub fn clear_all_callbacks() {
    vhpi_callbacks().lock().unwrap().clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicI32;

    static TEST_LOCK: Mutex<()> = Mutex::new(());
    static FIRED: AtomicI32 = AtomicI32::new(0);

    extern "C" fn stub_cb(_data: *mut t_vhpi_cb_data) -> i32 {
        FIRED.fetch_add(1, Ordering::SeqCst);
        0
    }

    fn make_cb(reason: i32) -> t_vhpi_cb_data {
        t_vhpi_cb_data {
            reason,
            cb_rtn: Some(stub_cb),
            user_data: std::ptr::null_mut(),
            obj: VhpiHandle::NULL,
            time: std::ptr::null_mut(),
        }
    }

    #[test]
    fn test_vhpi_cb_register_fire_remove() {
        let _g = TEST_LOCK.lock().unwrap();
        clear_all_callbacks();
        FIRED.store(0, Ordering::SeqCst);
        let h = vhpi_register_cb(&make_cb(vhpiCbStartOfSimulation));
        assert!(h.is_valid());
        dispatch_start_of_simulation();
        assert_eq!(FIRED.load(Ordering::SeqCst), 1, "callback terpanggil sekali");
        assert_eq!(vhpi_remove_cb(h), 1);
        dispatch_start_of_simulation();
        assert_eq!(FIRED.load(Ordering::SeqCst), 1, "setelah remove tidak terpanggil");
        clear_all_callbacks();
    }

    #[test]
    fn test_vhpi_cb_reason_filter() {
        let _g = TEST_LOCK.lock().unwrap();
        clear_all_callbacks();
        FIRED.store(0, Ordering::SeqCst);
        let h = vhpi_register_cb(&make_cb(vhpiCbEndOfSimulation));
        dispatch_start_of_simulation();
        assert_eq!(FIRED.load(Ordering::SeqCst), 0, "reason berbeda tak dipanggil");
        dispatch_end_of_simulation();
        assert_eq!(FIRED.load(Ordering::SeqCst), 1, "reason cocok dipanggil");
        assert_eq!(vhpi_remove_cb(h), 1);
        clear_all_callbacks();
    }

    #[test]
    fn test_vhpi_cb_remove_middle_keeps_later_handle() {
        // Regresi ROUND 36: handle posisi STALE — remove tengah harus
        // tidak mempengaruhi handle berikutnya (unique id).
        let _g = TEST_LOCK.lock().unwrap();
        clear_all_callbacks();
        FIRED.store(0, Ordering::SeqCst);
        let h1 = vhpi_register_cb(&make_cb(vhpiCbReadWriteSynch));
        let h2 = vhpi_register_cb(&make_cb(vhpiCbReadWriteSynch));
        let h3 = vhpi_register_cb(&make_cb(vhpiCbReadWriteSynch));
        // hapus tengah (h2) + pertama (h1)
        assert_eq!(vhpi_remove_cb(h2), 1);
        assert_eq!(vhpi_remove_cb(h1), 1);
        // h3 tetap valid → fire 1x (hanya h3)
        dispatch_synch();
        assert_eq!(FIRED.load(Ordering::SeqCst), 1, "h3 harus tetap valid setelah h1+h2 dihapus");
        assert_eq!(vhpi_remove_cb(h3), 1);
        clear_all_callbacks();
    }

    #[test]
    fn test_vhpi_cb_remove_null() {
        assert_eq!(vhpi_remove_cb(VhpiHandle::NULL), 0);
    }
}
