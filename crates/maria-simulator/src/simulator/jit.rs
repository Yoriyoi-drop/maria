//! JIT Compiler untuk ekspresi evaluasi.
//!
//! Menggunakan monomorphization + caching untuk kompilasi ekspresi
//! ke fungsi native. Cranelift integration direncanakan untuk versi
//! selanjutnya.
//!
//! Arsitektur:
//! - `CompiledExpr` menyimpan hasil kompilasi ekspresi
//! - `JITCache` meng-cache ekspresi yang sudah dikompilasi
//! - `JITCompiler` adalah entry point utama

use std::collections::HashMap;
use std::sync::Mutex;

/// Hasil kompilasi sebuah ekspresi.
#[derive(Clone)]
pub struct CompiledExpr {
    /// Nama ekspresi (untuk debugging)
    pub name: String,
    /// Fungsi yang dikompilasi (Arc<dyn Fn> — bisa capture variable)
    pub eval_fn: std::sync::Arc<dyn Fn(&[u64], &[u64]) -> u64 + Send + Sync>,
    /// Jumlah argumen yang dibutuhkan
    pub arg_count: usize,
    /// Ukuran output (dalam bit)
    pub width: usize,
    /// Hit counter
    pub hit_count: u64,
}

// Manual Debug implementation since Arc<dyn Fn> doesn't implement Debug
impl std::fmt::Debug for CompiledExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompiledExpr")
            .field("name", &self.name)
            .field("arg_count", &self.arg_count)
            .field("width", &self.width)
            .field("hit_count", &self.hit_count)
            .finish()
    }
}

/// Cache ekspresi yang sudah dikompilasi.
pub struct JITCache {
    compiled: Mutex<HashMap<u64, CompiledExpr>>,
    hits: Mutex<u64>,
    misses: Mutex<u64>,
}

impl JITCache {
    pub fn new() -> Self {
        JITCache {
            compiled: Mutex::new(HashMap::new()),
            hits: Mutex::new(0),
            misses: Mutex::new(0),
        }
    }

    /// Cari ekspresi di cache berdasarkan hash.
    pub fn lookup(&self, hash: u64) -> Option<CompiledExpr> {
        let cache = self.compiled.lock().unwrap();
        if let Some(expr) = cache.get(&hash) {
            let mut expr = expr.clone();
            expr.hit_count += 1;
            let mut hits = self.hits.lock().unwrap();
            *hits += 1;
            // Update cache entry with new hit count
            drop(cache);
            let mut cache = self.compiled.lock().unwrap();
            cache.insert(hash, expr.clone());
            Some(expr)
        } else {
            let mut misses = self.misses.lock().unwrap();
            *misses += 1;
            None
        }
    }

    /// Insert compiled expression ke cache.
    pub fn insert(&self, hash: u64, expr: CompiledExpr) {
        let mut cache = self.compiled.lock().unwrap();
        cache.insert(hash, expr);
    }

    /// Hit rate.
    pub fn hit_rate(&self) -> f64 {
        let hits = *self.hits.lock().unwrap();
        let misses = *self.misses.lock().unwrap();
        let total = hits + misses;
        if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        }
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.compiled.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clear cache.
    pub fn clear(&self) {
        self.compiled.lock().unwrap().clear();
    }
}

impl Default for JITCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Built-in evaluator functions yang bisa dipanggil oleh JIT compiler.
pub mod intrinsics {
    /// Binary addition: a + b
    pub fn add(a: u64, b: u64) -> u64 {
        a.wrapping_add(b)
    }

    /// Binary subtraction: a - b
    pub fn sub(a: u64, b: u64) -> u64 {
        a.wrapping_sub(b)
    }

    /// Bitwise AND: a & b
    pub fn bit_and(a: u64, b: u64) -> u64 {
        a & b
    }

    /// Bitwise OR: a | b
    pub fn bit_or(a: u64, b: u64) -> u64 {
        a | b
    }

    /// Bitwise XOR: a ^ b
    pub fn bit_xor(a: u64, b: u64) -> u64 {
        a ^ b
    }

    /// Arithmetic: a * b
    pub fn mul(a: u64, b: u64) -> u64 {
        a.wrapping_mul(b)
    }

    /// Shift left: a << b
    pub fn shl(a: u64, b: u64) -> u64 {
        a.wrapping_shl(b as u32)
    }

    /// Shift right (logical): a >> b
    pub fn shr(a: u64, b: u64) -> u64 {
        a.wrapping_shr(b as u32)
    }

