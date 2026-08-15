//! VPI Handle System — Object Model Wrapper.
//!
//! Maps IR design objects (modules, signals, processes, etc.) to VPI handles.
//! Handles are opaque pointers to internal VpiObject instances.

use super::types::*;
use maria_ir::*;
use maria_core::intern::Symbol;
use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicU64, Ordering};

/// Internal VPI object types
#[derive(Debug, Clone)]
pub(crate) enum VpiObjectKind {
    Null,
    Module(usize, Symbol),          // index in design.modules
    Signal(SignalId, usize),        // (signal_id, instance_level)
    Process(usize),                 // process index
    Port(SignalId, usize),          // (signal_id, port_index)
    Scope(String),                  // fully qualified scope name
    Iterator {
        kind: VpiIteratorKind,
        items: Vec<VpiObject>,
        cursor: usize,
    },
    SysTfCall {
        name: String,
        is_function: bool,
    },
    Time(u64),
}

#[derive(Debug, Clone)]
pub(crate) enum VpiIteratorKind {
    ModuleSignals,
    ModulePorts,
    ModuleProcesses,
    TopModules,
    ScopeChildren,
    NetBits,
}

/// Internal VPI object (hidden behind opaque handle)
#[derive(Debug, Clone)]
pub(crate) struct VpiObject {
    pub kind: VpiObjectKind,
    pub(crate) id: u64,
}

impl VpiObject {
    pub fn new(kind: VpiObjectKind) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        VpiObject {
            kind,
            id: NEXT_ID.fetch_add(1, Ordering::SeqCst),
        }
    }

    pub fn null() -> Self {
        VpiObject {
            kind: VpiObjectKind::Null,
            id: 0,
        }
    }
}

// ─── Object Registry ───

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::OnceLock;

fn vpi_objects() -> &'static Mutex<HashMap<u64, VpiObject>> {
    static MAP: OnceLock<Mutex<HashMap<u64, VpiObject>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register a VPI object and return an opaque handle.
pub(crate) fn vpi_register_object(obj: VpiObject) -> vpiHandle {
    let id = obj.id;
    let mut registry = vpi_objects().lock().unwrap();
    registry.insert(id, obj);
    vpiHandle {
        ptr: id as *mut std::ffi::c_void,
    }
}

/// Look up a VPI object by handle.
pub(crate) fn vpi_lookup_object(handle: vpiHandle) -> Option<VpiObject> {
    if handle.is_null() {
        return None;
    }
    let id = handle.ptr as u64;
    let registry = vpi_objects().lock().unwrap();
    registry.get(&id).cloned()
}

/// Remove a VPI object (for vpi_free_object).
pub(crate) fn vpi_remove_object(handle: vpiHandle) -> bool {
    if handle.is_null() {
        return false;
    }
    let id = handle.ptr as u64;
    let mut registry = vpi_objects().lock().unwrap();
    registry.remove(&id).is_some()
}

/// Clear all VPI objects (called at end of simulation).
pub(crate) fn vpi_clear_all_objects() {
    let mut registry = vpi_objects().lock().unwrap();
    registry.clear();
}

// ─── vpi_handle ───

/// vpi_handle(type, ref_handle) — get the handle of a related object.
pub fn vpi_handle(vpi_type: i32, ref_handle: vpiHandle) -> vpiHandle {
    let obj = match vpi_lookup_object(ref_handle) {
        Some(o) => o,
        None => return vpiHandle::NULL,
    };
    match (vpi_type, &obj.kind) {
        (vpiModule, VpiObjectKind::Signal(_, _)) => {
            // Return the module that contains this signal
            // For now, return the top module
            super::with_vpi_engine(|engine| {
                let name = engine.design.top.name;
                let obj = VpiObject::new(VpiObjectKind::Module(0, name));
                vpi_register_object(obj)
            }).unwrap_or(vpiHandle::NULL)
        }
        (vpiScope, VpiObjectKind::Module(_, _)) => {
            // Module is also a scope
            ref_handle
        }
        (vpiReg | vpiNet, VpiObjectKind::Signal(_, _)) => {
            ref_handle
        }
        (vpiParent, VpiObjectKind::Signal(_, _)) => {
            // Return the module containing this signal
            super::with_vpi_engine(|engine| {
                let name = engine.design.top.name;
                let obj = VpiObject::new(VpiObjectKind::Module(0, name));
                vpi_register_object(obj)
            }).unwrap_or(vpiHandle::NULL)
        }
        (vpiParent, _) => vpiHandle::NULL,
        _ => vpiHandle::NULL,
    }
}

