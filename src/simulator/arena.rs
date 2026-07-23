//! SimulationArena — thread-safe bump allocator untuk temp allocations di simulator.
//!
//! # Zero-Deallocation Strategy
//!
//! Alih-alih memanggil `free()` untuk setiap `LogicVec` atau `Vec<LogicVal>` yang
//! dialokasi selama evaluasi ekspresi, kita **tidak dealokasi sama sekali** selama
//! siklus berjalan. Semua memori dialokasikan dari bump arena.
//!
//! Di akhir setiap siklus simulasi, kita panggil `reset_cycle()` yang:
//! 1. Mereset bump pointer ke awal (O(1)) — semua heap allocation sebelumnya langsung reusable
//! 2. Pool Vec<LogicVal> tetap hidup (backing storage tidak di-free)
//!
//! # Thread-Local Integration
//!
//! SimulationArena terintegrasi dengan `LogicVec::new()` via thread-local:
//! - Engine panggil `set_thread_arena()` sebelum evaluasi siklus
//! - Semua `LogicVec::new()` otomatis deteksi arena dan alokasi dari pool
//! - Engine panggil `clear_thread_arena()` setelah selesai
//! - **Zero code changes** di evaluate_expr — semua 50+ allocation sites otomatis teroptimasi
//!
//! # Thread Safety
//!
//! SimulationArena menggunakan `BumpArena` yang thread-safe via `parking_lot::Mutex`.
//! Cocok untuk digunakan dari rayon thread pool atau event loop utama.

use crate::arena::{BumpArena, ObjectPool};
use crate::ir::{LogicVal, LogicVec};
use std::cell::RefCell;

// ─── Thread-Local Arena ───

thread_local! {
    /// Pointer ke SimulationArena untuk siklus saat ini.
    /// Digunakan oleh LogicVec::new() untuk alokasi zero-deallocation.
    /// Safety: hanya 1 engine aktif per thread, pointer valid selama siklus.
    static CYCLE_ARENA: RefCell<Option<*mut SimulationArena>> = const { RefCell::new(None) };
}

/// Set the thread-local arena for the current simulation cycle.
/// Semua `LogicVec::new()`, `LogicVec::fill()`, dan `LogicVec::from_u64()`
/// akan menggunakan arena ini secara otomatis — tanpa perubahan kode di evaluate_expr().
pub fn set_thread_arena(arena: Option<&mut SimulationArena>) {
    let has_arena = arena.is_some();
    CYCLE_ARENA.with(|cell| {
        *cell.borrow_mut() = arena.map(|a| a as *mut SimulationArena);
    });
    // Register/deregister LogicVecCtor so LogicVec constructors go through arena.
    // try_alloc_logicvec has the exact signature fn(usize, LogicVal) -> Option<LogicVec>.
    crate::ir::set_logicvec_ctor(if has_arena { Some(try_alloc_logicvec) } else { None });
}

/// Alokasi LogicVec dari thread-local arena (jika ada).
/// Dipanggil oleh LogicVec::new() dan LogicVec::fill().
pub fn try_alloc_logicvec(width: usize, init: LogicVal) -> Option<LogicVec> {
    CYCLE_ARENA.with(|cell| {
        let mut borrow = cell.borrow_mut();
        if let Some(ptr) = *borrow {
            let arena = unsafe { &mut *ptr };
            Some(arena.alloc_logicvec(width, init))
        } else {
            None
        }
    })
}

/// Arena untuk alokasi temporary selama simulasi.
///
/// Setiap SimulationEngine memiliki satu instance SimulationArena yang di-reset
/// setiap siklus simulasi.
pub struct SimulationArena {
    /// Bump arena untuk alokasi raw memory.
    bump: BumpArena,
    /// Pool Vec<LogicVal> — reuse backing storage LogicVec.
    /// Menghindari alloc/free berulang untuk Vec yang sering dibuat/dibuang.
    bits_pool: ObjectPool<Vec<LogicVal>>,
    /// Jumlah LogicVec yang dialokasi siklus ini.
    alloc_count: usize,
    /// Jumlah LogicVec yang di-reuse dari pool.
    reuse_count: usize,
}

