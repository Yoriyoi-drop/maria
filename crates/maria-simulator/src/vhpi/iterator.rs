//! VHPI Iterator (IEEE 1076-2008 §C.5.7) — vhpi_iterate / vhpi_scan.
//!
//! `vhpi_iterate(object_kind, handle)` membuat iterator atas object terkait;
//! `vhpi_scan(iterator)` mengembalikan object berikutnya (NULL saat habis).
#![allow(non_upper_case_globals)]

use super::handle::{VhpiHandle, VhpiObjectKind};
use super::object::*;
use maria_ir::SignalKind;

/// vhpi_iterate(kind, ref_handle) — iterator object terkait ref_handle.
pub fn vhpi_iterate(kind: i32, ref_handle: VhpiHandle) -> VhpiHandle {
    let obj = match super::handle::lookup(ref_handle) {
        Some(o) => o,
        None => return VhpiHandle::NULL,
    };
    match (kind, &obj.kind) {
        (vhpiSignal, VhpiObjectKind::Module(_, _)) => {
            // Iterate semua signal modul (bukan port).
            with_vhpi_engine(|engine| {
                let items: Vec<VhpiHandle> = engine.design.top.signals.iter().enumerate()
                    .filter(|(_, s)| !matches!(s.kind, SignalKind::Input | SignalKind::Output | SignalKind::Inout))
                    .map(|(i, _)| super::handle::register_object(VhpiObjectKind::Signal(i, 0)))
                    .collect();
                if items.is_empty() { return VhpiHandle::NULL; }
                super::handle::register_object(VhpiObjectKind::Iterator { items, cursor: 0 })
            }).unwrap_or(VhpiHandle::NULL)
        }
        (vhpiPort, VhpiObjectKind::Module(_, _)) => {
            with_vhpi_engine(|engine| {
                let items: Vec<VhpiHandle> = engine.design.top.signals.iter().enumerate()
                    .filter(|(_, s)| matches!(s.kind, SignalKind::Input | SignalKind::Output | SignalKind::Inout))
                    .map(|(i, _)| super::handle::register_object(VhpiObjectKind::Port(i, 0)))
                    .collect();
                if items.is_empty() { return VhpiHandle::NULL; }
                super::handle::register_object(VhpiObjectKind::Iterator { items, cursor: 0 })
            }).unwrap_or(VhpiHandle::NULL)
        }
        _ => VhpiHandle::NULL,
    }
}

/// vhpi_scan(iterator) — object berikutnya, NULL saat habis.
pub fn vhpi_scan(iter_handle: VhpiHandle) -> VhpiHandle {
    if iter_handle.is_null() { return VhpiHandle::NULL; }
    let id = iter_handle.ptr as u64;
    let mut reg = super::handle::vhpi_objects_for_scan();
    let next = {
        let obj = match reg.get_mut(&id) {
            Some(o) => o,
            None => return VhpiHandle::NULL,
        };
        match &mut obj.kind {
            VhpiObjectKind::Iterator { items, cursor } => {
                if *cursor >= items.len() { return VhpiHandle::NULL; }
                let item = items[*cursor];
                *cursor += 1;
                item
            }
            _ => return VhpiHandle::NULL,
        }
    };
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_vhpi_scan_null_and_non_iterator() {
        let _g = TEST_LOCK.lock().unwrap();
        crate::vhpi::handle::vhpi_clear_all_objects();
        // scan NULL → NULL
        assert!(vhpi_scan(VhpiHandle::NULL).is_null());
        // scan pada object bukan iterator → NULL
        let sig = crate::vhpi::handle::register_object(VhpiObjectKind::Signal(0, 0));
        assert!(vhpi_scan(sig).is_null());
        crate::vhpi::handle::vhpi_clear_all_objects();
    }
}
