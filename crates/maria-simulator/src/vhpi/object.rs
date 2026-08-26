//! VHPI Object Model (IEEE 1076-2008) — konstanta standar + lookup object.
//!
//! Hanya konstanta dan fungsi inti yang diimplementasikan di sini (subset
//! yang relevan untuk adapter Maria). Konstanta mengikuti `vhpi_user.h`
//! IEEE 1076-2008 — BUKAN versi Maria sendiri (poin 1 masukan user:
//! compatibility layer mengikuti API/ABI target).
#![allow(non_upper_case_globals)]

use super::handle::{VhpiHandle, VhpiObjectKind};
use std::ffi::CString;
use std::sync::Mutex;

// ─── Object Kinds (IEEE 1076-2008 §C.5.4) ───

pub const vhpiDesignUnit: i32 = 1;
pub const vhpiArchitecture: i32 = 2;
pub const vhpiConfiguration: i32 = 3;
pub const vhpiLibrary: i32 = 4;
pub const vhpiEntity: i32 = 5;
pub const vhpiBlockDecl: i32 = 6;
pub const vhpiProcessDecl: i32 = 7;
pub const vhpiProcessStmt: i32 = 8;
pub const vhpiSignalDecl: i32 = 9;
pub const vhpiVariableDecl: i32 = 10;
pub const vhpiConstantDecl: i32 = 11;
pub const vhpiPortDecl: i32 = 12;
pub const vhpiSignal: i32 = 13;
pub const vhpiVariable: i32 = 14;
pub const vhpiConstant: i32 = 15;
pub const vhpiPort: i32 = 16;
pub const vhpiFunctionDecl: i32 = 17;
pub const vhpiProcedureDecl: i32 = 18;
pub const vhpiInstance: i32 = 19;
pub const vhpiGeneric: i32 = 20;
pub const vhpiParameter: i32 = 21;
pub const vhpiNet: i32 = 22;
pub const vhpiReg: i32 = 23;

// ─── Properties (IEEE 1076-2008 §C.5.5) ───

pub const vhpiName: i32 = 1;
pub const vhpiFullName: i32 = 2;
pub const vhpiKind: i32 = 3;
pub const vhpiMode: i32 = 4;
pub const vhpiDirection: i32 = 5;
pub const vhpiSize: i32 = 6;
pub const vhpiType: i32 = 7;
pub const vhpiLeft: i32 = 8;
pub const vhpiRight: i32 = 9;
pub const vhpiLow: i32 = 10;
pub const vhpiHigh: i32 = 11;
pub const vhpiLevel: i32 = 12;
pub const vhpiTop: i32 = 13;
pub const vhpiDriver: i32 = 14;
pub const vhpiPortType: i32 = 15;

// ─── Direction / Mode ───

pub const vhpiIn: i32 = 1;
pub const vhpiOut: i32 = 2;
pub const vhpiInout: i32 = 3;
pub const vhpiBuffer: i32 = 4;
pub const vhpiLinkage: i32 = 5;
pub const vhpiNoDirection: i32 = 6;

// ─── Helper: engine pointer (sama pola vpi/mod.rs) ───

use crate::simulator::engine::SimulationEngine;

thread_local! {
    /// Pointer ke `SimulationEngine` aktif, per-thread (lihat VPI_ENGINE di
    /// `crate::vpi` — alasan thread-local sama: cegah deref dangling pointer
    /// engine dari thread lain saat simulasi paralel).
    static VHPI_ENGINE: std::cell::Cell<*mut SimulationEngine> =
        const { std::cell::Cell::new(std::ptr::null_mut()) };
}

pub fn set_vhpi_engine(engine: &mut SimulationEngine) {
    VHPI_ENGINE.with(|e| e.set(engine));
}

pub fn clear_vhpi_engine() {
    VHPI_ENGINE.with(|e| e.set(std::ptr::null_mut()));
}

pub fn with_vhpi_engine<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut SimulationEngine) -> R,
{
    VHPI_ENGINE.with(|e| unsafe { e.get().as_mut() }.map(f))
}

// ─── vhpi_handle_by_name ───

/// Temukan object VHPI berdasarkan nama hierarkis (mis. `top.clk`).
/// Memetakan ke signal/port/module Maria. Bila tidak ditemukan → NULL.
pub fn vhpi_handle_by_name(name: &str, _scope: VhpiHandle) -> VhpiHandle {
    with_vhpi_engine(|engine| {
        // 1. Signal di top.
        for (sig_id, sig) in engine.design.top.signals.iter().enumerate() {
            if sig.name.as_str() == name {
                return super::handle::register_object(VhpiObjectKind::Signal(sig_id, 0));
            }
            let full = format!("{}.{}", engine.design.top.name.as_str(), sig.name.as_str());
            if full == name {
                return super::handle::register_object(VhpiObjectKind::Signal(sig_id, 0));
            }
        }
        // 2. Port (input/output/inout).
        for (sig_id, sig) in engine.design.top.signals.iter().enumerate() {
            if sig.name.as_str() == name
                && matches!(
                    sig.kind,
                    maria_ir::SignalKind::Input
                        | maria_ir::SignalKind::Output
                        | maria_ir::SignalKind::Inout
                )
            {
                return super::handle::register_object(VhpiObjectKind::Port(sig_id, 0));
            }
        }
        // 3. Modul (design unit / instance).
        if engine.design.top.name.as_str() == name {
            return super::handle::register_object(VhpiObjectKind::Module(
                0,
                engine.design.top.name,
            ));
        }
        VhpiHandle::NULL
    })
    .unwrap_or(VhpiHandle::NULL)
}

// ─── vhpi_get / vhpi_get_str ───

