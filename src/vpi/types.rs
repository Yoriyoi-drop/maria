//! VPI Type Definitions — IEEE 1800-2012 Section 36.
//!
//! C-compatible types for VPI FFI. All types use C representation
//! for direct FFI with external C code.

use std::ffi::CStr;
use std::os::raw::c_char;

// ─── Object Types ───
// vpi_user.h constants for object types

pub const vpiModule: i32 = 5;
pub const vpiInstance: i32 = 6;
pub const vpiScope: i32 = 9;
pub const vpiIO: i32 = 10;
pub const vpiSysFunc: i32 = 11;
pub const vpiSysTask: i32 = 12;
pub const vpiConstant: i32 = 17;
pub const vpiParameter: i32 = 21;
pub const vpiNet: i32 = 23;
pub const vpiReg: i32 = 28;
pub const vpiMemory: i32 = 30;
pub const vpiIntegerVar: i32 = 34;
pub const vpiTimeVar: i32 = 37;
pub const vpiRealVar: i32 = 39;
pub const vpiStringVar: i32 = 45;
pub const vpiPort: i32 = 49;
pub const vpiOperation: i32 = 61;
pub const vpiGenScope: i32 = 66;
pub const vpiAssignment: i32 = 69;
pub const vpiFor: i32 = 72;
pub const vpiWhile: i32 = 73;
pub const vpiRepeat: i32 = 74;
pub const vpiForever: i32 = 75;
pub const vpiIf: i32 = 76;
pub const vpiIfElse: i32 = 77;
pub const vpiCase: i32 = 78;
pub const vpiBegin: i32 = 82;
pub const vpiFork: i32 = 83;
pub const vpiProcess: i32 = 85;
pub const vpiSysTfCall: i32 = 89;
pub const vpiFunction: i32 = 94;
pub const vpiTask: i32 = 95;
pub const vpiClassDefn: i32 = 108;
pub const vpiClassObj: i32 = 109;
pub const vpiCoverageEvent: i32 = 126;
pub const vpiAssertion: i32 = 127;

// ─── Property IDs ───

pub const vpiType: i32 = 1;
pub const vpiName: i32 = 2;
pub const vpiFullName: i32 = 3;
pub const vpiSize: i32 = 4;
pub const vpiFile: i32 = 5;
pub const vpiLineNo: i32 = 6;
pub const vpiTopModule: i32 = 7;
pub const vpiDefName: i32 = 11;
pub const vpiTimeUnit: i32 = 17;
pub const vpiTimePrecision: i32 = 18;
pub const vpiDirection: i32 = 22;
pub const vpiConstType: i32 = 26;
pub const vpiNetType: i32 = 34;
pub const vpiRegType: i32 = 37;
pub const vpiVector: i32 = 39;
pub const vpiExpanded: i32 = 40;
pub const vpiLeftRange: i32 = 42;
pub const vpiRightRange: i32 = 43;
pub const vpiParent: i32 = 65;
pub const vpiArray: i32 = 77;
pub const vpiIsScalar: i32 = 78;
pub const vpiDelay: i32 = 80;
pub const vpiPosedge: i32 = 97;
pub const vpiNegedge: i32 = 98;
pub const vpiOpType: i32 = 110;
pub const vpiCaseType: i32 = 111;
pub const vpiIterator: i32 = 117;
pub const vpiData: i32 = 141;
pub const vpiAlways: i32 = 182;
pub const vpiAlwaysComb: i32 = 183;
pub const vpiAlwaysFF: i32 = 184;
pub const vpiAlwaysLatch: i32 = 185;
pub const vpiInitial: i32 = 192;
pub const vpiFinal: i32 = 193;
pub const vpiAssert: i32 = 194;
pub const vpiAssume: i32 = 195;
pub const vpiCover: i32 = 196;
pub const vpiProperty: i32 = 197;
pub const vpiSequence: i32 = 198;
pub const vpiClockingBlock: i32 = 201;