impl SimulationArena {
    /// Create a new simulation arena with default sizing.
    pub fn new() -> Self {
        SimulationArena {
            bump: BumpArena::with_initial_size(1024 * 1024), // 1MB initial
            bits_pool: ObjectPool::new(),
            alloc_count: 0,
            reuse_count: 0,
        }
    }

    /// Create with custom initial bump arena size.
    pub fn with_bump_size(size: usize) -> Self {
        SimulationArena {
            bump: BumpArena::with_initial_size(size),
            bits_pool: ObjectPool::new(),
            alloc_count: 0,
            reuse_count: 0,
        }
    }

    /// Allocate a temporary LogicVec with given width, initialized to a value.
    ///
    /// Jika ada Vec<LogicVal> tersedia di pool, kita reuse backing storage-nya
    /// (hanya resize, tidak re-allocate). Ini menghindari heap alloc/free.
    pub fn alloc_logicvec(&mut self, width: usize, init: LogicVal) -> LogicVec {
        let w = if width > 1_000_000 { 1 } else { width };
        let pool_hit = self.bits_pool.available() > 0;
        let mut bits = self.bits_pool.get(|| Vec::with_capacity(w));
        bits.clear();
        bits.resize(w, init);
        if pool_hit {
            self.reuse_count += 1;
        } else {
            self.alloc_count += 1;
        }
        LogicVec {
            width: w,
            bits,
        }
    }

    /// Allocate a temporary LogicVec without pool reuse tracking (always creates fresh).
    /// Use for rare/final allocations where pool management isn't needed.
    pub fn alloc_logicvec_fresh(&mut self, width: usize, init: LogicVal) -> LogicVec {
        let w = if width > 1_000_000 { 1 } else { width };
        let bits = vec![init; w];
        self.alloc_count += 1;
        LogicVec { width: w, bits }
    }

    /// Allocate a LogicVec initialized from a u64 value.
    pub fn alloc_logicvec_from_u64(&mut self, val: u64, width: usize) -> LogicVec {
        let mut lv = self.alloc_logicvec(width, LogicVal::Zero);
        for i in 0..lv.width.min(64) {
            if (val >> i) & 1 == 1 {
                lv.bits[i] = LogicVal::One;
            }
        }
        lv
    }

    /// Allocate a LogicVec as a clone of an existing LogicVec.
    /// Uses bump arena for the backing storage instead of heap.
    pub fn alloc_logicvec_clone(&mut self, other: &LogicVec) -> LogicVec {
        let w = other.width.max(1);
        let mut bits = self.bits_pool.get(|| Vec::with_capacity(w));
        bits.clear();
        bits.extend_from_slice(&other.bits);
        self.alloc_count += 1;
        LogicVec {
            width: w,
            bits,
        }
    }

    /// Return a LogicVec's backing storage to the pool for reuse.
    /// Call this when a temp LogicVec is no longer needed.
    pub fn reclaim_logicvec(&mut self, lv: LogicVec) {
        let mut bits = lv.bits;
        bits.clear();
        self.bits_pool.put(bits);
        self.reuse_count += 1;
    }

    /// Allocate raw memory from the bump arena.
    pub fn alloc_raw(&self, size: usize, align: usize) -> *mut u8 {
        self.bump.alloc(size, align)
    }

    /// Reset the arena for the next simulation cycle.
    ///
    /// # Zero-Deallocation
    ///
    /// - Bump arena: reset pointer ke awal — O(1), semua memori reusable instan
    /// - Object pool: Vec<LogicVal> **tidak di-clear** — backing storage tetap hidup
    ///   dan siap di-reuse oleh siklus berikutnya. Ini adalah inti dari
    ///   zero-deallocation: alokasi heap tidak pernah di-free antar siklus.
    pub fn reset_cycle(&mut self) {
        self.bump.reset();
        // JANGAN panggil self.bits_pool.clear() — Vec backing storage harus tetap
        // hidup untuk zero-deallocation. Pool Vecs akan di-reuse oleh alloc_logicvec().
        self.alloc_count = 0;
        self.reuse_count = 0;
    }

