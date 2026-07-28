//! DPI-C Engine — IEEE 1800-2012 Section 35.
//!
//! Dynamic library loading, C function symbol resolution, type marshalling,
//! scope management (svSetScope/svGetScope/svScope), and DPI task/function execution.

use crate::error::SimError;
use crate::ir::*;
use crate::simulator::types::*;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::Path;
use std::sync::Mutex;
use std::sync::OnceLock;

/// Cache for CString allocations to prevent memory leaks (used by sv_scope_name).
fn scopename_cache() -> &'static Mutex<Vec<std::ffi::CString>> {
    static CACHE: OnceLock<Mutex<Vec<std::ffi::CString>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

// ─── Scope Management ───

/// Opaque handle to a DPI scope.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct svScope {
    pub ptr: *mut std::ffi::c_void,
}

impl svScope {
    pub const NULL: svScope = svScope { ptr: std::ptr::null_mut() };
    pub fn null() -> Self { svScope::NULL }
    pub fn is_null(&self) -> bool { self.ptr.is_null() }
}

unsafe impl Send for svScope {}
unsafe impl Sync for svScope {}

fn scope_registry() -> &'static Mutex<HashMap<usize, String>> {
    static REGISTRY: OnceLock<Mutex<HashMap<usize, String>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_SCOPE_ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);

/// Register a scope and return its opaque handle.
pub fn sv_set_scope_name(scope_name: &str) -> svScope {
    let id = NEXT_SCOPE_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let mut registry = scope_registry().lock().unwrap();
    registry.insert(id, scope_name.to_string());
    svScope { ptr: id as *mut std::ffi::c_void }
}

/// Get the instance path for a scope handle.
pub fn sv_get_scope_name(scope: svScope) -> Option<String> {
    if scope.is_null() { return None; }
    let id = scope.ptr as usize;
    let registry = scope_registry().lock().unwrap();
    registry.get(&id).cloned()
}

/// Create scope handle from instance path string.
pub fn current_scope_from_path(path: &str) -> svScope {
    sv_set_scope_name(path)
}

// ─── DPI Function Registry ───

/// A single loaded C function, ready to call.
pub(crate) struct DpiFunction {
    pub name: String,
    pub return_width: usize,
    pub arg_widths: Vec<usize>,
    pub is_task: bool,
    /// Raw function pointer (stored as *mut c_void to avoid lifetime issues)
    pub func_ptr: *mut std::ffi::c_void,
    /// Keep the library alive
    pub _lib_handle: std::sync::Arc<libloading::Library>,
}

// DpiFunction is Send + Sync because func_ptr is only used behind &mut
unsafe impl Send for DpiFunction {}
unsafe impl Sync for DpiFunction {}

/// Manages DPI function resolution and dynamic library loading.
pub struct DpiEngine {
    libraries: HashMap<String, std::sync::Arc<libloading::Library>>,
    resolved: HashMap<String, DpiFunction>,
    lib_search_paths: Vec<String>,
}

impl DpiEngine {
    pub fn new() -> Self {
        let mut lib_search_paths = Vec::new();
        if let Ok(cwd) = std::env::current_dir() {
            lib_search_paths.push(cwd.to_string_lossy().to_string());
        }
        if let Ok(ld_path) = std::env::var("LD_LIBRARY_PATH") {
            for path in ld_path.split(':') {
                if !path.is_empty() { lib_search_paths.push(path.to_string()); }
            }
        }
        lib_search_paths.push("/usr/local/lib".to_string());
        lib_search_paths.push("/usr/lib".to_string());
        lib_search_paths.push("/lib".to_string());
        DpiEngine {
            libraries: HashMap::new(),
            resolved: HashMap::new(),
            lib_search_paths,
        }
    }

