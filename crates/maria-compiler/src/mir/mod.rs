//! Mid-Level IR — simulation-optimized intermediate representation.

pub mod mir;
pub mod lower;
pub mod opt;
#[cfg(feature = "jit")]
pub mod jit;

pub use mir::*;
pub use lower::lower_module;
pub use opt::optimize_module;

// MirJitCompiler fallback when jit feature is disabled
#[cfg(feature = "jit")]
pub use jit::MirJitCompiler;

/// MirJitCompiler fallback — digunakan saat `jit` feature tidak aktif.
/// Semua method no-op, engine tetap berfungsi tanpa compiled-code simulation.
#[cfg(not(feature = "jit"))]
pub struct MirJitCompiler;

/// CompiledMirProcess fallback — dummy type untuk mir.jit module when feature is off.
#[cfg(not(feature = "jit"))]
pub struct CompiledMirProcess {
    pub code_ptr: *const u8,
}

#[cfg(not(feature = "jit"))]
impl MirJitCompiler {
    pub fn new() -> Option<Self> { None }
    pub fn compile_process(&mut self, _process: &crate::mir::MirProcess, _n_sigs: usize) -> Option<CompiledMirProcess> { None }
    pub unsafe fn call_process(_code_ptr: *const u8, _signals: &[u64], _out: &mut [u64]) {}
}