    /// Equality: a == b
    pub fn eq(a: u64, b: u64) -> u64 {
        if a == b { 1 } else { 0 }
    }

    /// Less-than: a < b
    pub fn lt(a: u64, b: u64) -> u64 {
        if a < b { 1 } else { 0 }
    }
}

/// JIT Compiler — mengkompilasi ekspresi menjadi fungsi native.
pub struct JITCompiler {
    compiled_count: usize,
    cache: JITCache,
}

impl JITCompiler {
    pub fn new() -> Result<Self, String> {
        Ok(JITCompiler {
            compiled_count: 0,
            cache: JITCache::new(),
        })
    }

    /// Compile a binary expression into a native function.
    /// Returns a compiled expression descriptor.
    pub fn compile_binary(
        &mut self,
        op: &str,
        lhs: &CompiledExpr,
        rhs: &CompiledExpr,
        width: usize,
    ) -> CompiledExpr {
        // Generate a hash from the expression
        let hash = self.compute_hash(op, lhs, rhs, width);

        // Check cache
        if let Some(cached) = self.cache.lookup(hash) {
            return cached;
        }

        // Select the appropriate function (as Arc-wrapped closure)
        let eval_fn: std::sync::Arc<dyn Fn(&[u64], &[u64]) -> u64 + Send + Sync> = match op {
            "+" => std::sync::Arc::new(|args, _| intrinsics::add(args[0], args[1])),
            "-" => std::sync::Arc::new(|args, _| intrinsics::sub(args[0], args[1])),
            "*" => std::sync::Arc::new(|args, _| intrinsics::mul(args[0], args[1])),
            "&" => std::sync::Arc::new(|args, _| intrinsics::bit_and(args[0], args[1])),
            "|" => std::sync::Arc::new(|args, _| intrinsics::bit_or(args[0], args[1])),
            "^" => std::sync::Arc::new(|args, _| intrinsics::bit_xor(args[0], args[1])),
            "==" => std::sync::Arc::new(|args, _| intrinsics::eq(args[0], args[1])),
            "<" => std::sync::Arc::new(|args, _| intrinsics::lt(args[0], args[1])),
            "<<" => std::sync::Arc::new(|args, _| intrinsics::shl(args[0], args[1])),
            ">>" => std::sync::Arc::new(|args, _| intrinsics::shr(args[0], args[1])),
            _ => std::sync::Arc::new(|_, _| 0), // fallback
        };

        let expr = CompiledExpr {
            name: format!("{}_{}_{}", op, lhs.name, rhs.name),
            eval_fn,
            arg_count: 2,
            width,
            hit_count: 0,
        };

        self.compiled_count += 1;
        self.cache.insert(hash, expr.clone());
        expr
    }

    /// Compile a unary expression.
    pub fn compile_unary(
        &mut self,
        op: &str,
        operand: &CompiledExpr,
        width: usize,
    ) -> CompiledExpr {
        let dummy = CompiledExpr {
            name: String::new(),
            eval_fn: std::sync::Arc::new(|_, _| 0),
            arg_count: 0,
            width: 0,
            hit_count: 0,
        };
        let hash = self.compute_hash(op, operand, &dummy, width);

        if let Some(cached) = self.cache.lookup(hash) {
            return cached;
        }

        let eval_fn: std::sync::Arc<dyn Fn(&[u64], &[u64]) -> u64 + Send + Sync> = match op {
            "~" => std::sync::Arc::new(|args, _| !args[0]),
            "-" => std::sync::Arc::new(|args, _| args[0].wrapping_neg()),
            "!" => std::sync::Arc::new(|args, _| if args[0] == 0 { 1 } else { 0 }),
            _ => std::sync::Arc::new(|args, _| args[0]),
        };

        let expr = CompiledExpr {
            name: format!("{}_{}", op, operand.name),
            eval_fn,
            arg_count: 1,
            width,
            hit_count: 0,
        };

        self.compiled_count += 1;
        self.cache.insert(hash, expr.clone());
        expr
    }

    /// Create a compiled expression for a constant value.
    pub fn compile_const(&mut self, value: u64, width: usize) -> CompiledExpr {
        let eval_fn: std::sync::Arc<dyn Fn(&[u64], &[u64]) -> u64 + Send + Sync> =
            std::sync::Arc::new(move |_, _| value);
        let expr = CompiledExpr {
            name: format!("const_{}", value),
            eval_fn,
            arg_count: 0,
            width,
            hit_count: 0,
        };
        expr
    }

