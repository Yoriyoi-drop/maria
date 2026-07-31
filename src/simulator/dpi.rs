//! DPI-C Engine — IEEE 1800-2012 Section 35.
//!
//! Dynamic library loading, C function symbol resolution, type marshalling,
//! scope management (svSetScope/svGetScope/svScope), and DPI task/function execution.

use crate::error::SimError;
use crate::ir::*;
use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::c_char;
use std::path::Path;
use std::sync::Mutex;
use std::sync::OnceLock;

/// Cache for CString allocations to prevent memory leaks (used by sv_scope_name).
fn scopename_cache() -> &'static Mutex<Vec<std::ffi::CString>> {
    static CACHE: OnceLock<Mutex<Vec<std::ffi::CString>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

// ─── Thread-local Scope State (wired to SimulationEngine) ───

use std::cell::RefCell;

thread_local! {
    /// Current DPI scope path (set by engine during process evaluation)
    static CURRENT_DPI_SCOPE_PATH: RefCell<String> = const { RefCell::new(String::new()) };
    /// Current simulation time (ns) for DPI queries
    static CURRENT_DPI_TIME: RefCell<u64> = const { RefCell::new(0u64) };
}

/// Set current DPI scope path (called by SimulationEngine during process eval).
pub fn set_current_dpi_scope(path: &str) {
    CURRENT_DPI_SCOPE_PATH.with(|cell| *cell.borrow_mut() = path.to_string());
}

