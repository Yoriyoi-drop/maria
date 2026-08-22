//! Generator source SystemVerilog dari `Expr` (structure-aware).
//!
//! Menghasilkan modul `fuzz_mod` murni kombinasional: `assign y = <expr>(a,b)`
//! dengan `a`/`b` di-drive via `initial`. Input well-formed by construction
//! (bukan byte acak) → compile rate tinggi, jalur kode nyata terlatih.

use fastrand::Rng;

use super::expr::{gen_node, Expr};

/// Satu input fuzz: lebar, nilai stimulus a/b, ekspresi, dan seed RNG.
#[derive(Debug, Clone)]
pub struct GenInput {
    pub w: u32,
    pub a: u64,
    pub b: u64,
    pub expr: Expr,
    pub seed: u64,
}

fn lit_sv(v: u64, w: u32) -> String {
    let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
    let val = v & m;
    let mut bits = String::with_capacity(w as usize);
    for i in (0..w).rev() {
        bits.push(if (val >> i) & 1 == 1 { '1' } else { '0' });
    }
    format!("{}'b{}", w, bits)
}

impl GenInput {
    /// Render ke source SystemVerilog lengkap.
    pub fn to_source(&self) -> String {
        let w = self.w;
        let expr_sv = self.expr.to_sv(w);
        format!(
            "module fuzz_mod;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   reg [{hi}:0] b;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = {expr};\n\
             \x20   initial begin\n\
             \x20       a = {aval};\n\
             \x20       b = {bval};\n\
             \x20       #10;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            expr = expr_sv,
            aval = lit_sv(self.a, w),
            bval = lit_sv(self.b, w),
        )
    }
}

/// Hasilkan input fuzz baru dari seed RNG.
pub fn generate(seed: u64) -> GenInput {
    let mut rng = Rng::with_seed(seed);
    let w_choices = [1u32, 2, 4, 8, 16];
    let w = w_choices[rng.usize(0..w_choices.len())];
    let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
    let a = rng.u64(0..) & m;
    let b = rng.u64(0..) & m;
    let expr = gen_node(w, &mut rng, 0);
    GenInput { w, a, b, expr, seed }
}

/// Kloning input lalu mutasi ekspresinya (structure-aware mutation).
pub fn mutate_from(src: &GenInput, seed: u64) -> GenInput {
    let mut rng = Rng::with_seed(seed);
    let mut expr = src.expr.clone();
    expr.mutate(src.w, &mut rng);
    let mut a = src.a;
    let mut b = src.b;
    if rng.bool() {
        let m = if src.w >= 64 { u64::MAX } else { (1u64 << src.w) - 1 };
        a = rng.u64(0..) & m;
    }
    if rng.bool() {
        let m = if src.w >= 64 { u64::MAX } else { (1u64 << src.w) - 1 };
        b = rng.u64(0..) & m;
    }
    let w = if rng.bool() { src.w } else { w_choices_pick(&mut rng) };
    GenInput { w, a, b, expr, seed }
}

fn w_choices_pick(rng: &mut Rng) -> u32 {
    let w_choices = [1u32, 2, 4, 8, 16];
    w_choices[rng.usize(0..w_choices.len())]
}
