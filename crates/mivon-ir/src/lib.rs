pub mod ir;

pub use ir::*;

// Re-export tipe nilai logika dari mivon-core agar `mivon_ir::LogicVec` /
// `use mivon_ir::*` (glob) tetap menyediakan LogicVec/LogicVal seperti dulu.
pub use mivon_core::{LogicVal, LogicVec};