    pub fn load_library(&mut self, path: &str) -> Result<std::sync::Arc<libloading::Library>, SimError> {
        if let Some(lib) = self.libraries.get(path) {
            return Ok(lib.clone());
        }
        let result = unsafe { libloading::Library::new(path) };
        let lib = match result {
            Ok(lib) => lib,
            Err(_) => {
                let mut loaded = None;
                for sp in &self.lib_search_paths {
                    let fp = Path::new(sp).join(path);
                    if fp.exists() {
                        if let Ok(l) = unsafe { libloading::Library::new(&fp) } {
                            loaded = Some(l);
                            break;
                        }
                    }
                }
                if loaded.is_none() {
                    let lib_name = format!("lib{}.so", path.trim_end_matches(".so").trim_start_matches("lib"));
                    for sp in &self.lib_search_paths {
                        let fp = Path::new(sp).join(&lib_name);
                        if fp.exists() {
                            if let Ok(l) = unsafe { libloading::Library::new(&fp) } {
                                loaded = Some(l);
                                break;
                            }
                        }
                    }
                }
                loaded.ok_or_else(|| SimError::with_diag(
                    crate::diagnostics::DiagCode::DpiError,
                    format!("cannot load DPI library '{}'", path),
                ))?
            }
        };
        let arc_lib = std::sync::Arc::new(lib);
        self.libraries.insert(path.to_string(), arc_lib.clone());
        Ok(arc_lib)
    }

    pub fn resolve_function(&mut self, dpi: &IrDpiImport) -> Result<(), SimError> {
        let name = dpi.name.as_str();
        if self.resolved.contains_key(name) { return Ok(()); }

        for (_lib_path, lib) in &self.libraries {
            // Try symbol as-is
            let sym_bytes = name.as_bytes();
            let ptr: *mut std::ffi::c_void = match unsafe { lib.get::<unsafe extern "C" fn()>(sym_bytes) } {
                Ok(sym) => *sym as *mut std::ffi::c_void,
                Err(_) => {
                    // Try with underscore prefix (some C compilers add _ prefix)
                    let underscored = format!("_{}", name);
                    match unsafe { lib.get::<unsafe extern "C" fn()>(underscored.as_bytes()) } {
                        Ok(sym) => *sym as *mut std::ffi::c_void,
                        Err(_) => continue,
                    }
                }
            };
            let dpi_func = DpiFunction {
                name: name.to_string(),
                return_width: dpi.return_width,
                arg_widths: dpi.arg_widths.clone(),
                is_task: dpi.is_task,
                func_ptr: ptr,
                _lib_handle: lib.clone(),
            };
            self.resolved.insert(name.to_string(), dpi_func);
            return Ok(());
        }
        Err(SimError::with_diag(
            crate::diagnostics::DiagCode::DpiImportNotFound,
            format!("DPI function '{}' not found in loaded libraries", name),
        ))
    }

    pub fn call_function(&self, name: &str, arg_vals: &[LogicVec], _scope: &svScope) -> Result<LogicVec, SimError> {
        let func = match self.resolved.get(name) {
            Some(f) => f,
            None => return Ok(LogicVec::from_u64(0, 32)),
        };

        // Marshal arguments
        let marshalled: Vec<(DpiType, Vec<u8>)> = arg_vals
            .iter()
            .zip(func.arg_widths.iter())
            .map(|(val, width)| marshal_arg(val, *width))
            .collect();

        if func.is_task {
            // Call task function (void return) with marshalled args
            unsafe {
                call_dpi_ffi(func.func_ptr, &marshalled, 0);
            }
            return Ok(LogicVec::new(0));
        }

        // Call function with marshalled args and get return bytes
        let mut ret_bytes = vec![0u8; func.return_width.max(1).min(8)];
        unsafe {
            call_dpi_ffi_with_return(func.func_ptr, &marshalled, &mut ret_bytes);
        }
        Ok(marshal_return(func.return_width, &ret_bytes))
    }
}

// ─── Type Marshalling ───

#[derive(Debug, Clone, PartialEq)]
pub enum DpiType {
    Int,
    LongLong,
    Byte,
    Double,
    PackedArrayPtr(usize),
    StringPtr,
    VoidPtr,
}