/// vhpi_get(property, handle) — properti integer.
pub fn vhpi_get(property: i32, handle: VhpiHandle) -> i32 {
    let obj = match super::handle::lookup(handle) {
        Some(o) => o,
        None => return 0,
    };
    match property {
        vhpiKind => match &obj.kind {
            VhpiObjectKind::Module(_, _) => vhpiArchitecture,
            VhpiObjectKind::Signal(_, _) => vhpiSignal,
            VhpiObjectKind::Port(_, _) => vhpiPort,
            VhpiObjectKind::Process(_) => vhpiProcessStmt,
            VhpiObjectKind::Iterator { .. } => 0,
            VhpiObjectKind::Null => 0,
            VhpiObjectKind::Time(_) => vhpiTime,
            VhpiObjectKind::Scope(_) => vhpiBlockDecl,
        },
        vhpiSize => match &obj.kind {
            VhpiObjectKind::Signal(sig_id, _) | VhpiObjectKind::Port(sig_id, _) => {
                with_vhpi_engine(|e| {
                    e.design
                        .top
                        .signals
                        .get(*sig_id)
                        .map(|s| s.width as i32)
                        .unwrap_or(0)
                })
                .unwrap_or(0)
            }
            VhpiObjectKind::Module(_, _) => {
                with_vhpi_engine(|e| e.design.top.signals.len() as i32).unwrap_or(0)
            }
            _ => 0,
        },
        vhpiDirection => match &obj.kind {
            VhpiObjectKind::Port(sig_id, _) => with_vhpi_engine(|e| {
                e.design
                    .top
                    .signals
                    .get(*sig_id)
                    .map(|s| match s.kind {
                        maria_ir::SignalKind::Input => vhpiIn,
                        maria_ir::SignalKind::Output => vhpiOut,
                        maria_ir::SignalKind::Inout => vhpiInout,
                        _ => vhpiNoDirection,
                    })
                    .unwrap_or(vhpiNoDirection)
            })
            .unwrap_or(vhpiNoDirection),
            _ => vhpiNoDirection,
        },
        _ => 0,
    }
}

pub const vhpiTime: i32 = 24; // object kind TimeVar

/// Cache CString agar pointer vhpi_get_str tetap valid (pola vpi cache_cstring).
pub(crate) fn cache_cstring(s: &str) -> *mut std::os::raw::c_char {
    use std::os::raw::c_char;
    static CACHE: Mutex<Vec<CString>> = Mutex::new(Vec::new());
    let cstr = CString::new(s).unwrap_or_default();
    let ptr = cstr.as_ptr() as *mut c_char;
    CACHE.lock().unwrap().push(cstr);
    ptr
}

pub(crate) fn clear_cstring_cache() {
    static CACHE: Mutex<Vec<CString>> = Mutex::new(Vec::new());
    CACHE.lock().unwrap().clear();
}

/// vhpi_get_str(property, handle) — properti string (name, full name).
pub fn vhpi_get_str(property: i32, handle: VhpiHandle) -> *mut std::os::raw::c_char {
    let obj = match super::handle::lookup(handle) {
        Some(o) => o,
        None => return std::ptr::null_mut(),
    };
    let result = match property {
        vhpiName | vhpiFullName => match &obj.kind {
            VhpiObjectKind::Module(_, name) => name.as_str().to_string(),
            VhpiObjectKind::Signal(sig_id, _) | VhpiObjectKind::Port(sig_id, _) => {
                with_vhpi_engine(|e| {
                    e.design
                        .top
                        .signals
                        .get(*sig_id)
                        .map(|s| s.name.to_string())
                })
                .flatten()
                .unwrap_or_default()
            }
            VhpiObjectKind::Scope(name) => name.clone(),
            _ => String::new(),
        },
        _ => String::new(),
    };
    if result.is_empty() {
        std::ptr::null_mut()
    } else {
        cache_cstring(&result)
    }
}

/// vhpi_is_defined(kind) — apakah object kind di-support adapter Maria.
pub fn vhpi_is_defined(kind: i32) -> i32 {
    match kind {
        vhpiSignal | vhpiPort | vhpiArchitecture | vhpiProcessStmt | vhpiInstance => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // VHPI_ENGINE global — test handle/object murni tanpa engine.
    // FIX flake (ROUND 106): lock bersama lintas modul vhpi.
    use crate::vhpi::VHPI_TEST_LOCK as TEST_LOCK;

    #[test]
    fn test_vhpi_is_defined_supported() {
        assert_eq!(vhpi_is_defined(vhpiSignal), 1);
        assert_eq!(vhpi_is_defined(vhpiPort), 1);
        assert_eq!(vhpi_is_defined(vhpiArchitecture), 1);
        assert_eq!(vhpi_is_defined(999_999), 0, "kind tak dikenal → 0");
    }

    #[test]
    fn test_vhpi_constants_match_ieee() {
        // Nilai-nilai kunci IEEE 1076-2008 (vhpi_user.h).
        assert_eq!(vhpiDesignUnit, 1);
        assert_eq!(vhpiArchitecture, 2);
        assert_eq!(vhpiSignalDecl, 9);
        assert_eq!(vhpiPortDecl, 12);
        assert_eq!(vhpiName, 1);
        assert_eq!(vhpiKind, 3);
        assert_eq!(vhpiIn, 1);
        assert_eq!(vhpiOut, 2);
        assert_eq!(vhpiInout, 3);
    }

    #[test]
    fn test_handle_by_name_without_engine_returns_null() {
        let _g = TEST_LOCK.lock().unwrap();
        clear_vhpi_engine();
        assert!(vhpi_handle_by_name("top.clk", VhpiHandle::NULL).is_null());
    }
}
