//! Generator scenario MV (.mv) — testcase → Maria-MV, bukan SV mentah.
//!
//! Backend MvMediated: fuzzer menghasilkan TEKS `.mv` canonical (bukan SV),
//! di-lower ke HDL oleh `mv_lower` sebelum masuk Maria. Generator ini
//! "100% valid by construction" dalam grammar .mv (sama seperti gen.rs untuk
//! SV): semua sinyal dideklarasikan duluan, assignment operator sesuai konteks
//! blok (seq=`<=`, comb/always/initial=`=`), lebar RHS ≤ lebar LHS (E2002),
//! dan ref hanya ke sinyal ter-deklarasi (E2001).
//!
//! 1 file = 1 tanggung jawab: hanya generate `.mv` text.
//!
//! Ragam struktural (target kedalaman, bukan template):
//! - topologi port: lebar, signed, jumlah
//! - chain dependensi: input → expr → comb sig → seq reg → FSM state → out
//! - clock/reset: async / sync / negedge
//! - generate: genfor + inst child, genif
//! - state machine: enum + case dalam seq
//! - 4-state: fill 'x/'z, X-injeksi stimulus

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::Rng;

/// Nama sinyal yang selalu ter-deklarasi (port).
const BASE_W: usize = 8;

/// Satu sinyal ter-deklarasi (symbol table).
#[derive(Clone)]
struct Sig {
    name: String,
    width: usize,
}

/// Konteks blok — menentukan operator assignment yang sah.
#[derive(Clone, Copy, PartialEq)]
enum Blk {
    Comb, // blocking =
    Seq,  // non-blocking <=
    Tb,   // initial/final — blocking, boleh drive port input
}

struct Builder<'a> {
    rng: &'a mut StdRng,
    seq: u64,
    sigs: Vec<Sig>,
}

impl<'a> Builder<'a> {
    fn fresh(&mut self, prefix: &str) -> String {
        self.seq += 1;
        format!("{prefix}{}", self.seq)
    }

    /// Ref/atom ekspresi: sinyal ter-deklarasi atau literal lebar `w`.
    fn atom(&mut self, w: usize) -> String {
        if self.sigs.is_empty() || self.rng.gen_bool(0.3) {
            return self.literal(w);
        }
        let s = self.sigs.choose(self.rng).unwrap().clone();
        if s.width >= w || self.rng.gen_bool(0.7) {
            s.name
        } else {
            // Sinyal lebih sempit dari konteks — aman (widening diizinkan).
            s.name
        }
    }

    fn literal(&mut self, w: usize) -> String {
        match self.rng.gen_range(0..5u32) {
            0 => format!("{w}'d{}", self.rng.gen_range(0..(1u64 << w.min(16)))),
            1 => format!("{w}'h{:x}", self.rng.gen_range(0..(1u64 << w.min(16)))),
            2 => "'0".to_string(),
            3 => "'1".to_string(),
            _ => "'x".to_string(),
        }
    }

    /// Ekspresi rekursif lebar ~w — operator aman utk lebar (shift tidak
    /// melebar; comparison/ternary 1-bit RHS = widening diizinkan).
    fn expr(&mut self, w: usize, d: u8) -> String {
        if d == 0 || self.rng.gen_bool(0.45) {
            return self.atom(w);
        }
        match self.rng.gen_range(0..8u32) {
            0 => {
                let op = *["+", "-", "&", "|", "^", "<<", ">>"].choose(self.rng).unwrap();
                format!("({}) {op} ({})", self.expr(w, d - 1), self.expr(w, d - 1))
            }
            1 => format!("({} == {})", self.expr(w, d - 1), self.expr(w, d - 1)),
            2 => format!("({} ? {} : {})", self.expr(w.min(4), d - 1), self.expr(w, d - 1), self.expr(w, d - 1)),
            3 => format!("~({})", self.expr(w, d - 1)),
            4 => format!("-({})", self.expr(w, d - 1)),
            5 => format!("({}) << (2)", self.expr(w, d - 1)),
            6 => format!("({}) >>> (1)", self.expr(w, d - 1)),
            _ => format!("($clog2({w}))"),
        }
    }