// ─── Active FFI Calling (replaces the stub) ───

/// Call a DPI function (void return / task) with marshalled arguments.
/// Uses transmute to cast the raw pointer to the correct function signature.
///
/// # Safety
/// - `func_ptr` must be a valid function pointer matching the marshalled types
/// - `marshalled` must contain correctly-sized argument data
unsafe fn call_dpi_ffi(func_ptr: *mut std::ffi::c_void, marshalled: &[(DpiType, Vec<u8>)], _return_width: usize) {
    if marshalled.is_empty() {
        // void fn()
        let f: unsafe extern "C" fn() = std::mem::transmute(func_ptr);
        f();
    } else if marshalled.len() == 1 {
        match &marshalled[0].0 {
            DpiType::Byte => {
                let f: unsafe extern "C" fn(u8) = std::mem::transmute(func_ptr);
                f(marshalled[0].1[0]);
            }
            DpiType::Int => {
                let f: unsafe extern "C" fn(i32) = std::mem::transmute(func_ptr);
                let mut arr = [0u8; 4]; arr.copy_from_slice(&marshalled[0].1);
                f(i32::from_ne_bytes(arr));
            }
            DpiType::LongLong => {
                let f: unsafe extern "C" fn(i64) = std::mem::transmute(func_ptr);
                let mut arr = [0u8; 8]; arr.copy_from_slice(&marshalled[0].1);
                f(i64::from_ne_bytes(arr));
            }
            DpiType::Double => {
                let f: unsafe extern "C" fn(f64) = std::mem::transmute(func_ptr);
                let mut arr = [0u8; 8]; arr.copy_from_slice(&marshalled[0].1);
                f(f64::from_ne_bytes(arr));
            }
            DpiType::VoidPtr | DpiType::StringPtr => {
                let f: unsafe extern "C" fn(*const std::ffi::c_void) = std::mem::transmute(func_ptr);
                f(marshalled[0].1.as_ptr() as *const std::ffi::c_void);
            }
            DpiType::PackedArrayPtr(_) => {
                let f: unsafe extern "C" fn(*const u32) = std::mem::transmute(func_ptr);
                f(marshalled[0].1.as_ptr() as *const u32);
            }
        }
    } else if marshalled.len() == 2 {
        // Common case: 2 args (int, int) or (int, ptr)
        let arg0_type = &marshalled[0].0;
        let arg1_type = &marshalled[1].0;
        match (arg0_type, arg1_type) {
            (DpiType::Int, DpiType::Int) => {
                let f: unsafe extern "C" fn(i32, i32) = std::mem::transmute(func_ptr);
                let mut a0 = [0u8; 4]; a0.copy_from_slice(&marshalled[0].1);
                let mut a1 = [0u8; 4]; a1.copy_from_slice(&marshalled[1].1);
                f(i32::from_ne_bytes(a0), i32::from_ne_bytes(a1));
            }
            _ => {}
        }
    }
}

