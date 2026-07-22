//! Object Pool — reusable object pool untuk mengurangi alokasi.
//!
//! Objects yang sering di-allocate/deallocate bisa di-reuse via pool.

use std::cell::UnsafeCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Thread-safe object pool with free list.
pub struct ObjectPool<T> {
    free_list: UnsafeCell<VecDeque<T>>,
    total_allocated: AtomicUsize,
    total_reused: AtomicUsize,
}

unsafe impl<T: Send> Send for ObjectPool<T> {}
unsafe impl<T: Send + Sync> Sync for ObjectPool<T> {}

impl<T> ObjectPool<T> {
    pub fn new() -> Self {
        ObjectPool {
            free_list: UnsafeCell::new(VecDeque::new()),
            total_allocated: AtomicUsize::new(0),
            total_reused: AtomicUsize::new(0),
        }
    }

    /// Create a pool with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        ObjectPool {
            free_list: UnsafeCell::new(VecDeque::with_capacity(capacity)),
            total_allocated: AtomicUsize::new(0),
            total_reused: AtomicUsize::new(0),
        }
    }

    /// Get an object from the pool (or create new).
    /// Factory is called only if pool is empty.
    pub fn get(&self, factory: impl FnOnce() -> T) -> T {
        let free_list = unsafe { &mut *self.free_list.get() };
        if let Some(obj) = free_list.pop_front() {
            self.total_reused.fetch_add(1, Ordering::Relaxed);
            obj
        } else {
            self.total_allocated.fetch_add(1, Ordering::Relaxed);
            factory()
        }
    }

    /// Return an object to the pool for reuse.
    pub fn put(&self, obj: T) {
        let free_list = unsafe { &mut *self.free_list.get() };
        free_list.push_back(obj);
    }

    /// Number of objects currently in the pool.
    pub fn available(&self) -> usize {
        let free_list = unsafe { &*self.free_list.get() };
        free_list.len()
    }

    /// Total objects allocated (not from pool).
    pub fn total_allocated(&self) -> usize {
        self.total_allocated.load(Ordering::Relaxed)
    }

    /// Total objects reused from pool.
    pub fn total_reused(&self) -> usize {
        self.total_reused.load(Ordering::Relaxed)
    }

    /// Reuse rate: reused / (allocated + reused).
    pub fn reuse_rate(&self) -> f64 {
        let allocated = self.total_allocated() as f64;
        let reused = self.total_reused() as f64;
        let total = allocated + reused;
        if total == 0.0 {
            0.0
        } else {
            reused / total
        }
    }

    /// Clear the pool.
    pub fn clear(&self) {
        let free_list = unsafe { &mut *self.free_list.get() };
        free_list.clear();
    }
}

impl<T> Default for ObjectPool<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_object_pool_basic() {
        let pool = ObjectPool::new();
        let obj = pool.get(|| vec![1, 2, 3]);
        assert_eq!(obj, vec![1, 2, 3]);
        pool.put(obj);
        assert_eq!(pool.available(), 1);
    }

    #[test]
    fn test_object_pool_reuse() {
        let pool = ObjectPool::new();
        let obj = pool.get(|| vec![0u8; 1024]);
        pool.put(obj);

        let obj2 = pool.get(|| vec![0u8; 1024]);
        assert!(pool.total_reused() > 0);
        pool.put(obj2);
    }

    #[test]
    fn test_object_pool_reuse_rate() {
        let pool = ObjectPool::new();
        // Pre-fill pool
        for _ in 0..5 {
            pool.put(vec![0u8]);
        }
        for _ in 0..20 {
            let obj = pool.get(|| vec![0u8]);
            pool.put(obj);
        }
        assert!(pool.reuse_rate() > 0.8);
    }
}
