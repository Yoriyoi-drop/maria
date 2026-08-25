//! Generator source SystemVerilog dari `Expr` (structure-aware).
//!
//! Menghasilkan modul `fuzz_mod` murni kombinasional: `assign y = <expr>(a,b)`
//! dengan `a`/`b` di-drive via `initial`. Input well-formed by construction
//! (bukan byte acak) → compile rate tinggi, jalur kode nyata terlatih.

use fastrand::Rng;

use super::expr::{gen_node, Expr};

/// Satu input fuzz: lebar, nilai stimulus a/b, ekspresi, dan seed RNG.
///
/// `wb` = lebar variabel `b`, SELALU `≤ w`. Dengan `b` tersimpan pre-masked
/// ke `wb`, model emas yang ada tetap eksak tanpa perubahan semantik:
/// - `Var('b')`: zero-extension ke konteks identik dengan masking;
/// - `BitSel/PartSel('b', …)` di luar `wb` → X (LRM §11.5.1) — persis
///   seperti select pada variabel sempit di dunia nyata;
/// - kelas bug baru yang tercakup: implicit truncation/extension saat dua
///   operan berbeda lebar bertemu di satu ekspresi.
#[derive(Debug, Clone)]
pub struct GenInput {
    pub w: u32,
    /// Lebar deklarasi `b` (≤ w).
    pub wb: u32,
    pub a: u64,
    pub b: u64,
    pub expr: Expr,
    pub seed: u64,
}

impl GenInput {
    /// Paksa invariant: `1 ≤ wb ≤ w`, `b` ter-mask ke `wb`.
    pub fn normalize(&mut self) {
        if self.w == 0 {
            self.w = 1;
        }
        self.wb = self.wb.clamp(1, self.w);
        self.b &= mask_of(self.wb);
        self.a &= mask_of(self.w);
    }
}

/// Render literal sized `{w}'b...` — dipakai generator source DAN modul
/// fuzz multi-vektor (stimulus berurutan).
pub fn lit_sv(v: u64, w: u32) -> String {
    if w == 0 {
        return "0".to_string();
    }
    let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
    let val = v & m;
    let mut bits = String::with_capacity(w as usize);
    for i in (0..w).rev() {
        let bit = if i >= 64 { 0 } else { (val >> i) & 1 };
        bits.push(if bit == 1 { '1' } else { '0' });
    }
    format!("{}'b{}", w, bits)
}

/// Pilihan lebar bit — termasuk boundary (31/32/33, 15/16/17) untuk
/// menyentuh jalur kode width-handling di lexer/parser/elaborator/engine.
/// 63/64/65 menguji batas u64 internal (`mask_of` cabang `w >= 64`,
/// `to_u64`, truncation literal >64 bit). 72/96/128 melatih penyimpanan
/// multi-word LogicVec (>64 bit) end-to-end.
pub const WIDTH_CHOICES: [u32; 18] = [
    1, 2, 3, 4, 7, 8, 15, 16, 17, 31, 32, 33, 63, 64, 65, 72, 96, 128,
];

/// Nilai stimulus boundary — 0, 1, all-ones, dan pola bit ekstrem sering
/// memicu bug yang dilewatkan nilai acak (mis. shift by max, div near-max).
const BOUNDARY_VALUES: [u64; 8] = [
    0,
    1,
    2,
    u64::MAX,
    0x5555_5555_5555_5555,
    0xAAAA_AAAA_AAAA_AAAA,
    0x8000_0000_0000_0000,
    0xFFFF_FFFF_FFFF_FFFF, // = u64::MAX, duplikat disengaja untuk bobot
];

/// Mask nilai stimulus ke lebar `w` (nilai selalu muat u64; bit ≥ 64 = 0).
pub fn mask_of(w: u32) -> u64 {
    if w >= 64 {
        u64::MAX
    } else {
        (1u64 << w) - 1
    }
}

impl GenInput {
    /// Render ke source SystemVerilog lengkap.
    pub fn to_source(&self) -> String {
        let w = self.w;
        let expr_sv = self.expr.to_sv(w);
        format!(
            "module fuzz_mod;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   reg [{bhi}:0] b;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = {expr};\n\
             \x20   initial begin\n\
             \x20       a = {aval};\n\
             \x20       b = {bval};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            bhi = self.wb - 1,
            expr = expr_sv,
            aval = lit_sv(self.a, w),
            bval = lit_sv(self.b, self.wb),
        )
    }
}

/// Pilih lebar `b` ≤ `w` dari WIDTH_CHOICES (termasuk == w agar ~setengah
/// kasus tetap selebar sama).
fn pick_wb(w: u32, rng: &mut Rng) -> u32 {
    let candidates: Vec<u32> = WIDTH_CHOICES.iter().copied().filter(|&c| c <= w).collect();
    if rng.bool() {
        w
    } else {
        candidates[rng.usize(0..candidates.len())]
    }
}

/// Hasilkan input fuzz baru dari seed RNG.
pub fn generate(seed: u64) -> GenInput {
    let mut rng = Rng::with_seed(seed);
    let w = WIDTH_CHOICES[rng.usize(0..WIDTH_CHOICES.len())];
    let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
    // ~25% stimulus dari boundary values (dimask ke lebar), sisanya acak.
    let a = if rng.usize(0..4) == 0 {
        BOUNDARY_VALUES[rng.usize(0..BOUNDARY_VALUES.len())] & m
    } else {
        rng.u64(0..) & m
    };
    let b = if rng.usize(0..4) == 0 {
        BOUNDARY_VALUES[rng.usize(0..BOUNDARY_VALUES.len())] & m
    } else {
        rng.u64(0..) & m
    };
    // Kedalaman 5 (dulu 4): subtree lebih dalam menaikkan kemungkinan
    // bug intermediet width/X yang tak muncul di ekspresi dangkal.
    let expr = gen_node(w, &mut rng, 0);
    let wb = pick_wb(w, &mut rng);
    let b = b & mask_of(wb);
    let mut input = GenInput {
        w,
        wb,
        a,
        b,
        expr,
        seed,
    };
    input.normalize();
    input
}

/// Kloning input lalu mutasi ekspresinya (structure-aware mutation).
pub fn mutate_from(src: &GenInput, seed: u64) -> GenInput {
    let mut rng = Rng::with_seed(seed);
    let mut expr = src.expr.clone();
    expr.mutate(src.w, &mut rng);
    let mut a = src.a;
    let mut b = src.b;
    if rng.bool() {
        let m = if src.w >= 64 {
            u64::MAX
        } else {
            (1u64 << src.w) - 1
        };
        a = rng.u64(0..) & m;
    }
    if rng.bool() {
        let m = if src.w >= 64 {
            u64::MAX
        } else {
            (1u64 << src.w) - 1
        };
        b = rng.u64(0..) & m;
    }
    let w = if rng.bool() {
        src.w
    } else {
        w_choices_pick(&mut rng)
    };
    let wb = pick_wb(w, &mut rng);
    let mut input = GenInput {
        w,
        wb,
        a,
        b,
        expr,
        seed,
    };
    input.normalize();
    input
}

fn w_choices_pick(rng: &mut Rng) -> u32 {
    WIDTH_CHOICES[rng.usize(0..WIDTH_CHOICES.len())]
}
