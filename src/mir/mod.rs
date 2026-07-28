//! Mid-Level IR — simulation-optimized intermediate representation.

pub mod mir;
pub mod lower;
pub mod opt;
pub mod jit;

pub use mir::*;
pub use lower::lower_module;
pub use opt::optimize_module;
pub use jit::MirJitCompiler;