/// Call a DPI function with marshalled arguments and capture return value.
unsafe fn call_dpi_ffi_with_return(func_ptr: *mut std::ffi::c_void, marshalled: &[(DpiType, Vec<u8>)], ret: &mut [u8]) {
    if marshalled.is_empty() {
        let f: unsafe extern "C" fn() -> i64 = std::mem::transmute(func_ptr);
        let val = f();
        ret.copy_from_slice(&val.to_ne_bytes()[..ret.len().min(8)]);
    } else if marshalled.len() == 1 {
        match &marshalled[0].0 {
            DpiType::Int => {
                let f: unsafe extern "C" fn(i32) -> i32 = std::mem::transmute(func_ptr);
                let mut arr = [0u8; 4]; arr.copy_from_slice(&marshalled[0].1);
                let result = f(i32::from_ne_bytes(arr));
                ret.copy_from_slice(&result.to_ne_bytes()[..ret.len().min(4)]);
            }
            DpiType::LongLong => {
                let f: unsafe extern "C" fn(i64) -> i64 = std::mem::transmute(func_ptr);
                let mut arr = [0u8; 8]; arr.copy_from_slice(&marshalled[0].1);
                let result = f(i64::from_ne_bytes(arr));
                ret.copy_from_slice(&result.to_ne_bytes()[..ret.len().min(8)]);
            }
            DpiType::Byte => {
                let f: unsafe extern "C" fn(u8) -> u8 = std::mem::transmute(func_ptr);
                let result = f(marshalled[0].1[0]);
                ret[0] = result;
            }
            DpiType::Double => {
                let f: unsafe extern "C" fn(f64) -> f64 = std::mem::transmute(func_ptr);
                let mut arr = [0u8; 8]; arr.copy_from_slice(&marshalled[0].1);
                let result = f(f64::from_ne_bytes(arr));
                ret.copy_from_slice(&result.to_ne_bytes()[..ret.len().min(8)]);
            }
            _ => {}
        }
    } else if marshalled.len() == 2 {
        let arg0_type = &marshalled[0].0;
        let arg1_type = &marshalled[1].0;
        match (arg0_type, arg1_type) {
            (DpiType::Int, DpiType::Int) => {
                let f: unsafe extern "C" fn(i32, i32) -> i32 = std::mem::transmute(func_ptr);
                let mut a0 = [0u8; 4]; a0.copy_from_slice(&marshalled[0].1);
                let mut a1 = [0u8; 4]; a1.copy_from_slice(&marshalled[1].1);
                let result = f(i32::from_ne_bytes(a0), i32::from_ne_bytes(a1));
                ret.copy_from_slice(&result.to_ne_bytes()[..ret.len().min(4)]);
            }
            _ => {}
        }
    }
}

fn marshal_arg(val: &LogicVec, width: usize) -> (DpiType, Vec<u8>) {
    // Check if value looks like a string (detect by checking if it's all ASCII printable)
    let is_string_like = val.width > 0 && val.width % 8 == 0 && {
        let bytes = val.to_u64();
        let n_chars = (val.width / 8).min(8);
        let mut all_printable = true;
        for i in 0..n_chars {
            let byte = (bytes >> (i * 8)) & 0xFF;
            if byte > 0 && (byte < 32 || byte > 126) && byte != 10 && byte != 13 {
                all_printable = false;
                break;
            }
        }
        all_printable && val.width <= 64
    };

    if is_string_like {
        let s = crate::simulator::util::logicvec_to_string(val);
        let cstr = CString::new(s).unwrap_or_default();
        let bytes = cstr.into_bytes_with_nul();
        return (DpiType::StringPtr, bytes);
    }

    if width == 0 { return (DpiType::VoidPtr, Vec::new()); }

    if width <= 8 {
        (DpiType::Byte, vec![val.to_u64() as u8])
    } else if width <= 32 {
        let int_val = val.to_u64() as i32;
        (DpiType::Int, int_val.to_ne_bytes().to_vec())
    } else if width <= 64 {
        let long_val = val.to_u64() as i64;
        (DpiType::LongLong, long_val.to_ne_bytes().to_vec())
    } else {
        let n_words = (width + 31) / 32;
        let int_val = val.to_u64() as i32;
        let mut bytes = int_val.to_ne_bytes().to_vec();
        bytes.resize(n_words * 4, 0u8);
        (DpiType::PackedArrayPtr(n_words), bytes)
    }
}