// ─── vpi_handle_by_name ───

/// vpi_handle_by_name(name, scope) — find an object by its full hierarchical name.
pub fn vpi_handle_by_name(name: &str, scope: vpiHandle) -> vpiHandle {
    let _ = scope; // Ignore scope for now (search top-level)
    super::with_vpi_engine(|engine| {
        // 1. Check module names
        for (idx, (_, module)) in engine.design.modules.iter().enumerate() {
            if module.name.as_str() == name {
                let obj = VpiObject::new(VpiObjectKind::Module(idx, module.name));
                return vpi_register_object(obj);
            }
        }
        // 2. Check top module
        if engine.design.top.name.as_str() == name {
            let obj = VpiObject::new(VpiObjectKind::Module(0, engine.design.top.name));
            return vpi_register_object(obj);
        }
        // 3. Check signal names (top level)
        for (sig_id, sig) in engine.design.top.signals.iter().enumerate() {
            if sig.name.as_str() == name {
                let obj = VpiObject::new(VpiObjectKind::Signal(sig_id, 0));
                return vpi_register_object(obj);
            }
            // Also check hierarchical name
            let full_name = format!("{}.{}", engine.design.top.name, sig.name);
            if full_name == name {
                let obj = VpiObject::new(VpiObjectKind::Signal(sig_id, 0));
                return vpi_register_object(obj);
            }
        }
        vpiHandle::NULL
    }).unwrap_or(vpiHandle::NULL)
}

// ─── vpi_iterate ───

/// vpi_iterate(type, ref_handle) — create an iterator for objects related to ref_handle.
pub fn vpi_iterate(vpi_type: i32, ref_handle: vpiHandle) -> vpiHandle {
    let obj = match vpi_lookup_object(ref_handle) {
        Some(o) => o,
        None => return vpiHandle::NULL,
    };
    match (vpi_type, &obj.kind) {
        (vpiReg | vpiNet, VpiObjectKind::Module(_, _)) => {
            // Iterate signals of a module
            super::with_vpi_engine(|engine| {
                let sigs: Vec<VpiObject> = engine.design.top.signals.iter().enumerate()
                    .filter(|(_, s)| match vpi_type {
                        vpiReg => s.kind == SignalKind::Reg || s.kind == SignalKind::Logic,
                        vpiNet => s.kind == SignalKind::Wire,
                        _ => true,
                    })
                    .map(|(i, _)| VpiObject::new(VpiObjectKind::Signal(i, 0)))
                    .collect();
                if sigs.is_empty() {
                    return vpiHandle::NULL;
                }
                let iter = VpiObject::new(VpiObjectKind::Iterator {
                    kind: VpiIteratorKind::ModuleSignals,
                    items: sigs,
                    cursor: 0,
                });
                vpi_register_object(iter)
            }).unwrap_or(vpiHandle::NULL)
        }
        (vpiPort, VpiObjectKind::Module(_, _)) => {
            // Iterate ports of a module
            super::with_vpi_engine(|engine| {
                let ports = engine.design.top.inputs.iter().chain(engine.design.top.outputs.iter())
                    .chain(engine.design.top.inouts.iter())
                    .enumerate()
                    .map(|(i, sig_id)| VpiObject::new(VpiObjectKind::Port(*sig_id, i)))
                    .collect::<Vec<_>>();
                if ports.is_empty() {
                    return vpiHandle::NULL;
                }
                let iter = VpiObject::new(VpiObjectKind::Iterator {
                    kind: VpiIteratorKind::ModulePorts,
                    items: ports,
                    cursor: 0,
                });
                vpi_register_object(iter)
            }).unwrap_or(vpiHandle::NULL)
        }
        (vpiTopModule, _) => {
            // Iterate all top-level modules
            super::with_vpi_engine(|engine| {
                let modules: Vec<VpiObject> = engine.design.modules.iter()
                    .enumerate()
                    .map(|(i, (name, _))| VpiObject::new(VpiObjectKind::Module(i, *name)))
                    .collect();
                if modules.is_empty() {
                    return vpiHandle::NULL;
                }
                let iter = VpiObject::new(VpiObjectKind::Iterator {
                    kind: VpiIteratorKind::TopModules,
                    items: modules,
                    cursor: 0,
                });
                vpi_register_object(iter)
            }).unwrap_or(vpiHandle::NULL)
        }
        _ => vpiHandle::NULL,
    }
}

// ─── vpi_scan ───

