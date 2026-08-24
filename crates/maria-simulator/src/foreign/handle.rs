//! Foreign Handle Registry (arsitektur masukan user poin 6).
//!
//! Library eksternal TIDAK pernah menerima pointer internal Maria. Semua
//! handle adalah `u64` id yang dipetakan ke object Maria melalui registry —
//! aman (tidak ada dangling pointer), mudah lifetime management, dan bisa
//! di-serialize/di-debug.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Jenis object Maria yang bisa di-reference foreign library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleKind {
    Module,
    Instance,
    Net,
    Variable,
    Memory,
    Port,
    Process,
    Scope,
    Signal,
    SysTf,
    Callback,
    Iterator,
    Time,
}

/// Handle foreign — opaque `u64` id, bukan pointer.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForeignHandle {
    pub id: u64,
    pub kind: HandleKind,
}

impl ForeignHandle {
    pub const NULL: ForeignHandle = ForeignHandle {
        id: 0,
        kind: HandleKind::Scope,
    };
    pub const fn null() -> Self {
        ForeignHandle::NULL
    }
    pub fn is_null(&self) -> bool {
        self.id == 0
    }
    pub fn is_valid(&self) -> bool {
        self.id != 0
    }
}

/// Registry: id → object (disimpan sebagai `Box<dyn Any>` agar bisa menampung
/// tipe object per adapter). Registry GLOBAL (per proses) — pola sama dengan
/// VPI_OBJECTS di vpi/handle.rs, tapi generik dan dipakai VHPI/PLI juga.
pub struct HandleRegistry {
    objects: HashMap<u64, Box<dyn std::any::Any + Send + Sync>>,
    next_id: AtomicU64,
}

impl HandleRegistry {
    pub fn global() -> &'static Mutex<HandleRegistry> {
        static REGISTRY: OnceLock<Mutex<HandleRegistry>> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            Mutex::new(HandleRegistry {
                objects: HashMap::new(),
                next_id: AtomicU64::new(1),
            })
        })
    }

    /// Register object → handle u64. Kind diambil dari `ForeignHandle` caller
    /// (registry tidak tahu tipe object — adapter yang tahu).
    pub fn insert<T: std::any::Any + Send + Sync>(
        &mut self,
        kind: HandleKind,
        obj: T,
    ) -> ForeignHandle {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.objects.insert(id, Box::new(obj));
        ForeignHandle { id, kind }
    }

    /// Ambil object by id (downcast oleh pemanggil).
    pub fn get(&self, id: u64) -> Option<&(dyn std::any::Any + Send + Sync)> {
        self.objects.get(&id).map(|b| b.as_ref())
    }

    /// Hapus object (vpi_free_object / vhpi_release_handle). `true` bila ada.
    pub fn remove(&mut self, id: u64) -> bool {
        self.objects.remove(&id).is_some()
    }

    /// Bersihkan semua (end of simulation).
    pub fn clear(&mut self) {
        self.objects.clear();
    }

    /// Jumlah object aktif (untuk test / leak check).
    pub fn len(&self) -> usize {
        self.objects.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Registry global — serialkan test (pola sama dgn vpi/handle.rs).
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_handle_registry_roundtrip() {
        let _g = TEST_LOCK.lock().unwrap();
        let mut reg = HandleRegistry {
            objects: HashMap::new(),
            next_id: AtomicU64::new(1),
        };
        let h = reg.insert(HandleKind::Signal, 42u32);
        assert!(h.is_valid(), "handle valid");
        assert_eq!(h.kind, HandleKind::Signal);
        // downcast
        let got = reg.get(h.id).expect("object ada");
        assert_eq!(*got.downcast_ref::<u32>().unwrap(), 42);
        // remove
        assert!(reg.remove(h.id), "remove sukses");
        assert!(reg.get(h.id).is_none(), "setelah remove tidak ada");
        assert!(!reg.remove(h.id), "remove ganda → false");
    }

    #[test]
    fn test_foreign_handle_null() {
        assert!(ForeignHandle::NULL.is_null());
        assert!(!ForeignHandle::NULL.is_valid());
        let h = ForeignHandle {
            id: 7,
            kind: HandleKind::Port,
        };
        assert!(!h.is_null());
        assert!(h.is_valid());
    }

    #[test]
    fn test_handle_registry_clear_and_len() {
        let _g = TEST_LOCK.lock().unwrap();
        let mut reg = HandleRegistry {
            objects: HashMap::new(),
            next_id: AtomicU64::new(1),
        };
        for _ in 0..5 {
            reg.insert(HandleKind::Net, String::from("wire"));
        }
        assert_eq!(reg.len(), 5);
        reg.clear();
        assert_eq!(reg.len(), 0);
    }
}