    /// Assignment ke target.
    fn assign(&mut self, blk: Blk, w: usize, tgt: &str, d: u8) -> String {
        let op = if blk == Blk::Seq { "<=" } else { "=" };
        format!("{tgt} {op} {}", self.expr(w, d))
    }

    /// Statement acak (bukan leaf) — if/case/seq-stmts utk blok.
    fn stmt(&mut self, blk: Blk, w: usize, depth: u8) -> String {
        let tgt = |b: &mut Self| -> String {
            let cands: Vec<String> = b
                .sigs
                .iter()
                .filter(|s| !s.name.starts_with('i') || s.name.starts_with("iv"))
                .map(|s| s.name.clone())
                .collect();
            if cands.is_empty() {
                let n = b.fresh("r");
                b.sigs.push(Sig { name: n.clone(), width: w });
                n
            } else {
                cands.choose(b.rng).unwrap().clone()
            }
        };
        if depth == 0 {
            let t = tgt(self);
            return self.assign(blk, w, &t, 1);
        }
        match self.rng.gen_range(0..6u32) {
            0 => {
                let t = tgt(self);
                self.assign(blk, w, &t, 2)
            }
            1 => {
                let c = self.expr(w.min(4), 1);
                let s1 = self.stmt(blk, w, depth - 1);
                let s2 = self.stmt(blk, w, depth - 1);
                format!("if ({c}) {{\n{s1}\n}} else {{\n{s2}\n}}")
            }
            2 => {
                let c = self.expr(w.min(4), 1);
                let t = tgt(self);
                let op = if blk == Blk::Seq { "<=" } else { "=" };
                let mut items = String::new();
                let n = self.rng.gen_range(1..=3);
                for _ in 0..n {
                    let v = self.rng.gen_range(0..(1u64 << 4));
                    items.push_str(&format!("    {v}'d{v}: {} {op} {}\n", t, self.expr(w, 1)));
                }
                format!(
                    "case ({c}) {{\n{items}    default: {} {op} '0\n}}",
                    t
                )
            }
            3 => {
                // `for i in 0..3` — var loop TIDAK di-module-scope: jangan
                // di-push ke sigs (ref luar loop = E2001 undefined signal).
                let cn = self.fresh("lc");
                let b2 = self.stmt(blk, w, depth - 1);
                format!("for {cn} in 0..3 {{\n{b2}\n}}")
            }
            4 => {
                let t = tgt(self);
                if blk == Blk::Seq {
                    format!("{} <= '0", t)
                } else {
                    format!("{} = '1", t)
                }
            }
            _ => {
                let t = tgt(self);
                self.assign(blk, w, &t, 1)
            }
        }
    }
}

