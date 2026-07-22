//! Bump (Arena) allocator.
//!
//! Mengalokasikan memori dengan menaikkan pointer — O(1) allocation.
//! Chunk tumbuh secara eksponensial: 64KB → 128KB → 256KB → ... → 16MB.
//! Dealokasi: reset seluruh arena (drop semua chunk) — O(1).
//!
//! Thread safety: menggunakan `parking_lot::Mutex` untuk akses ke chunk list.

use std::alloc::{alloc, Layout};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};

// ─── Constants ───

const INITIAL_CHUNK_SIZE: usize = 64 * 1024; // 64KB
const MAX_CHUNK_SIZE: usize = 16 * 1024 * 1024; // 16MB
const DEFAULT_ALIGN: usize = 8; // Default alignment

// ─── Chunk ───

struct Chunk {
    ptr: NonNull<u8>,
    capacity: usize,
    used: AtomicUsize,
}

impl Chunk {
    fn new(size: usize) -> Self {
        let layout =
            Layout::from_size_align(size, DEFAULT_ALIGN).expect("invalid layout for arena chunk");
        let ptr = unsafe { alloc(layout) };
        let ptr = NonNull::new(ptr).expect("arena chunk allocation failed");
        Chunk {
            ptr,
            capacity: size,
            used: AtomicUsize::new(0),
        }
    }

    /// Allocate `size` bytes with given alignment from this chunk.
    /// Returns None if chunk is full.
    fn alloc(&self, size: usize, align: usize) -> Option<NonNull<u8>> {
        loop {
            let current = self.used.load(Ordering::Acquire);
            let start = align_up(self.ptr.as_ptr() as usize + current, align);
            let new_used = (start - self.ptr.as_ptr() as usize) + size;

            if new_used > self.capacity {
                return None;
            }

            if self
                .used
                .compare_exchange_weak(current, new_used, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                // Zero out the allocated memory (safety for Copy types)
                unsafe {
                    std::ptr::write_bytes(start as *mut u8, 0, size);
                }
                return Some(unsafe { NonNull::new_unchecked(start as *mut u8) });
            }
            // CAS failed, retry
        }
    }
}

// Chunk owns heap memory — safe to Send between threads since protected by Mutex.
unsafe impl Send for Chunk {}

impl Drop for Chunk {
    fn drop(&mut self) {
        let layout = Layout::from_size_align(self.capacity, DEFAULT_ALIGN).expect("invalid layout");
        unsafe {
            std::alloc::dealloc(self.ptr.as_ptr(), layout);
        }
    }
}

// ─── BumpArena ───

/// Bump allocator dengan chunk doubling.
///
/// Thread-safe: menggunakan `parking_lot::Mutex` untuk akses concurrent.
///
/// # Examples
///
/// ```
/// use maria::arena::BumpArena;
///
/// let arena = BumpArena::new();
/// let ptr = arena.alloc(32, 8);
/// assert!(!ptr.is_null());
/// ```
pub struct BumpArena {
    /// Current chunk + full chunks, protected by Mutex
    inner: parking_lot::Mutex<ArenaInner>,
    /// Next chunk size (grows exponentially) — atomic for lock-free read
    next_chunk_size: AtomicUsize,
}

struct ArenaInner {
    /// Current chunk being allocated from
    current: Option<Chunk>,
    /// List of full (retired) chunks
    chunks: Vec<Chunk>,
}

impl BumpArena {
    /// Create a new bump arena.
    pub fn new() -> Self {
        BumpArena {
            inner: parking_lot::Mutex::new(ArenaInner {
                current: None,
                chunks: Vec::new(),
            }),
            next_chunk_size: AtomicUsize::new(INITIAL_CHUNK_SIZE),
        }
    }

    /// Create a new bump arena with a specific initial chunk size.
    pub fn with_initial_size(size: usize) -> Self {
        let inner = ArenaInner {
            current: Some(Chunk::new(size)),
            chunks: Vec::new(),
        };
        BumpArena {
            inner: parking_lot::Mutex::new(inner),
            next_chunk_size: AtomicUsize::new(next_chunk_size(size)),
        }
    }

    /// Allocate `size` bytes with given alignment.
    /// Returns a raw pointer to the allocated memory.
    pub fn alloc(&self, size: usize, align: usize) -> *mut u8 {
        // Ensure minimum size and alignment
        let size = size.max(1);
        let align = align.max(DEFAULT_ALIGN);

        // Try current chunk first — fast path via lock
        let mut inner = self.inner.lock();
        if let Some(ref chunk) = inner.current {
            if let Some(ptr) = chunk.alloc(size, align) {
                return ptr.as_ptr();
            }
            // Current chunk is full — retire it
            let old_chunk = inner.current.take();
            if let Some(old) = old_chunk {
                inner.chunks.push(old);
            }
        }

        // Allocate new chunk — ensure it's at least as large as the request
        let chunk_size = self.next_chunk_size().max(size + align);
        let new_chunk = Chunk::new(chunk_size);
        let ptr = new_chunk
            .alloc(size, align)
            .expect("new chunk must have enough space");

        inner.current = Some(new_chunk);
        ptr.as_ptr()
    }