// ─── Direction ───

pub const vpiInput: i32 = 1;
pub const vpiOutput: i32 = 2;
pub const vpiInout: i32 = 3;
pub const vpiNoDirection: i32 = 4;

// ─── Value Formats ───

pub const vpiBinStrVal: i32 = 1;
pub const vpiOctStrVal: i32 = 2;
pub const vpiDecStrVal: i32 = 3;
pub const vpiHexStrVal: i32 = 4;
pub const vpiScalarVal: i32 = 5;
pub const vpiIntVal: i32 = 6;
pub const vpiRealVal: i32 = 7;
pub const vpiStringVal: i32 = 8;
pub const vpiVectorVal: i32 = 9;
pub const vpiStrengthVal: i32 = 10;
pub const vpiTimeVal: i32 = 11;
pub const vpiObjTypeVal: i32 = 12;

// ─── VPI Vector Value ───

pub const VPI_MAX_NBITS: usize = 64;

/// C-compatible struct for VPI vector value (aval/bval format)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct t_vpi_vector {
    pub aval: u32,
    pub bval: u32,
}

/// C-compatible struct for VPI time
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct t_vpi_time {
    pub ttype: i32,
    pub low: u32,
    pub high: u32,
    pub real: f64,
}

impl t_vpi_time {
    pub fn new_sim_time() -> Self {
        t_vpi_time {
            ttype: 0, // vpiScaledRealTime
            low: 0,
            high: 0,
            real: 0.0,
        }
    }
}

/// C-compatible struct for VPI delay
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct t_vpi_delay {
    pub da: *mut t_vpi_time,
    pub no_of_delays: i32,
    pub time_type_p: *mut t_vpi_time,
    pub mt_flag: i32,
    pub app_flag: i32,
    pub cb_trigger: Option<unsafe extern "C" fn()>,
}

/// C-compatible union for VPI value
#[repr(C)]
#[derive(Clone)]
pub union vpi_value_union {
    pub scalar: i32,
    pub integer: i32,
    pub real: f64,
    pub string: *mut c_char,
    pub vector: t_vpi_vector,
    pub time: t_vpi_time,
}

// Manual Debug for union to avoid reading from uninitialized fields
impl std::fmt::Debug for vpi_value_union {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "vpi_value_union {{ ... }}")
    }
}

// Unsafe: vpi_value_union is only used behind &/Mutex with proper locking
unsafe impl Send for vpi_value_union {}
unsafe impl Sync for vpi_value_union {}
impl Copy for vpi_value_union {}

// VPI FFI structs contain raw pointers — inherently not Send/Sync, but
// they're only used behind Mutex or at FFI boundaries with proper locking.
unsafe impl Send for t_cb_data {}
unsafe impl Sync for t_cb_data {}
unsafe impl Send for s_vpi_systf_data {}
unsafe impl Sync for s_vpi_systf_data {}
unsafe impl Send for t_vpi_delay {}
unsafe impl Sync for t_vpi_delay {}
unsafe impl Send for t_vpi_value {}
unsafe impl Sync for t_vpi_value {}

/// C-compatible struct for VPI value
#[repr(C)]
#[derive(Debug, Clone)]
pub struct t_vpi_value {
    pub format: i32,
    pub value: vpi_value_union,
}

impl Default for t_vpi_value {
    fn default() -> Self {
        t_vpi_value {
            format: vpiIntVal,
            value: vpi_value_union { integer: 0 },
        }
    }
}

// ─── Callback Reasons ───

