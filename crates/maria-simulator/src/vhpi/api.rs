//! VHPI C ABI entry points — `#[no_mangle] extern "C"` mengikuti IEEE
//! 1076-2008 (vhpi_user.h). Library eksternal memanggil fungsi ini melalui
//! ABI standar — Maria menyediakan adapter, bukan menerjemahkan library ke
//! Rust (arsitektur masukan user poin 3).
#![allow(non_upper_case_globals)]

use super::callback::{self, t_vhpi_cb_data};
use super::handle::VhpiHandle;
use super::iterator;
use super::object;
use super::value::{self, t_vhpi_value};

// ─── Control Types (IEEE 1076-2008) ───

pub const vhpiStop: i32 = 1;
pub const vhpiFinish: i32 = 2;
pub const vhpiReset: i32 = 3;
pub const vhpiSetInteractiveScope: i32 = 5;

/// vhpi_handle_by_name(name, scope) — temukan object by hierarchical name.
#[no_mangle]
pub unsafe extern "C" fn vhpi_handle_by_name(
    name: *const std::os::raw::c_char,
    scope: VhpiHandle,
) -> VhpiHandle {
    if name.is_null() {
        return VhpiHandle::NULL;
    }
    let s = match std::ffi::CStr::from_ptr(name).to_str() {
        Ok(s) => s,
        Err(_) => return VhpiHandle::NULL,
    };
    object::vhpi_handle_by_name(s, scope)
}

/// vhpi_iterate(kind, ref_handle) — iterator object terkait.
#[no_mangle]
pub unsafe extern "C" fn vhpi_iterate(kind: i32, ref_handle: VhpiHandle) -> VhpiHandle {
    iterator::vhpi_iterate(kind, ref_handle)
}

/// vhpi_scan(iterator) — object berikutnya (NULL saat habis).
#[no_mangle]
pub unsafe extern "C" fn vhpi_scan(iterator_handle: VhpiHandle) -> VhpiHandle {
    iterator::vhpi_scan(iterator_handle)
}

/// vhpi_get(property, handle) — properti integer.
#[no_mangle]
pub unsafe extern "C" fn vhpi_get(property: i32, handle: VhpiHandle) -> i32 {
    object::vhpi_get(property, handle)
}

/// vhpi_get_str(property, handle) — properti string.
#[no_mangle]
pub unsafe extern "C" fn vhpi_get_str(
    property: i32,
    handle: VhpiHandle,
) -> *mut std::os::raw::c_char {
    object::vhpi_get_str(property, handle)
}

/// vhpi_get_value(handle, value_p) — baca nilai signal.
#[no_mangle]
pub unsafe extern "C" fn vhpi_get_value(handle: VhpiHandle, value_p: *mut t_vhpi_value) -> i32 {
    if value_p.is_null() {
        return -1;
    }
    let obj = match super::handle::lookup(handle) {
        Some(o) => o,
        None => return -1,
    };
    let format = (*value_p).format;
    match &obj.kind {
        super::handle::VhpiObjectKind::Signal(sig_id, _)
        | super::handle::VhpiObjectKind::Port(sig_id, _) => {
            super::object::with_vhpi_engine(|engine| {
                let val = engine.state.read_signal(*sig_id).clone();
                *value_p = value::logicvec_to_vhpi(&val, format);
                0
            })
            .unwrap_or(-1)
        }
        _ => -1,
    }
}

/// vhpi_put_value(handle, value_p, time_p, flags) — tulis nilai signal.
#[no_mangle]
pub unsafe extern "C" fn vhpi_put_value(
    handle: VhpiHandle,
    value_p: *mut t_vhpi_value,
    _time_p: *mut value::t_vhpi_time,
    _flags: i32,
) -> i32 {
    if value_p.is_null() {
        return -1;
    }
    let obj = match super::handle::lookup(handle) {
        Some(o) => o,
        None => return -1,
    };
    match &obj.kind {
        super::handle::VhpiObjectKind::Signal(sig_id, _)
        | super::handle::VhpiObjectKind::Port(sig_id, _) => {
            super::object::with_vhpi_engine(|engine| {
                let val = value::vhpi_to_logicvec(value_p);
                engine.state.write_signal(*sig_id, val);
                0
            })
            .unwrap_or(-1)
        }
        _ => -1,
    }
}

/// vhpi_release_handle(handle) — bebaskan object handle.
#[no_mangle]
pub unsafe extern "C" fn vhpi_release_handle(handle: VhpiHandle) -> i32 {
    super::handle::vhpi_release_handle(handle)
}

/// vhpi_register_cb(cb_data_p) — daftarkan callback VHPI.
#[no_mangle]
pub unsafe extern "C" fn vhpi_register_cb(cb_data_p: *mut t_vhpi_cb_data) -> VhpiHandle {
    if cb_data_p.is_null() {
        return VhpiHandle::NULL;
    }
    callback::vhpi_register_cb(&*cb_data_p)
}

/// vhpi_remove_cb(handle) — hapus callback.
#[no_mangle]
pub unsafe extern "C" fn vhpi_remove_cb(handle: VhpiHandle) -> i32 {
    callback::vhpi_remove_cb(handle)
}

/// vhpi_control(operation, ...) — kontrol simulasi (stop/finish/reset).
#[no_mangle]
pub unsafe extern "C" fn vhpi_control(operation: i32, _user_data: *mut std::ffi::c_void) -> i32 {
    match operation {
        vhpiFinish => {
            super::object::with_vhpi_engine(|engine| {
                engine.running = false;
            });
            0
        }
        vhpiStop | vhpiReset | vhpiSetInteractiveScope => 0,
        _ => -1,
    }
}

/// vhpi_is_defined(kind) — dukungan object kind.
#[no_mangle]
pub unsafe extern "C" fn vhpi_is_defined(kind: i32) -> i32 {
    object::vhpi_is_defined(kind)
}

/// Hook start-of-simulation (dipanggil engine run).
pub fn dispatch_start_of_simulation() {
    callback::dispatch_start_of_simulation();
}

/// Hook end-of-simulation (dipanggil engine cleanup).
pub fn dispatch_end_of_simulation() {
    callback::dispatch_end_of_simulation();
}

/// Hook time step (dipanggil scheduler tiap time advance).
pub fn dispatch_time_step() {
    callback::dispatch_time_step();
}

/// Hook synch (ReadWrite/ReadOnly).
pub fn dispatch_synch() {
    callback::dispatch_synch();
}
