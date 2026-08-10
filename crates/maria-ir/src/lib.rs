pub mod ir;

pub use ir::*;

// Re-export tipe nilai logika dari maria-core agar `maria_ir::LogicVec` /
// `use maria_ir::*` (glob) tetap menyediakan LogicVec/LogicVal seperti dulu.
pub use maria_core::{LogicVal, LogicVec};
