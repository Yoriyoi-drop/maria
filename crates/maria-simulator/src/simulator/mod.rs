pub mod arena;
pub mod checkpoint;
pub mod cosim;
pub mod distributed;
#[cfg(feature = "dpi")]
pub mod dpi;
pub mod liberty;
pub mod upf;
pub mod signal_history;
pub mod coverage_db;
pub mod engine;
pub mod jit;
pub mod packed;
pub mod packed_eval;
pub mod parallel;
pub mod simd_packed;
pub mod sdf;
pub mod state;
pub mod types;
pub mod util;
pub mod value;
#[cfg(feature = "jit")]
pub mod jit_cranelift;
#[cfg(feature = "jit")]
pub mod jit_eval;

pub use engine::*;
pub use jit::*;
pub use packed::*;
pub use sdf::*;
pub use state::*;
pub use types::*;
pub use util::*;
pub use value::*;
#[cfg(feature = "jit")]
pub use jit_cranelift::*;
#[cfg(feature = "jit")]
pub use jit_eval::*;

/// JIT Evaluator fallback — digunakan saat `jit` feature tidak aktif.
/// Semua method no-op, engine tetap berfungsi tanpa native compilation.
#[cfg(not(feature = "jit"))]
pub struct JITEvaluator;

#[cfg(not(feature = "jit"))]
impl JITEvaluator {
    pub fn new() -> Self { Self }
    pub fn stats(&self) -> (u64, u64, f64) { (0, 0, 0.0) }
    pub fn is_available(&self) -> bool { false }
    pub fn compiled_count(&self) -> usize { 0 }
    pub fn cache_hit_rate(&self) -> f64 { 0.0 }
    pub fn eval_binary(&mut self, _op: &maria_ir::BinaryIrOp, _lhs: &maria_ir::LogicVec, _rhs: &maria_ir::LogicVec) -> Option<maria_ir::LogicVec> { None }
    pub fn eval_unary(&mut self, _op: &maria_ir::UnaryIrOp, _val: &maria_ir::LogicVec) -> Option<maria_ir::LogicVec> { None }
    pub fn eval_expression(&mut self, _expr: &maria_ir::IrExpr, _signal_values: &[u64], _result_width: usize) -> Option<maria_ir::LogicVec> { None }
}