/// vpi_scan(iterator) — get the next object from an iterator.
pub fn vpi_scan(iter_handle: vpiHandle) -> vpiHandle {
    let mut registry = vpi_objects().lock().unwrap();
    let obj = match registry.get_mut(&(iter_handle.ptr as u64)) {
        Some(o) => o,
        None => return vpiHandle::NULL,
    };
    match &mut obj.kind {
        VpiObjectKind::Iterator { items, cursor, .. } => {
            if *cursor >= items.len() {
                return vpiHandle::NULL;
            }
            let item = items[*cursor].clone();
            *cursor += 1;
            // Register the item as a new object
            let item_id = item.id;
            registry.insert(item_id, item);
            drop(registry);
            vpiHandle {
                ptr: item_id as *mut std::ffi::c_void,
            }
        }
        _ => vpiHandle::NULL,
    }
}

// ─── vpi_get ───

/// vpi_get(property, handle) — get an integer property of an object.
pub fn vpi_get(property: i32, handle: vpiHandle) -> i32 {
    let obj = match vpi_lookup_object(handle) {
        Some(o) => o,
        None => return 0,
    };
    match property {
        vpiType => {
            match &obj.kind {
                VpiObjectKind::Module(_, _) => vpiModule,
                VpiObjectKind::Signal(_, _) => vpiReg,
                VpiObjectKind::Process(_) => vpiProcess,
                VpiObjectKind::Port(_, _) => vpiPort,
                VpiObjectKind::Iterator { .. } => 0,
                VpiObjectKind::Scope(_) => vpiScope,
                VpiObjectKind::SysTfCall { .. } => vpiSysFunc,
                VpiObjectKind::Null => 0,
                VpiObjectKind::Time(_) => vpiTimeVar,
            }
        }
        vpiSize => {
            match &obj.kind {
                VpiObjectKind::Signal(sig_id, _) => {
                    super::with_vpi_engine(|engine| {
                        engine.design.top.signals.get(*sig_id).map(|s| s.width as i32).unwrap_or(0)
                    }).unwrap_or(0)
                }
                VpiObjectKind::Module(_, _) => {
                    super::with_vpi_engine(|engine| {
                        engine.design.top.signals.len() as i32
                    }).unwrap_or(0)
                }
                _ => 0,
            }
        }
        vpiDirection => {
            match &obj.kind {
                VpiObjectKind::Port(sig_id, _) | VpiObjectKind::Signal(sig_id, _) => {
                    super::with_vpi_engine(|engine| {
                        if let Some(sig) = engine.design.top.signals.get(*sig_id) {
                            match sig.kind {
                                SignalKind::Input => vpiInput,
                                SignalKind::Output => vpiOutput,
                                SignalKind::Inout => vpiInout,
                                _ => vpiNoDirection,
                            }
                        } else {
                            vpiNoDirection
                        }
                    }).unwrap_or(vpiNoDirection)
                }
                _ => vpiNoDirection,
            }
        }
        vpiVector => {
            match &obj.kind {
                VpiObjectKind::Signal(sig_id, _) => {
                    super::with_vpi_engine(|engine| {
                        engine.design.top.signals.get(*sig_id).map(|s| (s.width > 1) as i32).unwrap_or(0)
                    }).unwrap_or(0)
                }
                _ => 0,
            }
        }
        vpiSigned => {
            match &obj.kind {
                VpiObjectKind::Signal(sig_id, _) => {
                    super::with_vpi_engine(|engine| {
                        engine.design.top.signals.get(*sig_id).map(|s| s.is_signed as i32).unwrap_or(0)
                    }).unwrap_or(0)
                }
                _ => 0,
            }
        }
        vpiLeftRange => {
            match &obj.kind {
                VpiObjectKind::Signal(sig_id, _) => {
                    super::with_vpi_engine(|engine| {
                        engine.design.top.signals.get(*sig_id).map(|s| s.width as i32 - 1).unwrap_or(-1)
                    }).unwrap_or(-1)
                }
                _ => -1,
            }
        }
        vpiRightRange => {
            match &obj.kind {
                VpiObjectKind::Signal(_, _) => 0,
                _ => 0,
            }
        }
        vpiOpType => 0,
        vpiLineNo => 0,
        vpiRegType => {
            match &obj.kind {
                VpiObjectKind::Signal(sig_id, _) => {
                    super::with_vpi_engine(|engine| {
                        engine.design.top.signals.get(*sig_id).map(|s| {
                            #[allow(unreachable_patterns)]
                            match s.kind {
                                SignalKind::Reg => vpiReg,
                                SignalKind::Logic => vpiReg,
                                SignalKind::Wire => vpiNet,
                                SignalKind::Input => vpiReg,
                                SignalKind::Output => vpiReg,
                                SignalKind::Inout => vpiReg,
                                _ => vpiReg,
                            }
                        }).unwrap_or(vpiReg)
                    }).unwrap_or(vpiReg)
                }
                _ => vpiReg,
            }
        }
        vpiTimeUnit => -12, // Default ps
        vpiTimePrecision => -12,
        _ => 0,
    }
}