    /// Allocate and initialize a value of type T.
    pub fn alloc_value<T>(&self, val: T) -> &mut T {
        let ptr = self.alloc(std::mem::size_of::<T>(), std::mem::align_of::<T>()) as *mut T;
        unsafe {
            std::ptr::write(ptr, val);
            &mut *ptr
        }
    }

    /// Allocate a slice of `count` uninitialized elements.
    pub fn alloc_slice<T>(&self, count: usize) -> &mut [T] {
        if count == 0 {
            return &mut [];
        }
        let size = count * std::mem::size_of::<T>();
        let align = std::mem::align_of::<T>();
        let ptr = self.alloc(size, align) as *mut T;
        unsafe { std::slice::from_raw_parts_mut(ptr, count) }
    }

    /// Allocate and initialize a slice from an iterator.
    pub fn alloc_slice_from_iter<T>(&self, iter: impl IntoIterator<Item = T>) -> &mut [T] {
        let items: Vec<T> = iter.into_iter().collect();
        let count = items.len();
        if count == 0 {
            return &mut [];
        }
        let slice = self.alloc_slice::<T>(count);
        for (i, item) in items.into_iter().enumerate() {
            unsafe {
                std::ptr::write(&mut slice[i] as *mut T, item);
            }
        }
        slice
    }

    /// Reset the arena — deallocate all chunks.
    /// All previously allocated pointers become invalid.
    pub fn reset(&mut self) {
        let mut inner = self.inner.lock();
        inner.current = None;
        inner.chunks.clear();
        self.next_chunk_size
            .store(INITIAL_CHUNK_SIZE, Ordering::Release);
    }

    /// Current memory usage in bytes.
    pub fn memory_used(&self) -> usize {
        let inner = self.inner.lock();
        let mut total = 0usize;
        if let Some(ref chunk) = inner.current {
            total += chunk.used.load(Ordering::Acquire);
        }
        for chunk in &inner.chunks {
            total += chunk.used.load(Ordering::Acquire);
        }
        total
    }

    /// Total allocated capacity in bytes.
    pub fn capacity(&self) -> usize {
        let inner = self.inner.lock();
        let mut total = 0usize;
        if let Some(ref chunk) = inner.current {
            total += chunk.capacity;
        }
        for chunk in &inner.chunks {
            total += chunk.capacity;
        }
        total
    }

    fn next_chunk_size(&self) -> usize {
        let size = self.next_chunk_size.load(Ordering::Acquire);
        let next = next_chunk_size(size);
        self.next_chunk_size.store(next, Ordering::Release);
        size
    }
}

fn next_chunk_size(current: usize) -> usize {
    (current * 2).min(MAX_CHUNK_SIZE)
}

fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

impl Default for BumpArena {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alloc_basic() {
        let arena = BumpArena::new();
        let ptr = arena.alloc(16, 8);
        assert!(!ptr.is_null());
        unsafe {
            std::ptr::write(ptr as *mut u64, 42u64);
            assert_eq!(*(ptr as *mut u64), 42);
        }
    }

    #[test]
    fn test_alloc_many() {
        let arena = BumpArena::new();
        let mut ptrs = Vec::new();
        for i in 0..1000 {
            let ptr = arena.alloc(64, 8);
            assert!(!ptr.is_null());
            unsafe {
                *(ptr as *mut u64) = i as u64;
            }
            ptrs.push(ptr);
        }
        // Verify all values
        for (i, &ptr) in ptrs.iter().enumerate() {
            unsafe {
                assert_eq!(*(ptr as *mut u64), i as u64);
            }
        }
    }

    #[test]
    fn test_alloc_value() {
        let arena = BumpArena::new();
        let val = arena.alloc_value(42u64);
        assert_eq!(*val, 42);
    }

    #[test]
    fn test_alloc_slice() {
        let arena = BumpArena::new();
        let slice = arena.alloc_slice::<u64>(10);
        assert_eq!(slice.len(), 10);
        for (i, item) in slice.iter_mut().enumerate() {
            *item = i as u64;
        }
        for i in 0..10 {
            assert_eq!(slice[i], i as u64);
        }
    }

    #[test]
    fn test_alloc_slice_empty() {
        let arena = BumpArena::new();
        let slice = arena.alloc_slice::<u64>(0);
        assert!(slice.is_empty());
    }

    #[test]
    fn test_reset() {
        let mut arena = BumpArena::new();
        let _ptr1 = arena.alloc(1024, 8);
        let used_before = arena.memory_used();
        assert!(used_before > 0);
        arena.reset();
        assert_eq!(arena.memory_used(), 0);
    }

    #[test]
    fn test_chunk_growth() {
        let arena = BumpArena::new();
        let initial = arena.capacity();
        // Allocate more than initial chunk
        let big_size = INITIAL_CHUNK_SIZE + 1;
        arena.alloc(big_size, 8);
        let after = arena.capacity();
        assert!(
            after > initial,
            "capacity should grow: {} <= {}",
            after,
            initial
        );
    }

    #[test]
    fn test_thread_safety() {
        let arena = BumpArena::new();
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| {
                    for i in 0..100 {
                        let ptr = arena.alloc(64, 8);
                        assert!(!ptr.is_null());
                        unsafe {
                            *(ptr as *mut u64) = i as u64;
                        }
                    }
                });
            }
        });
        // Verify total allocations: 8 threads * 100 allocs * 64 bytes each
        assert_eq!(arena.memory_used(), 8 * 100 * 64);
    }
}
