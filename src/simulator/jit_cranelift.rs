//! Cranelift JIT Backend — native code generation untuk ekspresi evaluasi.
//!
//! Menggunakan Cranelift code generator untuk mengkompilasi ekspresi
//! MIR ke native code (x86_64, ARM64, dll) via JIT.
//!
//! Arsitektur:
//! - `CraneliftEngine` — entry point, mengelola Cranelift module + context
//! - `build_binary_func / build_unary_func` — konversi operasi ke Cranelift IR
//! - Compile → native function pointer → call dari simulator

use std::collections::HashMap;
use std::sync::Mutex;

use cranelift::prelude::*;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};
use cranelift::codegen::ir::UserFuncName;

/// Hasil kompilasi Cranelift: function pointer + metadata.
#[derive(Clone)]
pub struct CraneliftCompiledFn {
    /// Nama fungsi (debugging)
    pub name: String,
    /// Pointer ke native code
    pub code_ptr: *const u8,
    /// Jumlah argumen
    pub arg_count: usize,
    /// Width output (bits)
    pub width: usize,
    /// Hit counter
    pub hit_count: u64,
}

// Safety: function pointer Send + Sync
unsafe impl Send for CraneliftCompiledFn {}
unsafe impl Sync for CraneliftCompiledFn {}

impl std::fmt::Debug for CraneliftCompiledFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CraneliftCompiledFn")
            .field("name", &self.name)
            .field("arg_count", &self.arg_count)
            .field("width", &self.width)
            .field("hit_count", &self.hit_count)
            .finish()
    }
}

/// Tipe operasi yang didukung oleh Cranelift JIT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JitOp {
    Add,
    Sub,
    Mul,
    And,
    Or,
    Xor,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Shl,
    Shr,
    Not,
    Neg,
}

/// Cranelift JIT Engine — mengkompilasi ekspresi ke native code.
pub struct CraneliftEngine {
    /// JIT module (Cranelift)
    module: JITModule,
    /// Function builder context
    ctx: FunctionBuilderContext,
    /// Compiled function cache: (name_hash) → compiled function
    cache: Mutex<HashMap<u64, CraneliftCompiledFn>>,
    /// Total compilation count
    compiled_count: Mutex<usize>,
    /// Total cache hits
    cache_hits: Mutex<u64>,
    cache_misses: Mutex<u64>,
}

impl CraneliftEngine {
    /// Create a new Cranelift JIT engine.
    ///
    /// Returns None if Cranelift initialization fails (e.g., unsupported
    /// architecture or missing CPU features).
    pub fn new() -> Option<Self> {
        let builder = JITBuilder::new(cranelift_module::default_libcall_names()).ok()?;
        let module = JITModule::new(builder);
        let ctx = FunctionBuilderContext::new();

        Some(CraneliftEngine {
            module,
            ctx,
            cache: Mutex::new(HashMap::new()),
            compiled_count: Mutex::new(0),
            cache_hits: Mutex::new(0),
            cache_misses: Mutex::new(0),
        })
    }

    /// Compile a binary operation to native code.
    ///
    /// Returns a compiled function that takes two u64 arguments and returns u64.
    pub fn compile_binary(&mut self, op: JitOp, width: usize) -> Option<CraneliftCompiledFn> {
        let op_name = format!("bin_{:?}", op);
        let hash = self.compute_hash(&op_name, width);

        // Check cache
        if let Some(cached) = self.cache_get(hash) {
            return Some(cached);
        }

        // Build Cranelift function
        let func = self.build_binary_func(op, width)?;
        let name = format!("jit_{}", op_name);

        // Compile and finalize
        let code_ptr = self.finalize_function(func, &name)?;

        let compiled = CraneliftCompiledFn {
            name,
            code_ptr,
            arg_count: 2,
            width,
            hit_count: 0,
        };

        *self.compiled_count.lock().unwrap() += 1;
        self.cache_insert(hash, compiled.clone());
        Some(compiled)
    }