/// Bentuk (shape) generator — tiap shape menggeser konstruk dominan.
fn gen_module(b: &mut Builder, shape: usize, nam: &str) -> String {
    let w = BASE_W;
    let mut s = String::new();

    // ── Header ──
    s.push_str(&format!("module {nam} {{\n"));

    // ── Ports ──
    s.push_str("    in clk : bit\n");
    s.push_str("    in rst_n : bit\n");
    s.push_str(&format!("    in a : logic[{}-1:0]\n", w));
    s.push_str(&format!("    in b : logic[{}-1:0]\n", w));
    s.push_str(&format!("    out y : logic[{}-1:0]\n", w));
    b.sigs.push(Sig { name: "a".into(), width: w });
    b.sigs.push(Sig { name: "b".into(), width: w });
    b.sigs.push(Sig { name: "y".into(), width: w });

    // ── Sinyal internal: chain input → comb → reg ──
    let t = b.fresh("t");
    b.sigs.push(Sig { name: t.clone(), width: w });
    s.push_str(&format!("    sig {t} : logic[{}-1:0]\n", w));
    for i in 0..2 {
        let r = b.fresh("r");
        b.sigs.push(Sig { name: r.clone(), width: w });
        s.push_str(&format!("    sig {r} : logic[{}-1:0]\n", w));
        let _ = i;
    }
    if shape % 3 == 1 {
        let sr = b.fresh("s");
        b.sigs.push(Sig { name: sr.clone(), width: w });
        s.push_str(&format!("    sig {sr} : signed logic[{}-1:0]\n", w));
    }

    // ── Const ──
    s.push_str(&format!("    const C = {}\n", b.rng.gen_range(1..=7)));

    // ── Comb: t = chain expr; y = t ──
    s.push_str(&format!(
        "    comb {{\n        {} = {}\n        y = ({} + C)\n    }}\n",
        t,
        b.expr(w, 3),
        t
    ));

    // ── Seq: akumulasi / FSM state (variasi per shape) ──
    let rname = b.sigs.iter().find(|x| x.name.starts_with('r')).unwrap().name.clone();
    let seq_body = if shape % 2 == 1 {
        // Shape ganjil: statement acak (if/case/assign) — kedalaman struktural.
        b.stmt(Blk::Seq, w, 2)
    } else {
        // Shape genap: akumulasi deterministik `r <= (r) & (t)`.
        format!("{} <= ({}) & ({})", rname, rname, t)
    };
    s.push_str(&format!(
        "    seq(clk, rst_n) {{\n        if (!rst_n) {{\n            {rname} <= '0\n        }} else {{\n            {seq_body}\n        }}\n    }}\n"
    ));

    // ── Clock ──
    s.push_str("    initial {\n        clk = '0\n        forever {\n            #5 clk = ~clk\n        }\n    }\n");

    // ── Stimulus + X-injeksi ──
    let va = b.rng.gen_range(0..(1u64 << 8));
    let vb = b.rng.gen_range(0..(1u64 << 8));
    s.push_str(&format!(
        "    initial {{\n        rst_n = '0\n        a = 8'd{va}\n        b = 8'd{vb}\n        #7 rst_n = '1\n        #10 a = 8'd{}\n    }}\n",
        b.rng.gen_range(0..(1u64 << 8))
    ));
    // X/Z injeksi (4-state)
    if b.rng.gen_bool(0.3) {
        let xv = if b.rng.gen_bool(0.5) { "'x" } else { "'z" };
        s.push_str(&format!("    initial {{\n        #25 b = {xv}\n    }}\n"));
    }

    // ── Assert (tidak X di akhir) ──
    s.push_str("    initial {\n        #80\n        assert (y !== 'x) $info(\"PASS\")\n        $finish\n    }\n");

    s.push_str("}\n");
    s
}

/// Hasilkan satu modul `.mv` (valid by construction).
pub fn gen_random_module(rng: &mut StdRng, shape: usize) -> String {
    let mut b = Builder {
        rng,
        seq: 0,
        sigs: Vec::new(),
    };
    let nam = format!("fz_mv_{}", shape);
    gen_module(&mut b, shape, &nam)
}

/// Jumlah shape.
pub const NUM_MV_SHAPES: usize = 6;

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn gen_modules_parse_and_check() {
        let mut rng = StdRng::seed_from_u64(7);
        for k in 0..NUM_MV_SHAPES {
            let src = gen_random_module(&mut rng, k);
            assert!(!src.is_empty());
            // Parse & type-check maria-mv harus sukses (valid by construction).
            let f = maria_api::mv::parser::parse(&src)
                .unwrap_or_else(|e| panic!("shape {k} parse gagal: {}.\n{src}", e.format()));
            maria_api::mv::check::check(&f)
                .unwrap_or_else(|e| panic!("shape {k} check gagal: {}.\n{src}", e.format()));
        }
    }

    #[test]
    fn gen_modules_diverse() {
        let mut rng = StdRng::seed_from_u64(11);
        let mut seen = std::collections::HashSet::new();
        for k in 0..NUM_MV_SHAPES {
            seen.insert(gen_random_module(&mut rng, k));
        }
        assert_eq!(seen.len(), NUM_MV_SHAPES, "shape harus menghasilkan modul unik");
    }
}