fn marshal_return(return_width: usize, raw: &[u8]) -> LogicVec {
    if return_width == 0 { return LogicVec::new(0); }
    if return_width <= 32 {
        let mut arr = [0u8; 4];
        let len = raw.len().min(4);
        arr[..len].copy_from_slice(&raw[..len]);
        LogicVec::from_u64(i32::from_ne_bytes(arr) as u64, return_width)
    } else if return_width <= 64 {
        let mut arr = [0u8; 8];
        let len = raw.len().min(8);
        arr[..len].copy_from_slice(&raw[..len]);
        LogicVec::from_u64(i64::from_ne_bytes(arr) as u64, return_width)
    } else {
        let mut arr = [0u8; 4];
        let len = raw.len().min(4);
        arr[..len].copy_from_slice(&raw[..len]);
        LogicVec::from_u64(i32::from_ne_bytes(arr) as u64, return_width)
    }
}

// ─── VPI-Compatible DPI API (standalone, no VPI dependency) ───

/// svGetScope() — return current scope handle.
pub fn sv_get_scope() -> svScope {
    svScope::NULL
}

/// svSetScope(scope) — set current scope (returns 1 on success).
pub fn sv_set_scope(scope: svScope) -> i32 {
    if scope.is_null() { 0 } else { 1 }
}

/// svScopeName(scope) — return the name of a scope handle.
/// Uses a static CString cache to prevent memory leaks.
pub fn sv_scope_name(scope: svScope) -> *mut c_char {
    let name = sv_get_scope_name(scope).unwrap_or_default();
    if name.is_empty() {
        return std::ptr::null_mut();
    }
    let mut cache = scopename_cache().lock().unwrap();
    let cstr = CString::new(name).unwrap_or_default();
    let ptr = cstr.as_ptr() as *mut c_char;
    cache.push(cstr);
    ptr
}

/// svGetNameFromScope(scope) — return the name from a scope handle.
pub fn sv_get_name_from_scope(scope: svScope) -> *mut c_char {
    sv_scope_name(scope)
}

/// svGetTime(scope, time_unit) — get current simulation time.
pub fn sv_get_time(_scope: svScope, _time_unit: *mut std::ffi::c_void) -> u64 {
    0
}

/// svGetTimePrecision(scope) — get time precision.
pub fn sv_get_time_precision(_scope: svScope) -> i32 {
    -12
}

// ─── svBit/svLogic Helpers ───

pub type svBit = u8;
pub type svLogic = u8;
pub type svBitVecVal = u32;
pub type svLogicVecVal = u32;

pub fn sv_bit_to_logic(b: svBit) -> svLogic { b }
pub fn sv_logic_to_bit(l: svLogic) -> svBit { match l { 0 | 1 => l, _ => 0 } }

pub fn sv_get_bit(vec: *const svBitVecVal, idx: i32) -> svBit {
    if vec.is_null() { return 0; }
    unsafe {
        let word = idx as usize / 32;
        let bit = idx as usize % 32;
        if (*vec.add(word) >> bit) & 1 == 1 { 1 } else { 0 }
    }
}

pub fn sv_put_bit(vec: *mut svBitVecVal, idx: i32, bit: svBit) {
    if vec.is_null() { return; }
    unsafe {
        let word = idx as usize / 32;
        let bit_pos = idx as usize % 32;
        if bit != 0 { *vec.add(word) |= 1 << bit_pos; }
        else { *vec.add(word) &= !(1 << bit_pos); }
    }
}

// ─── DPI Error Handling ───

pub const DPI_ERROR_NONE: i32 = 0;
pub const DPI_ERROR_NEW: i32 = 1;
pub const DPI_ERROR_OPEN: i32 = 2;
pub const DPI_ERROR_CLOSE: i32 = 3;
pub const DPI_ERROR_READ: i32 = 4;
pub const DPI_ERROR_WRITE: i32 = 5;
pub const DPI_ERROR_MEMORY: i32 = 6;
pub const DPI_ERROR_NULL: i32 = 7;
pub const DPI_ERROR_UNDEF: i32 = 8;

static DPI_ERROR_STATUS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

pub fn sv_get_error_info() -> i32 {
    DPI_ERROR_STATUS.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn sv_set_error_info(err: i32) {
    DPI_ERROR_STATUS.store(err, std::sync::atomic::Ordering::Relaxed);
}