// ─── CString Cache ───
// Prevents memory leaks from CString::into_raw() by caching all
// allocated CStrings and freeing them on vpi_clear_all_objects().

static CSTRING_CACHE: std::sync::Mutex<Vec<std::ffi::CString>> = std::sync::Mutex::new(Vec::new());

/// Allocate a CString, cache it, and return a raw pointer.
/// The pointer remains valid until vpi_clear_all_objects() is called.
pub(crate) fn cache_cstring(s: &str) -> *mut c_char {
    let cstr = CString::new(s).unwrap_or_default();
    let ptr = cstr.as_ptr() as *mut c_char;
    let mut cache = CSTRING_CACHE.lock().unwrap();
    cache.push(cstr);
    ptr
}

/// Free all cached CStrings (called during VPI cleanup).
pub(crate) fn clear_cstring_cache() {
    let mut cache = CSTRING_CACHE.lock().unwrap();
    cache.clear(); // Drops all CStrings, freeing memory
}

// ─── vpi_get_str ───

/// vpi_get_str(property, handle) — get a string property of an object.
pub fn vpi_get_str(property: i32, handle: vpiHandle) -> *mut c_char {
    let obj = match vpi_lookup_object(handle) {
        Some(o) => o,
        None => return std::ptr::null_mut(),
    };
    let result = match property {
        vpiName | vpiFullName => {
            match &obj.kind {
                VpiObjectKind::Module(_, name) => name.as_str().to_string(),
                VpiObjectKind::Signal(sig_id, _) => {
                    super::with_vpi_engine(|engine| {
                        engine.design.top.signals.get(*sig_id).map(|s| s.name.to_string())
                    }).flatten().unwrap_or_default()
                }
                VpiObjectKind::Port(sig_id, _) => {
                    super::with_vpi_engine(|engine| {
                        engine.design.top.signals.get(*sig_id).map(|s| s.name.to_string())
                    }).flatten().unwrap_or_default()
                }
                VpiObjectKind::Scope(name) => name.clone(),
                VpiObjectKind::SysTfCall { name, .. } => name.clone(),
                _ => String::new(),
            }
        }
        vpiDefName => {
            match &obj.kind {
                VpiObjectKind::Module(_, name) => name.as_str().to_string(),
                _ => String::new(),
            }
        }
        vpiFile => {
            // SignalInfo/IrModule tidak punya file_path field —
            // gunakan module name sebagai identitas
            match &obj.kind {
                VpiObjectKind::Signal(sig_id, _) => {
                    super::with_vpi_engine(|engine| {
                        engine.design.top.signals.get(*sig_id)
                            .map(|_| engine.design.top.name.as_str().to_string())
                    }).flatten().unwrap_or_default()
                }
                VpiObjectKind::Module(_, name) => {
                    name.as_str().to_string()
                }
                _ => String::new(),
            }
        }
        vpiStringVal => String::new(),
        _ => String::new(),
    };
    if result.is_empty() {
        return std::ptr::null_mut();
    }
    cache_cstring(&result)
}

// ─── vpi_free_object ───

/// vpi_free_object(handle) — release a VPI object.
pub fn vpi_free_object(handle: vpiHandle) -> i32 {
    if vpi_remove_object(handle) { 1 } else { 0 }
}

// ─── vpi_chk_error ───

static VPI_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub fn vpi_set_error(msg: &str) {
    *VPI_ERROR.lock().unwrap() = Some(msg.to_string());
}