    /// Compile a unary operation to native code.
    pub fn compile_unary(&mut self, op: JitOp, width: usize) -> Option<CraneliftCompiledFn> {
        let op_name = format!("un_{:?}", op);
        let hash = self.compute_hash(&op_name, width);

        if let Some(cached) = self.cache_get(hash) {
            return Some(cached);
        }

        let func = self.build_unary_func(op, width)?;
        let name = format!("jit_{}", op_name);
        let code_ptr = self.finalize_function(func, &name)?;

        let compiled = CraneliftCompiledFn {
            name,
            code_ptr,
            arg_count: 1,
            width,
            hit_count: 0,
        };

        *self.compiled_count.lock().unwrap() += 1;
        self.cache_insert(hash, compiled.clone());
        Some(compiled)
    }

    /// Build a Cranelift IR function for a binary operation.
    fn build_binary_func(
        &mut self,
        op: JitOp,
        width: usize,
    ) -> Option<cranelift::codegen::ir::Function> {
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));        let name = UserFuncName::user(0, 0);
        let mut func
            = cranelift::codegen::ir::Function::with_name_signature(name, sig);

        {
            let mut builder = FunctionBuilder::new(&mut func, &mut self.ctx);

            let block = builder.create_block();
            builder.append_block_params_for_function_params(block);
            builder.switch_to_block(block);
            builder.seal_block(block);

            let a = builder.block_params(block)[0];
            let b = builder.block_params(block)[1];

            let mask_val: i64 = if width < 64 {
                ((1u64 << width) - 1) as i64
            } else {
                -1i64 // u64::MAX as i64
            };
            let mask = builder.ins().iconst(types::I64, mask_val);

            let result = match op {
                JitOp::Add => builder.ins().iadd(a, b),
                JitOp::Sub => builder.ins().isub(a, b),
                JitOp::Mul => builder.ins().imul(a, b),
                JitOp::And => builder.ins().band(a, b),
                JitOp::Or => builder.ins().bor(a, b),
                JitOp::Xor => builder.ins().bxor(a, b),
                JitOp::Eq | JitOp::Ne => {
                    let cmp = match op {
                        JitOp::Eq => IntCC::Equal,
                        JitOp::Ne => IntCC::NotEqual,
                        _ => unreachable!(),
                    };
                    let cond = builder.ins().icmp(cmp, a, b);
                    let one = builder.ins().iconst(types::I64, 1i64);
                    let zero = builder.ins().iconst(types::I64, 0i64);
                    builder.ins().select(cond, one, zero)
                }
                JitOp::Lt | JitOp::Le | JitOp::Gt | JitOp::Ge => {
                    let cmp = match op {
                        JitOp::Lt => IntCC::SignedLessThan,
                        JitOp::Le => IntCC::SignedLessThanOrEqual,
                        JitOp::Gt => IntCC::SignedGreaterThan,
                        JitOp::Ge => IntCC::SignedGreaterThanOrEqual,
                        _ => unreachable!(),
                    };
                    let cond = builder.ins().icmp(cmp, a, b);
                    let one = builder.ins().iconst(types::I64, 1i64);
                    let zero = builder.ins().iconst(types::I64, 0i64);
                    builder.ins().select(cond, one, zero)
                }
                JitOp::Shl => builder.ins().ishl(a, b),
                JitOp::Shr => builder.ins().ushr(a, b),
                _ => builder.ins().iconst(types::I64, 0i64),
            };

            // Apply mask
            let masked = if width < 64 {
                builder.ins().band(result, mask)
            } else {
                result
            };

            builder.ins().return_(&[masked]);
            builder.finalize();
        }

        Some(func)
    }

    /// Build a Cranelift IR function for a unary operation.
    fn build_unary_func(
        &mut self,
        op: JitOp,
        width: usize,
    ) -> Option<cranelift::codegen::ir::Function> {
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));        let name = UserFuncName::user(0, 0);
        let mut func
            = cranelift::codegen::ir::Function::with_name_signature(name, sig);

        {
            let mut builder = FunctionBuilder::new(&mut func, &mut self.ctx);

            let block = builder.create_block();
            builder.append_block_params_for_function_params(block);
            builder.switch_to_block(block);
            builder.seal_block(block);

            let a = builder.block_params(block)[0];

            let result = match op {
                JitOp::Not => builder.ins().bnot(a),
                JitOp::Neg => builder.ins().ineg(a),
                _ => builder.ins().iconst(types::I64, 0i64),
            };

            // Apply width mask
            let masked = if width < 64 {
                let mask_val: i64 = ((1u64 << width) - 1) as i64;
                let mask = builder.ins().iconst(types::I64, mask_val);
                builder.ins().band(result, mask)
            } else {
                result
            };

            builder.ins().return_(&[masked]);
            builder.finalize();
        }

        Some(func)
    }

    /// Finalize a Cranelift function and return a pointer to native code.
    fn finalize_function(
        &mut self,
        func: cranelift::codegen::ir::Function,
        name: &str,
    ) -> Option<*const u8> {
        // Declare function in the module
        let id = self
            .module
            .declare_function(name, Linkage::Local, &func.signature)
            .ok()?;

        // Create a context and set the function body
        let mut ctx = cranelift::codegen::Context::new();
        ctx.func = func;

        // Define the function body
        self.module.define_function(id, &mut ctx).ok()?;

        // Finalize — generate machine code
        self.module.finalize_definitions();

        // Get pointer to compiled code
        let code_ptr = self.module.get_finalized_function(id);

        // Clear context for next compilation
        self.module.clear_context(&mut ctx);

        Some(code_ptr as *const u8)
    }

    /// Call a compiled binary function with arguments.
    ///
    /// # Safety
    /// `code_ptr` must point to a valid compiled function with signature
    /// `fn(u64, u64) -> u64`.
    pub unsafe fn call_binary(code_ptr: *const u8, a: u64, b: u64) -> u64 {
        let func: fn(u64, u64) -> u64 = std::mem::transmute(code_ptr);
        func(a, b)
    }

    /// Call a compiled unary function with an argument.
    ///
    /// # Safety
    /// `code_ptr` must point to a valid compiled function with signature
    /// `fn(u64) -> u64`.
    pub unsafe fn call_unary(code_ptr: *const u8, a: u64) -> u64 {
        let func: fn(u64) -> u64 = std::mem::transmute(code_ptr);
        func(a)
    }

    // ─── Cache Helpers ───

    fn compute_hash(&self, name: &str, width: usize) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        name.hash(&mut hasher);
        width.hash(&mut hasher);
        hasher.finish()
    }

    fn cache_get(&self, hash: u64) -> Option<CraneliftCompiledFn> {
        let cache = self.cache.lock().unwrap();
        if let Some(entry) = cache.get(&hash) {
            let mut entry = entry.clone();
            entry.hit_count += 1;
            *self.cache_hits.lock().unwrap() += 1;
            Some(entry)
        } else {
            *self.cache_misses.lock().unwrap() += 1;
            None
        }
    }

    fn cache_insert(&self, hash: u64, compiled: CraneliftCompiledFn) {
        self.cache.lock().unwrap().insert(hash, compiled);
    }

    // ─── Expression-level JIT ───

    // ─── Expression-level JIT ───

    /// Compute a structural hash for an IrExpr tree (for expression-level caching).
    fn hash_ir_expr(expr: &crate::ir::IrExpr) -> u64 {
        use std::hash::{Hash, Hasher};
        use crate::ir::{BinaryIrOp, IrExpr, UnaryIrOp};

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        Self::hash_ir_expr_recursive(expr, &mut hasher);
        hasher.finish()
    }

    fn hash_ir_expr_recursive(expr: &crate::ir::IrExpr, hasher: &mut std::collections::hash_map::DefaultHasher) {
        use std::hash::Hash;

        match expr {
            crate::ir::IrExpr::Const(lv) => {
                0u64.hash(hasher);
                lv.to_u64().hash(hasher);
                lv.width.hash(hasher);
            }
            crate::ir::IrExpr::Signal(id, w) => {
                1u64.hash(hasher);
                id.hash(hasher);
                w.hash(hasher);
            }
            crate::ir::IrExpr::BinaryOp(op, lhs, rhs) => {
                2u64.hash(hasher);
                Self::hash_binary_op(op, hasher);
                Self::hash_ir_expr_recursive(lhs, hasher);
                Self::hash_ir_expr_recursive(rhs, hasher);
            }
            crate::ir::IrExpr::UnaryOp(op, inner) => {
                3u64.hash(hasher);
                Self::hash_unary_op(op, hasher);
                Self::hash_ir_expr_recursive(inner, hasher);
            }
            crate::ir::IrExpr::Cond(cond, t, f) => {
                4u64.hash(hasher);
                Self::hash_ir_expr_recursive(cond, hasher);
                Self::hash_ir_expr_recursive(t, hasher);
                Self::hash_ir_expr_recursive(f, hasher);
            }
            _ => {
                255u64.hash(hasher); // unsupported variant
            }
        }
    }

    /// Hash a BinaryIrOp by its discriminant (manual since it doesn't derive Hash).
    fn hash_binary_op(op: &crate::ir::BinaryIrOp, hasher: &mut std::collections::hash_map::DefaultHasher) {
        use std::hash::Hash;
        use crate::ir::BinaryIrOp;
        let disc: u64 = match op {
            BinaryIrOp::Add => 0,
            BinaryIrOp::Sub => 1,
            BinaryIrOp::Mul => 2,
            BinaryIrOp::Div => 3,
            BinaryIrOp::Mod => 4,
            BinaryIrOp::Power => 5,
            BinaryIrOp::Eq => 6,
            BinaryIrOp::Neq => 7,
            BinaryIrOp::CaseEq => 8,
            BinaryIrOp::CaseNeq => 9,
            BinaryIrOp::EqWild => 10,
            BinaryIrOp::NeqWild => 11,
            BinaryIrOp::Lt => 12,
            BinaryIrOp::Le => 13,
            BinaryIrOp::Gt => 14,
            BinaryIrOp::Ge => 15,
            BinaryIrOp::BitAnd => 16,
            BinaryIrOp::BitOr => 17,
            BinaryIrOp::BitXor => 18,
            BinaryIrOp::BitXnor => 19,
            BinaryIrOp::Shl => 20,
            BinaryIrOp::Shr => 21,
            BinaryIrOp::Sshl => 22,
            BinaryIrOp::Sshr => 23,
            BinaryIrOp::LogicalAnd => 24,
            BinaryIrOp::LogicalOr => 25,
        };
        disc.hash(hasher);
    }

    /// Hash a UnaryIrOp by its discriminant (manual since it doesn't derive Hash).
    fn hash_unary_op(op: &crate::ir::UnaryIrOp, hasher: &mut std::collections::hash_map::DefaultHasher) {
        use std::hash::Hash;
        use crate::ir::UnaryIrOp;
        let disc: u64 = match op {
            UnaryIrOp::Plus => 0,
            UnaryIrOp::Minus => 1,
            UnaryIrOp::Not => 2,
            UnaryIrOp::BitNot => 3,
            UnaryIrOp::RedAnd => 4,
            UnaryIrOp::RedNand => 5,
            UnaryIrOp::RedOr => 6,
            UnaryIrOp::RedNor => 7,
            UnaryIrOp::RedXor => 8,
            UnaryIrOp::RedXnor => 9,
        };
        disc.hash(hasher);
    }

    /// Compile an IrExpr expression tree into a single native function.
    ///
    /// The function signature is: `fn(s0: u64, s1: u64, ..., sN: u64) -> u64`
    /// where N is the number of unique signal references in the expression.
    /// The `signal_ids` array maps each function parameter (index) to a global signal ID.
    ///
    /// Supported IrExpr variants:
    /// - Const, Signal (by index)
    /// - BinaryOp (all JitOps)
    /// - UnaryOp (Not, Neg)
    /// - Cond (ternary)
    ///
    /// Falls back to `None` for unsupported variants (SysFunc, MethodCall, etc.)
    /// or expressions with more than 8 unique signal references.
    pub fn compile_expression(
        &mut self,
        expr: &crate::ir::IrExpr,
        signal_ids: &[usize],
    ) -> Option<CraneliftCompiledFn> {
        if signal_ids.len() > 8 || signal_ids.is_empty() {
            return None;
        }

        let n_sigs = signal_ids.len();
        let expr_hash = Self::hash_ir_expr(expr);
        let hash = self.compute_hash("expr_tree", expr_hash as usize);

        if let Some(cached) = self.cache_get(hash) {
            return Some(cached);
        }

        // Build Cranelift IR for the expression tree
        let mut sig = self.module.make_signature();
        for _ in 0..n_sigs {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.returns.push(AbiParam::new(types::I64));

        let func_name = UserFuncName::user(0, hash as u32);
        let mut func = cranelift::codegen::ir::Function::with_name_signature(func_name, sig);

        {
            let mut builder = FunctionBuilder::new(&mut func, &mut self.ctx);
            let block = builder.create_block();
            builder.append_block_params_for_function_params(block);
            builder.switch_to_block(block);
            builder.seal_block(block);

            let params: Vec<Value> = builder.block_params(block).to_vec();

            // Build expression recursively — returns Cranelift Value
            let result = Self::build_expr_ir(expr, &params, signal_ids, &mut builder)?;

            builder.ins().return_(&[result]);
            builder.finalize();
        }

        let fn_name_str = format!("jit_expr_{:x}", hash);
        let code_ptr = self.finalize_function(func, &fn_name_str)?;

        let compiled = CraneliftCompiledFn {
            name: format!("expr_tree_{}sig", n_sigs),
            code_ptr,
            arg_count: n_sigs,
            width: 64,
            hit_count: 0,
        };

        *self.compiled_count.lock().unwrap() += 1;
        self.cache_insert(hash, compiled.clone());
        Some(compiled)
    }

    /// Recursively build Cranelift IR for an IrExpr.
    /// Returns the Cranelift `Value` representing the expression result.
    ///
    /// `signal_params` — Cranelift block parameters (index 0..n = signal_ids[0..n])
    /// `signal_ids` — maps param index → global signal ID for Signal() lookup
    /// Recursively build Cranelift IR for an IrExpr.
    /// Returns the Cranelift `Value` representing the expression result.
    ///
    /// This is a standalone function (not a method) to avoid borrow conflicts
    /// with `FunctionBuilder` that borrows `self.ctx`.
    ///
    /// `signal_params` — Cranelift block parameters (index 0..n = signal_ids[0..n])
    /// `signal_ids` — maps param index → global signal ID for Signal() lookup
    fn build_expr_ir(
        expr: &crate::ir::IrExpr,
        signal_params: &[Value],
        signal_ids: &[usize],
        builder: &mut FunctionBuilder,
    ) -> Option<Value> {
        use crate::ir::{BinaryIrOp, IrExpr, UnaryIrOp};

        match expr {
            IrExpr::Const(lv) => {
                let val = lv.to_u64();
                Some(builder.ins().iconst(types::I64, val as i64))
            }
            IrExpr::Signal(id, _width) => {
                // Find the position of this global signal ID in signal_ids
                let pos = signal_ids.iter().position(|sid| sid == id)?;
                if pos < signal_params.len() {
                    Some(signal_params[pos])
                } else {
                    None
                }
            }
            IrExpr::BinaryOp(op, lhs, rhs) => {
                let lv = Self::build_expr_ir(lhs, signal_params, signal_ids, builder)?;
                let rv = Self::build_expr_ir(rhs, signal_params, signal_ids, builder)?;

                match op {
                    BinaryIrOp::Add => Some(builder.ins().iadd(lv, rv)),
                    BinaryIrOp::Sub => Some(builder.ins().isub(lv, rv)),
                    BinaryIrOp::Mul => Some(builder.ins().imul(lv, rv)),
                    BinaryIrOp::BitAnd => Some(builder.ins().band(lv, rv)),
                    BinaryIrOp::BitOr => Some(builder.ins().bor(lv, rv)),
                    BinaryIrOp::BitXor => Some(builder.ins().bxor(lv, rv)),
                    BinaryIrOp::Shl => Some(builder.ins().ishl(lv, rv)),
                    BinaryIrOp::Shr => Some(builder.ins().ushr(lv, rv)),
                    BinaryIrOp::Eq | BinaryIrOp::CaseEq => {
                        let one = builder.ins().iconst(types::I64, 1i64);
                        let zero = builder.ins().iconst(types::I64, 0i64);
                        let cmp = builder.ins().icmp(IntCC::Equal, lv, rv);
                        Some(builder.ins().select(cmp, one, zero))
                    }
                    BinaryIrOp::Neq | BinaryIrOp::CaseNeq => {
                        let one = builder.ins().iconst(types::I64, 1i64);
                        let zero = builder.ins().iconst(types::I64, 0i64);
                        let cmp = builder.ins().icmp(IntCC::NotEqual, lv, rv);
                        Some(builder.ins().select(cmp, one, zero))
                    }
                    _ => Some(builder.ins().iconst(types::I64, 0i64)),
                }
            }
            IrExpr::UnaryOp(op, inner) => {
                let val = Self::build_expr_ir(inner, signal_params, signal_ids, builder)?;
                match op {
                    UnaryIrOp::BitNot => Some(builder.ins().bnot(val)),
                    UnaryIrOp::Minus => Some(builder.ins().ineg(val)),
                    UnaryIrOp::Plus => Some(val),
                    _ => Some(builder.ins().iconst(types::I64, 0i64)),
                }
            }
            IrExpr::Cond(cond, true_expr, false_expr) => {
                let cval = Self::build_expr_ir(cond, signal_params, signal_ids, builder)?;
                let tval = Self::build_expr_ir(true_expr, signal_params, signal_ids, builder)?;
                let fval = Self::build_expr_ir(false_expr, signal_params, signal_ids, builder)?;

                let zero = builder.ins().iconst(types::I64, 0i64);
                let is_true = builder.ins().icmp(IntCC::NotEqual, cval, zero);
                Some(builder.ins().select(is_true, tval, fval))
            }
            _ => None, // Unsupported expression type
        }
    }

    /// Call a compiled expression function with signal values.
    ///
    /// # Safety
    /// `code_ptr` must point to a valid compiled function with signature
    /// `fn(u64, ..., u64) -> u64` where the number of arguments matches `n_sigs`.
    pub unsafe fn call_expression(code_ptr: *const u8, signal_values: &[u64]) -> u64 {
        match signal_values.len() {
            0 => 0u64,
            1 => {
                let func: fn(u64) -> u64 = std::mem::transmute(code_ptr);
                func(signal_values[0])
            }
            2 => {
                let func: fn(u64, u64) -> u64 = std::mem::transmute(code_ptr);
                func(signal_values[0], signal_values[1])
            }
            3 => {
                let func: fn(u64, u64, u64) -> u64 = std::mem::transmute(code_ptr);
                func(signal_values[0], signal_values[1], signal_values[2])
            }
            4 => {
                let func: fn(u64, u64, u64, u64) -> u64 = std::mem::transmute(code_ptr);
                func(signal_values[0], signal_values[1], signal_values[2], signal_values[3])
            }
            5 => {
                let func: fn(u64, u64, u64, u64, u64) -> u64 = std::mem::transmute(code_ptr);
                func(signal_values[0], signal_values[1], signal_values[2], signal_values[3], signal_values[4])
            }
            6 => {
                let func: fn(u64, u64, u64, u64, u64, u64) -> u64 = std::mem::transmute(code_ptr);
                func(signal_values[0], signal_values[1], signal_values[2], signal_values[3], signal_values[4], signal_values[5])
            }
            7 => {
                let func: fn(u64, u64, u64, u64, u64, u64, u64) -> u64 = std::mem::transmute(code_ptr);
                func(signal_values[0], signal_values[1], signal_values[2], signal_values[3], signal_values[4], signal_values[5], signal_values[6])
            }
            8 => {
                let func: fn(u64, u64, u64, u64, u64, u64, u64, u64) -> u64 = std::mem::transmute(code_ptr);
                func(signal_values[0], signal_values[1], signal_values[2], signal_values[3], signal_values[4], signal_values[5], signal_values[6], signal_values[7])
            }
            _ => 0u64,
        }
    }

    // ─── Statistics ───

    pub fn compiled_count(&self) -> usize {
        *self.compiled_count.lock().unwrap()
    }

    pub fn cache_hit_rate(&self) -> f64 {
        let hits = *self.cache_hits.lock().unwrap();
        let misses = *self.cache_misses.lock().unwrap();
        let total = hits + misses;
        if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        }
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cranelift_engine_create() {
        let engine = CraneliftEngine::new();
        assert!(engine.is_some(), "CraneliftEngine should initialize on x86_64");
    }

    #[test]
    fn test_cranelift_binary_add() {
        let mut engine = CraneliftEngine::new().unwrap();
        let compiled = engine.compile_binary(JitOp::Add, 32).unwrap();

        unsafe {
            let result = CraneliftEngine::call_binary(compiled.code_ptr, 10, 20);
            assert_eq!(result, 30, "10 + 20 = 30");
        }
    }

    #[test]
    fn test_cranelift_binary_sub() {
        let mut engine = CraneliftEngine::new().unwrap();
        let compiled = engine.compile_binary(JitOp::Sub, 32).unwrap();

        unsafe {
            let result = CraneliftEngine::call_binary(compiled.code_ptr, 50, 23);
            assert_eq!(result, 27, "50 - 23 = 27");
        }
    }

    #[test]
    fn test_cranelift_binary_mul() {
        let mut engine = CraneliftEngine::new().unwrap();
        let compiled = engine.compile_binary(JitOp::Mul, 32).unwrap();

        unsafe {
            let result = CraneliftEngine::call_binary(compiled.code_ptr, 6, 7);
            assert_eq!(result, 42, "6 * 7 = 42");
        }
    }

    #[test]
    fn test_cranelift_binary_and() {
        let mut engine = CraneliftEngine::new().unwrap();
        let compiled = engine.compile_binary(JitOp::And, 8).unwrap();

        unsafe {
            let result = CraneliftEngine::call_binary(compiled.code_ptr, 0xFF, 0x0F);
            assert_eq!(result, 0x0F);
        }
    }

    #[test]
    fn test_cranelift_binary_or() {
        let mut engine = CraneliftEngine::new().unwrap();
        let compiled = engine.compile_binary(JitOp::Or, 8).unwrap();

        unsafe {
            let result = CraneliftEngine::call_binary(compiled.code_ptr, 0xF0, 0x0F);
            assert_eq!(result, 0xFF);
        }
    }

    #[test]
    fn test_cranelift_binary_xor() {
        let mut engine = CraneliftEngine::new().unwrap();
        let compiled = engine.compile_binary(JitOp::Xor, 8).unwrap();

        unsafe {
            let result = CraneliftEngine::call_binary(compiled.code_ptr, 0xFF, 0x0F);
            assert_eq!(result, 0xF0);
        }
    }

    #[test]
    fn test_cranelift_binary_eq() {
        let mut engine = CraneliftEngine::new().unwrap();
        let eq = engine.compile_binary(JitOp::Eq, 1).unwrap();

        unsafe {
            assert_eq!(CraneliftEngine::call_binary(eq.code_ptr, 5, 5), 1);
            assert_eq!(CraneliftEngine::call_binary(eq.code_ptr, 5, 6), 0);
        }
    }

    #[test]
    fn test_cranelift_binary_lt() {
        let mut engine = CraneliftEngine::new().unwrap();
        let lt = engine.compile_binary(JitOp::Lt, 1).unwrap();

        unsafe {
            assert_eq!(CraneliftEngine::call_binary(lt.code_ptr, 3, 7), 1);
            assert_eq!(CraneliftEngine::call_binary(lt.code_ptr, 7, 3), 0);
        }
    }

    #[test]
    fn test_cranelift_binary_shl() {
        let mut engine = CraneliftEngine::new().unwrap();
        let shl = engine.compile_binary(JitOp::Shl, 8).unwrap();

        unsafe {
            let result = CraneliftEngine::call_binary(shl.code_ptr, 1, 3);
            assert_eq!(result, 8);
        }
    }

    #[test]
    fn test_cranelift_unary_not() {
        let mut engine = CraneliftEngine::new().unwrap();
        let not = engine.compile_unary(JitOp::Not, 8).unwrap();

        unsafe {
            let result = CraneliftEngine::call_unary(not.code_ptr, 0xFF);
            assert_eq!(result, 0x00);
        }
    }

    #[test]
    fn test_cranelift_unary_neg() {
        let mut engine = CraneliftEngine::new().unwrap();
        let neg = engine.compile_unary(JitOp::Neg, 32).unwrap();

        unsafe {
            let result = CraneliftEngine::call_unary(neg.code_ptr, 42);
            // -42 in 32-bit is 0xFFFFFFD6 = 4294967254
            assert_eq!(result as i32, -42);
        }
    }

    #[test]
    fn test_cranelift_width_masking() {
        let mut engine = CraneliftEngine::new().unwrap();

        // 4-bit add: should mask to 4 bits
        let add = engine.compile_binary(JitOp::Add, 4).unwrap();
        unsafe {
            // 15 + 1 = 16 → masked to 4 bits → 0
            let result = CraneliftEngine::call_binary(add.code_ptr, 15, 1);
            assert_eq!(result, 0, "4-bit add should wrap: 15+1=0");
        }
    }

    #[test]
    fn test_cranelift_cache() {
        let mut engine = CraneliftEngine::new().unwrap();
        assert_eq!(engine.compiled_count(), 0);

        let _ = engine.compile_binary(JitOp::Add, 32);
        assert_eq!(engine.compiled_count(), 1);

        // Second call should hit cache
        let _ = engine.compile_binary(JitOp::Add, 32);
        assert!(engine.cache_hit_rate() > 0.0);
    }

    #[test]
    fn test_cranelift_binary_ne() {
        let mut engine = CraneliftEngine::new().unwrap();
        let ne = engine.compile_binary(JitOp::Ne, 1).unwrap();

        unsafe {
            assert_eq!(CraneliftEngine::call_binary(ne.code_ptr, 5, 5), 0);
            assert_eq!(CraneliftEngine::call_binary(ne.code_ptr, 5, 6), 1);
        }
    }

    #[test]
    fn test_cranelift_binary_gt() {
        let mut engine = CraneliftEngine::new().unwrap();
        let gt = engine.compile_binary(JitOp::Gt, 1).unwrap();

        unsafe {
            assert_eq!(CraneliftEngine::call_binary(gt.code_ptr, 7, 3), 1);
            assert_eq!(CraneliftEngine::call_binary(gt.code_ptr, 3, 7), 0);
        }
    }

    #[test]
    fn test_cranelift_binary_ge() {
        let mut engine = CraneliftEngine::new().unwrap();
        let ge = engine.compile_binary(JitOp::Ge, 1).unwrap();

        unsafe {
            assert_eq!(CraneliftEngine::call_binary(ge.code_ptr, 5, 5), 1);
            assert_eq!(CraneliftEngine::call_binary(ge.code_ptr, 7, 3), 1);
            assert_eq!(CraneliftEngine::call_binary(ge.code_ptr, 3, 7), 0);
        }
    }

    #[test]
    fn test_cranelift_binary_le() {
        let mut engine = CraneliftEngine::new().unwrap();
        let le = engine.compile_binary(JitOp::Le, 1).unwrap();

        unsafe {
            assert_eq!(CraneliftEngine::call_binary(le.code_ptr, 5, 5), 1);
            assert_eq!(CraneliftEngine::call_binary(le.code_ptr, 3, 7), 1);
            assert_eq!(CraneliftEngine::call_binary(le.code_ptr, 7, 3), 0);
        }
    }
}
