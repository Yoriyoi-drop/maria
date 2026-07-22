//! Typed arena — type-safe wrapper over BumpArena.
//!
//! Memungkinkan alokasi tipe T tanpa menulis size/align manual.
//! Semua nilai dialokasikan dalam bump arena yang sama.

use super::bump::BumpArena;
use std::marker::PhantomData;
use std::mem;
use std::ops::Deref;
use std::ptr;

/// Type-safe arena untuk tipe T.
///
/// # Examples
///
/// ```
/// use maria::arena::TypedArena;
///
/// #[derive(Debug, PartialEq)]
/// struct Node {
///     value: i32,
///     next: Option<usize>,
/// }
///
/// let arena = TypedArena::new();
/// let node = arena.alloc(Node { value: 42, next: None });
/// assert_eq!(node.value, 42);
/// ```
pub struct TypedArena<T> {
    arena: BumpArena,
    _marker: PhantomData<T>,
}

impl<T> TypedArena<T> {
    /// Create a new typed arena.
    pub fn new() -> Self {
        TypedArena {
            arena: BumpArena::new(),
            _marker: PhantomData,
        }
    }

    /// Create a new typed arena with custom initial chunk size.
    pub fn with_initial_size(size: usize) -> Self {
        TypedArena {
            arena: BumpArena::with_initial_size(size),
            _marker: PhantomData,
        }
    }

    /// Allocate a single value of type T.
    /// Returns a mutable reference valid for the arena's lifetime.
    pub fn alloc(&self, value: T) -> &mut T {
        let ptr = self.arena.alloc(mem::size_of::<T>(), mem::align_of::<T>()) as *mut T;
        unsafe {
            ptr::write(ptr, value);
            &mut *ptr
        }
    }

    /// Allocate an array of `count` uninitialized values.
    pub fn alloc_slice(&self, count: usize) -> &mut [T] {
        if count == 0 {
            return &mut [];
        }
        let ptr = self
            .arena
            .alloc(count * mem::size_of::<T>(), mem::align_of::<T>()) as *mut T;
        unsafe { std::slice::from_raw_parts_mut(ptr, count) }
    }

    /// Allocate and initialize a slice from an iterator.
    pub fn alloc_slice_from_iter(&self, iter: impl IntoIterator<Item = T>) -> &mut [T] {
        let items: Vec<T> = iter.into_iter().collect();
        let count = items.len();
        if count == 0 {
            return &mut [];
        }
        let slice = self.alloc_slice(count);
        for (i, item) in items.into_iter().enumerate() {
            unsafe {
                std::ptr::write(&mut slice[i] as *mut T, item);
            }
        }
        slice
    }

    /// Allocate a slice from an existing slice by cloning.
    pub fn alloc_slice_from_clone(&self, items: &[T]) -> &mut [T]
    where
        T: Clone,
    {
        let count = items.len();
        if count == 0 {
            return &mut [];
        }
        let slice = self.alloc_slice(count);
        for (i, item) in items.iter().enumerate() {
            unsafe {
                std::ptr::write(&mut slice[i] as *mut T, item.clone());
            }
        }
        slice
    }

    /// Number of bytes allocated in this arena.
    pub fn memory_used(&self) -> usize {
        self.arena.memory_used()
    }

    /// Total capacity reserved by this arena.
    pub fn capacity(&self) -> usize {
        self.arena.capacity()
    }

    /// Reset the arena (deallocate all).
    pub fn reset(&mut self) {
        self.arena.reset();
    }
}

impl<T: Default> TypedArena<T> {
    /// Allocate with default value.
    pub fn alloc_default(&self) -> &mut T {
        self.alloc(T::default())
    }
}

impl<T> Default for TypedArena<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Index into arena ───

/// A handle (index) into a TypedArena.
/// Zero-cost: u32 size, Copy semantics.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
#[allow(dead_code)]
pub struct ArenaIdx(pub u32);

impl ArenaIdx {
    pub fn as_usize(self) -> usize {
        self.0 as usize
    }

    pub fn from_usize(idx: usize) -> Self {
        ArenaIdx(idx as u32)
    }
}

/// Wrapper for arena-backed slices.
pub struct ArenaSlice<'a, T> {
    data: &'a [T],
}

impl<'a, T> ArenaSlice<'a, T> {
    pub fn new(data: &'a [T]) -> Self {
        ArenaSlice { data }
    }

    pub fn get(&self, idx: usize) -> &T {
        &self.data[idx]
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'a, T> {
        self.data.iter()
    }
}

impl<'a, T> Deref for ArenaSlice<'a, T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.data
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_typed_alloc() {
        let arena = TypedArena::new();
        let val = arena.alloc(42i32);
        assert_eq!(*val, 42);
    }

    #[test]
    fn test_typed_alloc_struct() {
        #[derive(Debug, PartialEq)]
        struct Point {
            x: i32,
            y: i32,
        }

        let arena = TypedArena::new();
        let p = arena.alloc(Point { x: 10, y: 20 });
        assert_eq!(p.x, 10);
        assert_eq!(p.y, 20);
    }

    #[test]
    fn test_typed_alloc_slice() {
        let arena = TypedArena::<i32>::new();
        let slice = arena.alloc_slice(5);
        assert_eq!(slice.len(), 5);
        slice[0] = 42;
        assert_eq!(slice[0], 42);
    }

    #[test]
    fn test_typed_alloc_slice_empty() {
        let arena = TypedArena::<i32>::new();
        let slice = arena.alloc_slice(0);
        assert!(slice.is_empty());
    }

    #[test]
    fn test_typed_alloc_from_iter() {
        let arena = TypedArena::new();
        let slice = arena.alloc_slice_from_iter(vec![1, 2, 3, 4, 5]);
        assert_eq!(slice.len(), 5);
        assert_eq!(slice[2], 3);
    }
}