/// vpi_chk_error(error_p) — retrieve the last VPI error.
pub fn vpi_chk_error() -> i32 {
    let guard = VPI_ERROR.lock().unwrap();
    if guard.is_some() {
        vpiError
    } else {
        vpiNoError
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // VPI_OBJECTS adalah global static — semua test di modul ini berbagi
    // registry yang sama dan berjalan PARALEL. reset_registry() satu test
    // menghapus objek test lain yang sedang berjalan → race. Serialkan semua
    // test handle.rs dengan lock bersama (test unit murni, tidak ada locking
    // lain yang terlibat).
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn reset_registry() {
        vpi_clear_all_objects();
        *VPI_ERROR.lock().unwrap() = None;
    }

    #[test]
    fn test_vpi_object_registry_roundtrip() {
        let _g = TEST_LOCK.lock().unwrap();
        reset_registry();
        // register → lookup → get tipe → free → lookup none → free ganda 0
        let obj = VpiObject::new(VpiObjectKind::Module(0, Symbol::intern("top")));
        let h = vpi_register_object(obj);
        assert!(h.is_valid(), "handle harus valid");
        let got = vpi_lookup_object(h).expect("objek harus ada");
        assert!(matches!(got.kind, VpiObjectKind::Module(0, _)));
        assert_eq!(vpi_get(vpiType, h), vpiModule, "tipe Module");
        assert_eq!(vpi_free_object(h), 1, "free sukses");
        assert!(vpi_lookup_object(h).is_none(), "setelah free tidak ada");
        assert_eq!(vpi_free_object(h), 0, "free ganda → 0");
        reset_registry();
    }

    #[test]
    fn test_vpi_handle_passthrough() {
        let _g = TEST_LOCK.lock().unwrap();
        reset_registry();
        // vpiReg/vpiNet/vpiScope pd Signal/Module → handle sama (ref_handle)
        let sig = VpiObject::new(VpiObjectKind::Signal(0, 0));
        let h = vpi_register_object(sig);
        let reg = vpi_handle(vpiReg, h);
        assert_eq!(reg.ptr, h.ptr, "vpiReg → handle yang sama");
        let net = vpi_handle(vpiNet, h);
        assert_eq!(net.ptr, h.ptr, "vpiNet → handle yang sama");
        let modu = VpiObject::new(VpiObjectKind::Module(0, Symbol::intern("top")));
        let mh = vpi_register_object(modu);
        let sc = vpi_handle(vpiScope, mh);
        assert_eq!(sc.ptr, mh.ptr, "vpiScope pd Module → handle yang sama");
        // handle null → vpiHandle::NULL
        assert!(vpi_handle(vpiReg, vpiHandle::NULL).is_null());
        reset_registry();
    }

    #[test]
    fn test_vpi_free_object_null_and_clear() {
        let _g = TEST_LOCK.lock().unwrap();
        reset_registry();
        assert_eq!(vpi_free_object(vpiHandle::NULL), 0, "null → 0");
        for _ in 0..3 {
            let o = VpiObject::new(VpiObjectKind::Scope("s".to_string()));
            let _ = vpi_register_object(o);
        }
        assert!(!vpi_objects().lock().unwrap().is_empty());
        reset_registry();
        assert!(vpi_objects().lock().unwrap().is_empty(), "clear semua objek");
    }

    #[test]
    fn test_vpi_get_str_module_name() {
        let _g = TEST_LOCK.lock().unwrap();
        reset_registry();
        let m = VpiObject::new(VpiObjectKind::Module(0, Symbol::intern("counter")));
        let h = vpi_register_object(m);
        let name = vpi_get_str(vpiName, h);
        assert!(!name.is_null(), "vpi_get_str vpiName harus non-null");
        assert_eq!(unsafe { cstr_to_str(name) }, "counter");
        // nama tidak terdaftar → null
        assert!(vpi_get_str(vpiName, vpiHandle::NULL).is_null());
        reset_registry();
    }

    #[test]
    fn test_vpi_chk_error_set_clear() {
        let _g = TEST_LOCK.lock().unwrap();
        reset_registry();
        assert_eq!(vpi_chk_error(), vpiNoError, "awal tanpa error");
        vpi_set_error("signal not found");
        assert_eq!(vpi_chk_error(), vpiError, "setelah set → vpiError");
        *VPI_ERROR.lock().unwrap() = None;
        assert_eq!(vpi_chk_error(), vpiNoError, "setelah clear → vpiNoError");
    }

    #[test]
    fn test_vpi_scan_iterator_empty() {
        let _g = TEST_LOCK.lock().unwrap();
        reset_registry();
        // scan pd handle bukan iterator → NULL
        let m = VpiObject::new(VpiObjectKind::Module(0, Symbol::intern("top")));
        let h = vpi_register_object(m);
        assert!(vpi_scan(h).is_null(), "scan non-iterator → NULL");
        assert!(vpi_scan(vpiHandle::NULL).is_null(), "scan null → NULL");
        reset_registry();
    }
}
