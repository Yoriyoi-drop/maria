//! Slab allocator — untuk object reuse dengan fixed-size.
//!
//! Berguna untuk node AST kecil atau token yang sering dialokasikan/dealokasi.
//! Menggunakan Vec-backed free list untuk reuse slot.

use std::cell::UnsafeCell;
use std::fmt;
use std::marker::PhantomData;

/// A slab-allocated index handle.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct Slot(pub u32);

/// Pre-allocated slab of objects.
///
/// Growing: double kapasitas saat penuh.
/// Tidak pernah mengecil — cocok untuk arena-style allocation.
pub struct Slab<T> {
    entries: UnsafeCell<Vec<Entry<T>>>,
    free_head: UnsafeCell<Option<u32>>,
    len: UnsafeCell<usize>,
    _marker: PhantomData<T>,
}

enum Entry<T> {
    Occupied(T),
    Empty { next_free: Option<u32> },
}

unsafe impl<T: Send> Send for Slab<T> {}
unsafe impl<T: Sync> Sync for Slab<T> {}

impl<T> Slab<T> {
    /// Create a new empty slab.
    pub fn new() -> Self {
        Slab {
            entries: UnsafeCell::new(Vec::new()),
            free_head: UnsafeCell::new(None),
            len: UnsafeCell::new(0),
            _marker: PhantomData,
        }
    }

    /// Create a slab with pre-allocated capacity.
    pub fn with_capacity(cap: usize) -> Self {
        let mut entries = Vec::with_capacity(cap);
        // Initialize free list
        for i in 0..cap {
            let next = if i + 1 < cap {
                Some(i as u32 + 1)
            } else {
                None
            };
            entries.push(Entry::Empty { next_free: next });
        }

        Slab {
            entries: UnsafeCell::new(entries),
            free_head: UnsafeCell::new(if cap > 0 { Some(0) } else { None }),
            len: UnsafeCell::new(0),
            _marker: PhantomData,
        }
    }

    /// Insert a value into the slab, returning its slot index.
    pub fn insert(&self, value: T) -> Slot {
        unsafe {
            // Check free list
            if let Some(free) = *self.free_head.get() {
                let entries = &mut *self.entries.get();
                let slot = free as usize;
                match &entries[slot] {
                    Entry::Empty { next_free } => {
                        *self.free_head.get() = *next_free;
                    }
                    _ => panic!("slab: free list corrupted"),
                }
                entries[slot] = Entry::Occupied(value);
                *self.len.get() += 1;
                return Slot(free);
            }

            // Grow the slab
            let entries = &mut *self.entries.get();
            let slot = entries.len();
            entries.push(Entry::Occupied(value));
            *self.len.get() += 1;
            Slot(slot as u32)
        }
    }

    /// Get a reference to the value at the given slot.
    pub fn get(&self, slot: Slot) -> Option<&T> {
        unsafe {
            let entries = &*self.entries.get();
            match entries.get(slot.0 as usize)? {
                Entry::Occupied(val) => Some(val),
                Entry::Empty { .. } => None,
            }
        }
    }

    /// Get a mutable reference to the value at the given slot.
    pub fn get_mut(&self, slot: Slot) -> Option<&mut T> {
        unsafe {
            let entries = &mut *self.entries.get();
            match entries.get_mut(slot.0 as usize)? {
                Entry::Occupied(val) => Some(val),
                Entry::Empty { .. } => None,
            }
        }
    }

    /// Remove a value from the slab, returning it.
    pub fn remove(&self, slot: Slot) -> Option<T> {
        unsafe {
            let entries = &mut *self.entries.get();
            let idx = slot.0 as usize;
            if idx >= entries.len() {
                return None;
            }

            // Swap with empty entry
            let old = std::mem::replace(
                &mut entries[idx],
                Entry::Empty {
                    next_free: *self.free_head.get(),
                },
            );

            match old {
                Entry::Occupied(val) => {
                    *self.free_head.get() = Some(slot.0);
                    *self.len.get() -= 1;
                    Some(val)
                }
                Entry::Empty { .. } => None,
            }
        }
    }

    /// Number of active entries.
    pub fn len(&self) -> usize {
        unsafe { *self.len.get() }
    }

    /// Whether the slab is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Total capacity (occupied + free slots).
    pub fn capacity(&self) -> usize {
        unsafe { (*self.entries.get()).len() }
    }

    /// Iterate over all occupied entries.
    pub fn iter(&self) -> SlabIter<'_, T> {
        SlabIter { slab: self, pos: 0 }
    }
}

impl<T> Default for Slab<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: fmt::Debug> fmt::Debug for Slab<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slab")
            .field("len", &self.len())
            .field("capacity", &self.capacity())
            .finish()
    }
}

/// Iterator over slab entries.
pub struct SlabIter<'a, T> {
    slab: &'a Slab<T>,
    pos: usize,
}

impl<'a, T> Iterator for SlabIter<'a, T> {
    type Item = (Slot, &'a T);

    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            let entries = &*self.slab.entries.get();
            while self.pos < entries.len() {
                let idx = self.pos;
                self.pos += 1;
                if let Entry::Occupied(val) = &entries[idx] {
                    return Some((Slot(idx as u32), val));
                }
            }
            None
        }
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slab_insert_get() {
        let slab = Slab::new();
        let s1 = slab.insert(42);
        let s2 = slab.insert(100);

        assert_eq!(*slab.get(s1).unwrap(), 42);
        assert_eq!(*slab.get(s2).unwrap(), 100);
    }

    #[test]
    fn test_slab_remove() {
        let slab = Slab::new();
        let s1 = slab.insert(42);
        assert_eq!(slab.remove(s1), Some(42));
        assert!(slab.get(s1).is_none());
    }

    #[test]
    fn test_slab_reuse() {
        let slab = Slab::new();
        let s1 = slab.insert(42);
        assert_eq!(slab.remove(s1), Some(42));
        // Slot should be reused
        let s2 = slab.insert(100);
        assert_eq!(s1, s2, "slot should be reused");
        assert_eq!(*slab.get(s2).unwrap(), 100);
    }

    #[test]
    fn test_slab_iter() {
        let slab = Slab::new();
        slab.insert(1);
        slab.insert(2);
        slab.insert(3);

        let items: Vec<i32> = slab.iter().map(|(_, v)| *v).collect();
        assert_eq!(items, vec![1, 2, 3]);
    }

    #[test]
    fn test_slab_len() {
        let slab = Slab::new();
        assert_eq!(slab.len(), 0);
        let s = slab.insert(42);
        assert_eq!(slab.len(), 1);
        slab.remove(s);
        assert_eq!(slab.len(), 0);
    }
}