    fn compute_hash(&self, op: &str, lhs: &CompiledExpr, rhs: &CompiledExpr, width: usize) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        op.hash(&mut hasher);
        lhs.name.hash(&mut hasher);
        rhs.name.hash(&mut hasher);
        width.hash(&mut hasher);
        hasher.finish()
    }

    pub fn compiled_count(&self) -> usize {
        self.compiled_count
    }

    /// Get cache statistics.
    pub fn cache_stats(&self) -> (usize, f64) {
        (self.cache.len(), self.cache.hit_rate())
    }
}

impl Default for JITCompiler {
    fn default() -> Self {
        Self::new().expect("JITCompiler::new failed")
    }
}

// ─── Tests ───
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jit_compiler_create() {
        let compiler = JITCompiler::new().unwrap();
        assert_eq!(compiler.compiled_count(), 0);
    }

    #[test]
    fn test_jit_const() {
        let mut compiler = JITCompiler::new().unwrap();
        let c = compiler.compile_const(42, 32);
        assert_eq!(c.width, 32);
        assert_eq!((c.eval_fn)(&[], &[]), 42);
    }

    #[test]
    fn test_jit_binary_add() {
        let mut compiler = JITCompiler::new().unwrap();
        let lhs = compiler.compile_const(10, 32);
        let rhs = compiler.compile_const(20, 32);
        let expr = compiler.compile_binary("+", &lhs, &rhs, 32);
        let result = (expr.eval_fn)(&[10, 20], &[]);
        assert_eq!(result, 30, "10 + 20 should be 30");
    }

    #[test]
    fn test_jit_binary_and() {
        let mut compiler = JITCompiler::new().unwrap();
        let lhs = compiler.compile_const(0xFF, 8);
        let rhs = compiler.compile_const(0x0F, 8);
        let expr = compiler.compile_binary("&", &lhs, &rhs, 8);
        let result = (expr.eval_fn)(&[0xFF, 0x0F], &[]);
        assert_eq!(result, 0x0F);
    }

    #[test]
    fn test_jit_binary_eq() {
        let mut compiler = JITCompiler::new().unwrap();
        let lhs = compiler.compile_const(5, 32);
        let rhs = compiler.compile_const(5, 32);
        let expr = compiler.compile_binary("==", &lhs, &rhs, 1);
        let result = (expr.eval_fn)(&[5, 5], &[]);
        assert_eq!(result, 1, "5 == 5 should be 1");
    }

    #[test]
    fn test_jit_binary_lt() {
        let mut compiler = JITCompiler::new().unwrap();
        let lhs = compiler.compile_const(3, 32);
        let rhs = compiler.compile_const(7, 32);
        let expr = compiler.compile_binary("<", &lhs, &rhs, 1);
        let result = (expr.eval_fn)(&[3, 7], &[]);
        assert_eq!(result, 1, "3 < 7 should be 1");
    }

    #[test]
    fn test_jit_unary_not() {
        let mut compiler = JITCompiler::new().unwrap();
        let op = compiler.compile_const(0xFF, 8);
        let expr = compiler.compile_unary("~", &op, 8);
        let result = (expr.eval_fn)(&[0xFF], &[]);
        assert_eq!(result, 0xFFFFFFFFFFFFFF00, "~0xFF for u64");
    }

    #[test]
    fn test_jit_cache() {
        let mut compiler = JITCompiler::new().unwrap();
        let lhs = compiler.compile_const(1, 32);
        let rhs = compiler.compile_const(2, 32);
        let _ = compiler.compile_binary("+", &lhs, &rhs, 32);
        let _ = compiler.compile_binary("+", &lhs, &rhs, 32); // should hit cache
        let (cached, hit_rate) = compiler.cache_stats();
        assert!(cached >= 1);
        assert!(hit_rate > 0.0);
    }

    #[test]
    fn test_jit_cache_hit_rate() {
        let mut compiler = JITCompiler::new().unwrap();
        let lhs = compiler.compile_const(10, 32);
        let rhs = compiler.compile_const(20, 32);
        let _ = compiler.compile_binary("+", &lhs, &rhs, 32);
        let _ = compiler.compile_binary("+", &lhs, &rhs, 32);
        let (_, hit_rate) = compiler.cache_stats();
        assert!(hit_rate > 0.0, "cache hit rate should be > 0");
    }
}