pub const cbValueChange: i32 = 1;
pub const cbStmt: i32 = 2;
pub const cbForce: i32 = 3;
pub const cbRelease: i32 = 4;
pub const cbAssign: i32 = 5;
pub const cbDeassign: i32 = 6;
pub const cbDelay: i32 = 7;
pub const cbCancelDelay: i32 = 8;
pub const cbEvent: i32 = 9;
pub const cbAfterDelay: i32 = 10;
pub const cbEndOfCompile: i32 = 11;
pub const cbEndOfReset: i32 = 23;
pub const cbEndOfSimulation: i32 = 24;
pub const cbStartOfSimulation: i32 = 25;
pub const cbReadWriteSynch: i32 = 30;
pub const cbReadOnlySynch: i32 = 31;
pub const cbNextSimTime: i32 = 32;
pub const cbEndOfTest: i32 = 33;
pub const cbTimeUnit: i32 = 35;
pub const cbForceRelease: i32 = 36;
pub const cbResume: i32 = 37;
pub const cbSuspend: i32 = 38;
pub const cbEnterInteractive: i32 = 46;
pub const cbExitInteractive: i32 = 47;
pub const cbInteractiveScope: i32 = 48;
pub const cbUnresolvedSystf: i32 = 49;
pub const cbAssignVal: i32 = 50;
pub const cbPLIError: i32 = 52;

/// C-compatible struct for VPI callback data
#[repr(C)]
#[derive(Debug, Clone)]
pub struct t_cb_data {
    pub reason: i32,
    pub cb_rtn: Option<unsafe extern "C" fn(*mut t_cb_data) -> i32>,
    pub user_data: *mut std::ffi::c_void,
    pub time: *mut t_vpi_time,
    pub value: *mut t_vpi_value,
    pub index: i32,
    pub obj: vpiHandle,
    pub obj_type: i32,
}

/// C-compatible struct for VPI system task/function data
#[repr(C)]
#[derive(Debug, Clone)]
pub struct s_vpi_systf_data {
    pub task_function_type: i32,
    pub tfname: *mut c_char,
    pub calltf: Option<unsafe extern "C" fn(*mut std::ffi::c_void) -> i32>,
    pub compiletf: Option<unsafe extern "C" fn(*mut std::ffi::c_void) -> i32>,
    pub sizetf: Option<unsafe extern "C" fn(*mut std::ffi::c_void) -> u32>,
    pub user_data: *mut std::ffi::c_void,
}

pub const vpiSystfTask: i32 = 1;
pub const vpiSystfFunc: i32 = 2;
pub const vpiSystfFuncInt: i32 = 3;
pub const vpiSystfFuncReal: i32 = 4;
pub const vpiSystfFuncStr: i32 = 5;
pub const vpiSystfFuncSized: i32 = 6;
pub const vpiSystfFuncTime: i32 = 7;

/// VPI Handle (opaque pointer to internal VPI object).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct vpiHandle {
    pub ptr: *mut std::ffi::c_void,
}

impl vpiHandle {
    pub const NULL: vpiHandle = vpiHandle {
        ptr: std::ptr::null_mut(),
    };

    pub fn is_null(&self) -> bool {
        self.ptr.is_null()
    }

    pub fn is_valid(&self) -> bool {
        !self.ptr.is_null()
    }
}

// ─── Error Codes & Types ───

pub const vpiNoError: i32 = 0;
pub const vpiWarning: i32 = 1;
pub const vpiError: i32 = 2;
pub const vpiInternal: i32 = 3;

pub const vpiReturnType: i32 = 51;
pub const vpiSigned: i32 = 53;
pub const vpiSuppressWarnings: i32 = 54;

// ─── Control Types ───

pub const vpiStop: i32 = 1;
pub const vpiFinish: i32 = 2;
pub const vpiReset: i32 = 3;
pub const vpiSetInteractiveScope: i32 = 5;

/// Convert a `*const c_char` C string to a Rust &str.
/// Returns "" on null or invalid UTF-8.
pub unsafe fn cstr_to_str<'a>(ptr: *const c_char) -> &'a str {
    if ptr.is_null() {
        return "";
    }
    CStr::from_ptr(ptr).to_str().unwrap_or("")
}
