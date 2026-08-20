//! VPI C FFI — extern "C" exports for all VPI functions.
//!
//! These functions are callable from C code linked against Maria.
//! They match the IEEE 1800-2012 VPI standard API exactly.

use super::types::*;
use super::handle;
use super::value;
use super::control;
use super::callback;
use super::systf;
use std::ffi::CStr;
use std::os::raw::c_char;

/// vpi_handle_by_name — find an object by name.
///
/// # Safety
///
/// `name` harus menunjuk ke C string null-terminated yang valid (atau null).
#[no_mangle]
pub unsafe extern "C" fn vpi_handle_by_name(name: *const c_char, scope: vpiHandle) -> vpiHandle {
    let name_str = super::types::cstr_to_str(name);
    handle::vpi_handle_by_name(name_str, scope)
}

/// vpi_handle — get related object.
///
/// # Safety
///
/// `ref_handle` harus merupakan handle VPI yang valid dari pemanggilan sebelumnya.
#[no_mangle]
pub unsafe extern "C" fn vpi_handle(vpi_type: i32, ref_handle: vpiHandle) -> vpiHandle {
    handle::vpi_handle(vpi_type, ref_handle)
}

/// vpi_iterate — create iterator.
///
/// # Safety
///
/// `ref_handle` harus merupakan handle VPI yang valid dari pemanggilan sebelumnya.
#[no_mangle]
pub unsafe extern "C" fn vpi_iterate(vpi_type: i32, ref_handle: vpiHandle) -> vpiHandle {
    handle::vpi_iterate(vpi_type, ref_handle)
}

/// vpi_scan — advance iterator.
///
/// # Safety
///
/// `iter_handle` harus merupakan handle iterator yang valid dari `vpi_iterate`.
#[no_mangle]
pub unsafe extern "C" fn vpi_scan(iter_handle: vpiHandle) -> vpiHandle {
    handle::vpi_scan(iter_handle)
}

/// vpi_get — get integer property.
///
/// # Safety
///
/// `handle` harus merupakan handle VPI yang valid dari pemanggilan sebelumnya.
#[no_mangle]
pub unsafe extern "C" fn vpi_get(property: i32, handle: vpiHandle) -> i32 {
    handle::vpi_get(property, handle)
}

/// vpi_get_str — get string property.
///
/// # Safety
///
/// `handle` harus merupakan handle VPI yang valid dari pemanggilan sebelumnya.
#[no_mangle]
pub unsafe extern "C" fn vpi_get_str(property: i32, handle: vpiHandle) -> *mut c_char {
    handle::vpi_get_str(property, handle)
}

/// vpi_get_value — get signal value.
///
/// # Safety
///
/// `expr` harus merupakan handle VPI yang valid; `value_p` harus menunjuk ke
/// `t_vpi_value` yang valid dan dapat ditulis, atau null (mengembalikan 0).
#[no_mangle]
pub unsafe extern "C" fn vpi_get_value(expr: vpiHandle, value_p: *mut t_vpi_value) -> i32 {
    if value_p.is_null() {
        return 0;
    }
    value::vpi_get_value(expr, &mut *value_p)
}

/// vpi_put_value — set signal value.
///
/// # Safety
///
/// `obj` harus merupakan handle VPI yang valid; `value_p` harus menunjuk ke
/// `t_vpi_value` yang valid, atau null (mengembalikan 0).
#[no_mangle]
pub unsafe extern "C" fn vpi_put_value(
    obj: vpiHandle,
    value_p: *const t_vpi_value,
    time_p: *mut t_vpi_time,
    flags: i32,
) -> i32 {
    if value_p.is_null() {
        return 0;
    }
    value::vpi_put_value(obj, &*value_p, time_p, flags)
}

/// vpi_register_cb — register callback.
///
/// # Safety
///
/// `cb_data_p` harus menunjuk ke `t_cb_data` yang valid dan diinisialisasi,
/// atau null (mengembalikan vpiHandle::NULL).
#[no_mangle]
pub unsafe extern "C" fn vpi_register_cb(cb_data_p: *mut t_cb_data) -> vpiHandle {
    if cb_data_p.is_null() {
        return vpiHandle::NULL;
    }
    callback::vpi_register_cb(&*cb_data_p)
}

/// vpi_remove_cb — remove callback.
///
/// # Safety
///
/// `cb_handle` harus merupakan handle callback yang valid dari `vpi_register_cb`.
#[no_mangle]
pub unsafe extern "C" fn vpi_remove_cb(cb_handle: vpiHandle) -> i32 {
    callback::vpi_remove_cb(cb_handle)
}

/// vpi_register_systf — register system task/function.
///
/// # Safety
///
/// `systf_data_p` harus menunjuk ke `s_vpi_systf_data` yang valid dan
/// diinisialisasi, atau null (mengembalikan vpiHandle::NULL).
#[no_mangle]
pub unsafe extern "C" fn vpi_register_systf(systf_data_p: *mut s_vpi_systf_data) -> vpiHandle {
    if systf_data_p.is_null() {
        return vpiHandle::NULL;
    }
    systf::vpi_register_systf(&*systf_data_p)
}

/// vpi_remove_systf — remove system task/function.
///
/// # Safety
///
/// `systf_handle` harus merupakan handle system task/function yang valid.
#[no_mangle]
pub unsafe extern "C" fn vpi_remove_systf(systf_handle: vpiHandle) -> i32 {
    systf::vpi_remove_systf(systf_handle)
}

/// vpi_control — simulation control.
///
/// # Safety
///
/// Fungsi ini aman untuk argumen integer biasa; hanya perlu `unsafe` untuk
/// menandai operasi FFI eksternal.
#[no_mangle]
pub unsafe extern "C" fn vpi_control(operation: i32, arg1: i32, arg2: i32) -> i32 {
    control::vpi_control(operation, arg1, arg2)
}

/// vpi_get_time — get current simulation time.
///
/// # Safety
///
/// `obj` harus merupakan handle VPI yang valid; `time_p` harus menunjuk ke
/// `t_vpi_time` yang valid dan dapat ditulis, atau null (mengembalikan 0).
#[no_mangle]
pub unsafe extern "C" fn vpi_get_time(obj: vpiHandle, time_p: *mut t_vpi_time) -> i32 {
    if time_p.is_null() {
        return 0;
    }
    control::vpi_get_time(obj, &mut *time_p)
}

/// vpi_free_object — release object.
///
/// # Safety
///
/// `obj` harus merupakan handle yang valid dari pemanggilan sebelumnya;
/// handle tidak boleh dipakai lagi setelah dibebaskan.
#[no_mangle]
pub unsafe extern "C" fn vpi_free_object(obj: vpiHandle) -> i32 {
    handle::vpi_free_object(obj)
}

/// vpi_chk_error — check for error.
///
/// # Safety
///
/// Fungsi ini tidak mengambil argumen pointer; `unsafe` diperlukan hanya untuk
/// konsistensi dengan API VPI extern.
#[no_mangle]
pub unsafe extern "C" fn vpi_chk_error() -> i32 {
    handle::vpi_chk_error()
}

// ─── Helper Functions ───

/// Internal: convert C string pointer to Rust string slice.
///
/// # Safety
///
/// `ptr` harus menunjuk ke C string null-terminated yang valid, atau null
/// (yang mengembalikan "").
#[allow(dead_code)]
pub(crate) unsafe fn cstr_to_str<'a>(ptr: *const c_char) -> &'a str {
    if ptr.is_null() {
        return "";
    }
    CStr::from_ptr(ptr).to_str().unwrap_or("")
}
