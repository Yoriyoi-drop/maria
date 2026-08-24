//! Maria FFI — Unified Foreign Interface (arsitektur masukan user).
//!
//! VPI/VHPI/PLI/DPI adalah ADAPTER di atas simulation kernel Maria, bukan
//! implementasi event scheduler masing-masing. Semua foreign callback masuk
//! ke satu event queue (`ForeignEvent`) yang diproses oleh scheduler utama —
//! library eksternal tidak pernah menyentuh scheduler Maria secara langsung.
//!
//! ```text
//!              Maria FFI
//!                 │
//!       ┌─────────┼─────────┐
//!       │         │         │
//!      VPI       VHPI      PLI
//!       │         │         │
//!       └─────────┼─────────┘
//!                 │
//!           Maria Sim API
//!                 │
//!        ┌────────┼────────┐
//!        │        │        │
//!      Handle   Value    Callback
//!        │        │        │
//!        └────────┼────────┘
//!                 │
//!            Event Queue
//! ```
//!
//! Prinsip: Maria tidak perlu "menjadi" VPI/VHPI/PLI secara internal — Maria
//! menyediakan ABI-compatible adapter di atas simulation kernel, sehingga
//! library eksternal menganggap dirinya berbicara dengan simulator yang
//! kompatibel.

pub mod handle;
pub mod loader;

pub use handle::{ForeignHandle, HandleKind, HandleRegistry};

/// RAII guard: deregistrasi engine VPI/VHPI saat drop.
/// Dipasang di awal `SimulationEngine::run()` agar SEMUA path keluar
/// (normal, early-return error, panic unwind) meninggalkan registry bersih —
/// tanpa ini, pointer ke engine yang sudah drop bisa tertinggal dan
/// diderefsimulasi berikutnya (SIGSEGV saat test paralel).
pub struct ForeignEngineGuard;

impl Drop for ForeignEngineGuard {
    fn drop(&mut self) {
        crate::vpi::clear_vpi_engine();
        crate::vhpi::object::clear_vhpi_engine();
    }
}

/// Event foreign yang masuk ke antrian scheduler utama (bukan dijalankan
/// langsung dari thread library). Setiap varian dipetakan ke region
/// scheduler IEEE 1800 yang sesuai.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignEvent {
    /// Signal berubah nilai → callback value-change (active/NBA region).
    ValueChange {
        /// Object handle (u64 id — tidak pernah pointer internal Maria).
        object: u64,
    },
    /// Batas region ReadWriteSynch (setelah NBA).
    ReadWriteSync,
    /// Batas region ReadOnlySynch (sinkronisasi baca, sebelum time advance).
    ReadOnlySync,
    /// Awal time step berikutnya (NextTimeStep).
    NextTimeStep,
    /// Callback terdaftar (mis. after-delay) siap dieksekusi.
    Callback { callback_id: u64 },
    /// End of simulation (vhpiCbEndOfSimulation / cbEndOfSimulation).
    EndOfSimulation,
}

/// Sumber foreign event — dipakai scheduler untuk memilih adapter yang tepat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignKind {
    Vpi,
    Vhpi,
    Pli,
    Dpi,
}