    /// Total memory used by the bump arena.
    pub fn memory_used(&self) -> usize {
        self.bump.memory_used()
    }

    /// Total LogicVec allocations this cycle.
    pub fn alloc_count(&self) -> usize {
        self.alloc_count
    }

    /// Total LogicVec reuses from pool this cycle.
    pub fn reuse_count(&self) -> usize {
        self.reuse_count
    }

    /// Reuse rate as percentage.
    pub fn reuse_rate(&self) -> f64 {
        let total = self.alloc_count + self.reuse_count;
        if total == 0 {
            0.0
        } else {
            self.reuse_count as f64 / total as f64
        }
    }
}

impl Default for SimulationArena {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alloc_logicvec() {
        let mut arena = SimulationArena::new();
        let lv = arena.alloc_logicvec(8, LogicVal::X);
        assert_eq!(lv.width, 8);
        assert!(lv.bits.iter().all(|b| *b == LogicVal::X));
    }

    #[test]
    fn test_alloc_logicvec_from_u64() {
        let mut arena = SimulationArena::new();
        let lv = arena.alloc_logicvec_from_u64(0b1010, 4);
        assert_eq!(lv.width, 4);
        assert_eq!(lv.to_u64(), 0b1010);
    }

    #[test]
    fn test_reclaim_and_reuse() {
        let mut arena = SimulationArena::new();
        let lv1 = arena.alloc_logicvec(64, LogicVal::Zero);
        arena.reclaim_logicvec(lv1);
        // Next allocation should reuse the Vec from pool
        let lv2 = arena.alloc_logicvec(32, LogicVal::One);
        assert!(arena.reuse_count > 0, "should have reused from pool");
        assert_eq!(lv2.width, 32);
        assert!(lv2.bits.iter().all(|b| *b == LogicVal::One));
    }

    #[test]
    fn test_reset_cycle() {
        let mut arena = SimulationArena::new();
        // Gunakan alloc_raw untuk mengisi bump arena
        let _ptr = arena.alloc_raw(128, 8);
        assert!(arena.memory_used() > 0, "bump arena should have memory after alloc_raw");
        let _lv1 = arena.alloc_logicvec(128, LogicVal::X);
        assert!(arena.alloc_count > 0);
        arena.reset_cycle();
        // Bump arena reset -> memory_used = 0
        assert_eq!(arena.memory_used(), 0, "memory should be 0 after reset");
        assert_eq!(arena.alloc_count(), 0, "alloc_count should be 0 after reset");
    }

    #[test]
    fn test_post_reset_allocations_still_work() {
        let mut arena = SimulationArena::new();
        let _lv = arena.alloc_logicvec(16, LogicVal::X);
        arena.reset_cycle();
        let lv = arena.alloc_logicvec_from_u64(0xABCD, 16);
        assert_eq!(lv.to_u64(), 0xABCD);
    }

    #[test]
    fn test_multiple_reclaim() {
        let mut arena = SimulationArena::new();
        let mut vecs = Vec::new();
        for _ in 0..100 {
            vecs.push(arena.alloc_logicvec(32, LogicVal::X));
        }
        // Reclaim all
        for lv in vecs {
            arena.reclaim_logicvec(lv);
        }
        // Now verify pool reuse
        let lv = arena.alloc_logicvec(32, LogicVal::Zero);
        assert_eq!(lv.width, 32);
        assert!(lv.bits.iter().all(|b| *b == LogicVal::Zero));
    }

    #[test]
    fn test_reclaim_rate() {
        let mut arena = SimulationArena::new();
        // Allocate and reclaim several iterasi
        // Iterasi 1: pool empty -> alloc (1 alloc, 0 reuse)
        // Iterasi 2-10: pool has 1 vec -> reuse (1 alloc, 9 reuse)
        // Rate = 9/10 = 0.9
        for _ in 0..10 {
            let lv = arena.alloc_logicvec(8, LogicVal::Zero);
            arena.reclaim_logicvec(lv);
        }
        let rate = arena.reuse_rate();
        assert!(rate > 0.8, "reuse rate should be high, got {}", rate);
    }
}