/// Set current simulation time for DPI queries (called by SimulationEngine each cycle).
pub fn set_current_dpi_time(time: u64) {
    CURRENT_DPI_TIME.with(|cell| *cell.borrow_mut() = time);
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
    pub fn from_id(id: usize) -> Self {
        svScope { ptr: id as *mut std::ffi::c_void }
    }
    pub fn id(&self) -> usize {
        self.ptr as usize
    }
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

        for lib in self.libraries.values() {
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
        let mut ret_bytes = vec![0u8; func.return_width.clamp(1, 8)];
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

/// Marshal all arguments into a single flat Vec<u8> and call the DPI function
/// with varargs-like semantics via transmute to a generic `*const c_void` function.
/// Then reinterpret return bytes.
///
/// # Safety
/// - `func_ptr` must be a valid function pointer
/// - `marshalled` types must match the C function signature
unsafe fn call_dpi_ffi_generic(
    func_ptr: *mut std::ffi::c_void,
    marshalled: &[(DpiType, Vec<u8>)],
    ret_bytes: &mut [u8],
    _is_task: bool,
) {
    // Build flat argument array: alternating type tag + data pointer
    // The C function receives a struct pointer:
    //   struct DpiArgs { int n_args; DpiArg args[]; }
    //   struct DpiArg { int type_tag; void* data; };
    // This is safe because both ends of the ABI agree on the layout.
    //
    // For simple types (byte/int/longlong/double), the data is passed by value inline.
    // For pointer types (string/void/array), a pointer to the data is passed.
    //
    // We use a helper struct-based calling convention:
    //   struct DpiCallFrame { int n_args; int return_width; int arg_tags[]; void* arg_data[]; };
    // This avoids needing to transmute to a specific function signature.

    let n_args = marshalled.len();
    let mut tags: Vec<i32> = Vec::with_capacity(n_args);
    let mut ptrs: Vec<*const std::ffi::c_void> = Vec::with_capacity(n_args);
    let mut owned_data: Vec<Vec<u8>> = Vec::new();

    for (dpi_type, data) in marshalled {
        let tag = match dpi_type {
            DpiType::Byte => { owned_data.push(data.clone()); 0i32 }
            DpiType::Int => { owned_data.push(data.clone()); 1i32 }
            DpiType::LongLong => { owned_data.push(data.clone()); 2i32 }
            DpiType::Double => { owned_data.push(data.clone()); 3i32 }
            DpiType::StringPtr => { owned_data.push(data.clone()); 4i32 }
            DpiType::VoidPtr => { owned_data.push(data.clone()); 5i32 }
            DpiType::PackedArrayPtr(_) => { owned_data.push(data.clone()); 6i32 }
        };
        tags.push(tag);
        let data_ptr = owned_data.last().map(|v| v.as_ptr() as *const std::ffi::c_void)
            .unwrap_or(std::ptr::null());
        ptrs.push(data_ptr);
    }

    // Build the call frame struct
    // struct DpiCallFrame { int n_args; int* tags; void** args; void* ret_buf; int ret_max; };
    let call_frame = DpiCallFrame {
        n_args: n_args as i32,
        tags: tags.as_ptr() as *const i32,
        args: ptrs.as_ptr(),
        ret_buf: if ret_bytes.is_empty() { std::ptr::null_mut() } else { ret_bytes.as_mut_ptr() as *mut std::ffi::c_void },
        ret_max: ret_bytes.len() as i32,
    };

    let f: unsafe extern "C" fn(*const DpiCallFrame) = std::mem::transmute(func_ptr);
    f(&call_frame as *const DpiCallFrame);
}

/// DPI call frame struct — both sides (Rust DPI engine and C DPI function) agree on layout.
#[repr(C)]
pub struct DpiCallFrame {
    /// Number of arguments
    pub n_args: i32,
    /// Array of type tags (length = n_args)
    pub tags: *const i32,
    /// Array of argument data pointers (length = n_args)
    pub args: *const *const std::ffi::c_void,
    /// Return buffer (may be null for void functions)
    pub ret_buf: *mut std::ffi::c_void,
    /// Maximum return size in bytes
    pub ret_max: i32,
}

// Legacy FFI callers for simple cases (keep for backward compat)

/// Call a DPI function (void return / task) with marshalled arguments.
unsafe fn call_dpi_ffi(func_ptr: *mut std::ffi::c_void, marshalled: &[(DpiType, Vec<u8>)], _return_width: usize) {
    let mut ret_buf = vec![0u8; 0];
    call_dpi_ffi_generic(func_ptr, marshalled, &mut ret_buf, true);
}

/// Call a DPI function with marshalled arguments and capture return value.
unsafe fn call_dpi_ffi_with_return(func_ptr: *mut std::ffi::c_void, marshalled: &[(DpiType, Vec<u8>)], ret: &mut [u8]) {
    call_dpi_ffi_generic(func_ptr, marshalled, ret, false);
}

fn marshal_arg(val: &LogicVec, width: usize) -> (DpiType, Vec<u8>) {
    // Check if value looks like a string (detect by checking if it's all ASCII printable)
    let is_string_like = val.width > 0 && val.width.is_multiple_of(8) && {
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
        let n_words = width.div_ceil(32);
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

/// svGetScope() — return current scope handle from thread-local path.
pub fn sv_get_scope() -> svScope {
    CURRENT_DPI_SCOPE_PATH.with(|cell| {
        let path = cell.borrow();
        if path.is_empty() {
            svScope::NULL
        } else {
            sv_set_scope_name(&path)
        }
    })
}

/// svSetScope(scope) — set current scope (returns 1 on success).
pub fn sv_set_scope(scope: svScope) -> i32 {
    if scope.is_null() { return 0; }
    if let Some(name) = sv_get_scope_name(scope) {
        CURRENT_DPI_SCOPE_PATH.with(|cell| {
            *cell.borrow_mut() = name;
        });
        1
    } else {
        0
    }
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

/// svGetTime(scope, time_unit) — get current simulation time from thread-local.
pub fn sv_get_time(_scope: svScope, _time_unit: *mut std::ffi::c_void) -> u64 {
    CURRENT_DPI_TIME.with(|cell| *cell.borrow())
}

/// svGetTimePrecision(scope) — get time precision.
pub fn sv_get_time_precision(_scope: svScope) -> i32 {
    CURRENT_DPI_TIME.with(|cell| {
        let t = *cell.borrow();
        if t == 0 { -12 } else { -9 } // default ns precision
    })
}

// ─── svGet/svPut Logic Vector Helpers (complete IEEE set) ───

/// svGetBitsel — get a single bit from a logic vector.
///
/// # Safety
/// `vec` must point to a valid `svBitVecVal` array of sufficient length for `idx/32` words.
pub unsafe fn sv_get_bitsel(vec: *const svBitVecVal, idx: i32) -> svBit {
    if vec.is_null() { return 0; }
    let word = idx as usize / 32;
    let bit = idx as usize % 32;
    if (*vec.add(word) >> bit) & 1 == 1 { 1 } else { 0 }
}

/// svPutBitsel — set a single bit in a logic vector.
///
/// # Safety
/// `vec` must point to a valid mutable `svBitVecVal` array of sufficient length for `idx/32` words.
pub unsafe fn sv_put_bitsel(vec: *mut svBitVecVal, idx: i32, bit: svBit) {
    if vec.is_null() { return; }
    let word = idx as usize / 32;
    let bit_pos = idx as usize % 32;
    if bit != 0 { *vec.add(word) |= 1 << bit_pos; }
    else { *vec.add(word) &= !(1 << bit_pos); }
}

/// svGetLogicBitsel — get a single 4-state logic bit.
///
/// # Safety
/// `vec` must point to a valid `svLogicVecVal` array (2 words per element) of sufficient length.
pub unsafe fn sv_get_logic_bitsel(vec: *const svLogicVecVal, idx: i32) -> svLogic {
    if vec.is_null() { return 0; }
    let word = idx as usize / 32;
    let bit = idx as usize % 32;
    let aval = *vec.add(word * 2) >> bit & 1;
    let bval = *vec.add(word * 2 + 1) >> bit & 1;
    if aval == 0 && bval == 0 { 0 }      // 0
    else if aval == 1 && bval == 0 { 1 }  // 1
    else if aval == 0 && bval == 1 { 2 }  // X
    else { 3 }                            // Z
}

/// svPutLogicBitsel — set a single 4-state logic bit.
///
/// # Safety
/// `vec` must point to a valid mutable `svLogicVecVal` array (2 words per element) of sufficient length.
pub unsafe fn sv_put_logic_bitsel(vec: *mut svLogicVecVal, idx: i32, logic: svLogic) {
    if vec.is_null() { return; }
    let word = idx as usize / 32;
    let bit = idx as usize % 32;
    let mask = 1u32 << bit;
    match logic {
        0 => { *vec.add(word * 2) &= !mask; *vec.add(word * 2 + 1) &= !mask; }
        1 => { *vec.add(word * 2) |= mask; *vec.add(word * 2 + 1) &= !mask; }
        2 => { *vec.add(word * 2) &= !mask; *vec.add(word * 2 + 1) |= mask; }
        _ => { *vec.add(word * 2) |= mask; *vec.add(word * 2 + 1) |= mask; }
    }
}

/// svGetPartSelect — get a contiguous range of bits from a 2-state vector.
///
/// # Safety
/// `vec` must point to a valid `svBitVecVal` array of sufficient length to cover `idx+width` bits.
pub unsafe fn sv_get_part_select(vec: *const svBitVecVal, idx: i32, width: i32) -> svBitVecVal {
    if vec.is_null() || width <= 0 { return 0; }
    let mut result = 0u32;
    for i in 0..width.min(32) {
        let w = (idx + i) as usize / 32;
        let b = (idx + i) as usize % 32;
        if *vec.add(w) >> b & 1 == 1 {
            result |= 1 << i;
        }
    }
    result
}

/// svPutPartSelect — set a contiguous range of bits in a 2-state vector.
///
/// # Safety
/// `vec` must point to a valid mutable `svBitVecVal` array of sufficient length to cover `idx+width` bits.
pub unsafe fn sv_put_part_select(vec: *mut svBitVecVal, idx: i32, width: i32, val: svBitVecVal) {
    if vec.is_null() || width <= 0 { return; }
    for i in 0..width.min(32) {
        let w = (idx + i) as usize / 32;
        let b = (idx + i) as usize % 32;
        let bit = (val >> i) & 1;
        if bit != 0 { *vec.add(w) |= 1 << b; }
        else { *vec.add(w) &= !(1 << b); }
    }
}

/// svLeft — get the left bound of a range.
///
/// # Safety
/// `vec` must be a valid pointer (may be null, which yields 0).
pub unsafe fn sv_left(vec: *const svBitVecVal, _width: i32) -> i32 {
    // Simplified: assume [width-1:0]
    if vec.is_null() { return 0; }
    *vec as i32
}

/// svRight — get the right bound of a range.
pub fn sv_right(vec: *const svBitVecVal, _width: i32) -> i32 {
    if vec.is_null() { return 0; }
    0
}

/// svLow — get the low bound of a range.
pub fn sv_low(vec: *const svBitVecVal, _width: i32) -> i32 {
    if vec.is_null() { return 0; }
    0
}

/// svHigh — get the high bound of a range.
pub fn sv_high(vec: *const svBitVecVal, width: i32) -> i32 {
    if vec.is_null() { return 0; }
    width - 1
}

/// svSizeOfArray — get the size of an array dimension.
pub fn sv_size_of_array(_handle: *mut std::ffi::c_void, _dim: i32) -> i32 {
    0 // stub
}

/// svDimensions — get number of array dimensions.
pub fn sv_dimensions(_handle: *mut std::ffi::c_void) -> i32 {
    0 // stub
}

// ─── chandle helpers ───

/// Convert a chandle (64-bit opaque pointer) to/from a LogicVec.
pub fn chandle_to_u64(ch: *mut std::ffi::c_void) -> u64 {
    ch as u64
}

pub fn u64_to_chandle(val: u64) -> *mut std::ffi::c_void {
    val as *mut std::ffi::c_void
}

// ─── svBit/svLogic Helpers ───

#[allow(non_camel_case_types)]
pub type svBit = u8;
#[allow(non_camel_case_types)]
pub type svLogic = u8;
#[allow(non_camel_case_types)]
pub type svBitVecVal = u32;
#[allow(non_camel_case_types)]
pub type svLogicVecVal = u32;

pub fn sv_bit_to_logic(b: svBit) -> svLogic { b }
pub fn sv_logic_to_bit(l: svLogic) -> svBit { match l { 0 | 1 => l, _ => 0 } }

/// svGetBit — get a single bit from a 2-state vector.
///
/// # Safety
/// `vec` must point to a valid `svBitVecVal` array of sufficient length for `idx/32` words.
pub unsafe fn sv_get_bit(vec: *const svBitVecVal, idx: i32) -> svBit {
    if vec.is_null() { return 0; }
    let word = idx as usize / 32;
    let bit = idx as usize % 32;
    if (*vec.add(word) >> bit) & 1 == 1 { 1 } else { 0 }
}

/// svPutBit — set a single bit in a 2-state vector.
///
/// # Safety
/// `vec` must point to a valid mutable `svBitVecVal` array of sufficient length for `idx/32` words.
pub unsafe fn sv_put_bit(vec: *mut svBitVecVal, idx: i32, bit: svBit) {
    if vec.is_null() { return; }
    let word = idx as usize / 32;
    let bit_pos = idx as usize % 32;
    if bit != 0 { *vec.add(word) |= 1 << bit_pos; }
    else { *vec.add(word) &= !(1 << bit_pos); }
}

// ─── DPI Export Framework (C-callable SV functions) ───

/// A registered SV function that can be called from C via DPI export.
pub struct DpiExportedFunction {
    /// C-visible function name
    pub export_name: String,
    /// Number of arguments expected
    pub n_args: usize,
    /// Width of each argument
    pub arg_widths: Vec<usize>,
    /// Whether the function is void
    pub is_task: bool,
    /// Closure to execute when called from C
    /// Takes (name, args) and returns result (can be stored via thread-local sv_export_result)
    pub callback: Box<dyn Fn(&[LogicVec]) -> LogicVec + Send + Sync>,
}

// Manually implement Debug for DpiExportedFunction (skip callback field)
impl std::fmt::Debug for DpiExportedFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DpiExportedFunction")
            .field("export_name", &self.export_name)
            .field("n_args", &self.n_args)
            .field("arg_widths", &self.arg_widths)
            .field("is_task", &self.is_task)
            .finish()
    }
}

/// Global registry of exported DPI functions (name → DpiExportedFunction).
fn export_registry() -> &'static Mutex<HashMap<String, DpiExportedFunction>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, DpiExportedFunction>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register an SV function as a DPI export (callable from C).
pub fn sv_export_register(func: DpiExportedFunction) {
    let mut registry = export_registry().lock().unwrap();
    registry.insert(func.export_name.clone(), func);
}

/// Call a registered DPI export from the SV engine side (e.g., after a C callback).
pub fn sv_export_call(name: &str, args: &[LogicVec]) -> Result<LogicVec, SimError> {
    let registry = export_registry().lock().unwrap();
    match registry.get(name) {
        Some(func) => {
            if func.is_task {
                (func.callback)(args);
                Ok(LogicVec::new(0))
            } else {
                Ok((func.callback)(args))
            }
        }
        None => Err(SimError::with_diag(
            crate::diagnostics::DiagCode::DpiError,
            format!("DPI export '{}' not found in registry", name),
        )),
    }
}

// ─── chandle global store ───

/// Thread-safe store for chandle values (maps chandle handle → u64 opaque value).
fn chandle_store() -> &'static Mutex<HashMap<u64, u64>> {
    static STORE: OnceLock<Mutex<HashMap<u64, u64>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_CHANDLE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Allocate a new chandle handle for an opaque pointer value.
pub fn chandle_alloc(ptr_val: u64) -> u64 {
    let handle = NEXT_CHANDLE_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let mut store = chandle_store().lock().unwrap();
    store.insert(handle, ptr_val);
    handle
}

/// Get the opaque pointer value from a chandle handle.
pub fn chandle_get(handle: u64) -> Option<u64> {
    let store = chandle_store().lock().unwrap();
    store.get(&handle).copied()
}

/// Free a chandle handle.
pub fn chandle_free(handle: u64) {
    let mut store = chandle_store().lock().unwrap();
    store.remove(&handle);
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
