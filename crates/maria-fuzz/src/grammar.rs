//! Grammar SystemVerilog token-level — dasar generate & mutate terstruktur.
//!
//! Paper #9 (NAUTILUS): grammar kontekstual — sintaksis valid dibuat dari
//!   produksi kecil, bukan karakter acak.
//! Paper #11 (Grammarinator): ekspansi produksi berulang — serpihan statement
//!   bisa digabung berulang menjadi blok lebih besar.

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::Rng;

/// Kata kunci deklarasi (untuk snippet/insertion).
pub const DECL_KEYWORDS: &[&str] = &["logic", "reg", "wire", "integer"];

/// Kata kunci blok prosedural.
pub const BLOCK_KEYWORDS: &[&str] = &[
    "always_ff",
    "always_comb",
    "always_latch",
    "always @(*)",
    "initial",
];

/// Kata kunci pernyataan kontrol.
pub const STMT_KEYWORDS: &[&str] = &["if", "else", "case", "for", "while", "repeat", "forever"];

/// Fungsi sistem yang dikenal engine maria.
pub const SYS_FUNCS: &[&str] = &[
    "$display", "$clog2", "$bits", "$size", "$left", "$right",
    "$urandom", "$random", "$fopen", "$fwrite", "$fclose",
    "$dumpfile", "$dumpvars", "$finish", "$stop",
    "$cast", "$typename", "$bits", "$countbits",
    "$onehot", "$isonehot", "$isunknown", "$isunknown",
    "$realtime", "$time", "$stime",
];

/// Apakah token adalah kata kunci SV yang dikenal fuzzer.
pub fn is_keyword(word: &str) -> bool {
    DECL_KEYWORDS.contains(&word)
        || BLOCK_KEYWORDS.contains(&word)
        || STMT_KEYWORDS.contains(&word)
        || SYS_FUNCS.contains(&word)
        || ["module", "endmodule", "assign", "input", "output", "begin", "end"].contains(&word)
}

/// Ekspansi produksi: serpihan deklarasi acak (Paper #11 ekspansi berulang).
pub fn random_decl(rng: &mut StdRng) -> String {
    let kw = *DECL_KEYWORDS.choose(rng).unwrap();
    let w = [1usize, 2, 4, 8, 16].choose(rng).unwrap();
    format!("{} [{}-1:0] fz_{};", kw, w, rng.gen_range(0..1000u32))
}

/// Ekspansi produksi: serpihan ekspresi acak bertipe (Paper #11/#15).
pub fn random_expr(rng: &mut StdRng) -> String {
    let a = format!("fz_a{}", rng.gen_range(0..4u32));
    let b = format!("fz_b{}", rng.gen_range(0..4u32));
    let op = *["+", "-", "&", "|", "^", "<<", ">>"].choose(rng).unwrap();
    format!("{} {} {}", a, op, b)
}

/// Ekspansi produksi: serpihan isi blok (`begin ... end`) — bisa digabung
/// berulang (Grammarinator-style). Rujuk simbol fuzzer `fz_*` agar mandiri.
/// Palet DIVERSE (bukan template): casez/casex, fork/join, event-control,
/// repeat/while, concat/replication, ternary, NBA-vs-blocking — variasi
/// semantik untuk mutasi & splice (sumber: semantic.rs P0 areas).
pub fn random_body_snippet(rng: &mut StdRng) -> String {
    let kw = *BLOCK_KEYWORDS.choose(rng).unwrap();
    let target = format!("fz_y{}", rng.gen_range(0..4u32));
    let expr = random_expr(rng);
    match rng.gen_range(0..12u32) {
        0 => format!("{} begin\n  {} = {};\nend", kw, target, expr),
        1 => format!(
            "{} begin\n  if (fz_sel{}) {} = {};\n  else {} = '0;\nend",
            kw,
            rng.gen_range(0..2u32),
            target,
            expr,
            format!("{}$clog2", target)
        ),
        2 => format!(
            "{} begin\n  case (fz_sel{})\n    1'd0: {} = {};\n    default: {} = ~{};\n  endcase\nend",
            kw,
            rng.gen_range(0..2u32),
            target,
            expr,
            target,
            target
        ),
        3 => format!(
            "{} begin\n  for (fz_i{} = 0; fz_i{} < 4; fz_i{} = fz_i{} + 1) begin\n    {} = {};\n  end\nend",
            kw,
            rng.gen_range(0..4u32),
            rng.gen_range(0..4u32),
            rng.gen_range(0..4u32),
            rng.gen_range(0..4u32),
            target,
            expr,
        ),
        // ── Perluasan: konstruk menekan area P0 (bukan template, variasi
        //    random per panggilan; serpihan di-splice ke seed apa pun). ──
        4 => format!(
            "{} begin\n  casez (fz_sel{})\n    2'b0?: {} = {};\n    2'b1?: {} = ~{};\n    default: {} = 'x;\n  endcase\nend",
            kw,
            rng.gen_range(0..2u32),
            target,
            expr,
            target,
            expr,
            target
        ),
        5 => format!(
            "{} begin\n  fork\n    #{} {} = {};\n    #{} {} = {};\n  join_none\nend",
            kw,
            rng.gen_range(1..=5),
            target,
            expr,
            rng.gen_range(1..=5),
            target,
            expr
        ),
        6 => format!(
            "{} begin\n  @(posedge fz_ck{});\n  #{} {} = {};\nend",
            kw,
            rng.gen_range(0..2u32),
            rng.gen_range(0..=3),
            target,
            expr
        ),
        7 => format!(
            "{} begin\n  repeat ({}) begin\n    {} <= {};\n  end\nend",
            kw,
            rng.gen_range(1..=4),
            target,
            expr
        ),
        8 => format!(
            "{} begin\n  {}= {{{}, {}}};\nend",
            kw,
            target,
            expr,
            expr
        ),
        9 => format!(
            "{} begin\n  {} = {} ? {} : '0;\n  #0;\n  {} = {};\nend",
            kw,
            target,
            expr,
            expr,
            target,
            expr
        ),
        10 => format!(
            "{} begin\n  while (fz_i{} < 3) begin\n    {} = {};\n    fz_i{} = fz_i{} + 1;\n  end\nend",
            kw,
            rng.gen_range(0..4u32),
            target,
            expr,
            rng.gen_range(0..4u32),
            rng.gen_range(0..4u32)
        ),
        _ => format!(
            "{} begin\n  {} <= {};\n  {} = {};\nend",
            kw,
            target,
            expr,
            target,
            expr
        ),
    }
}

/// Serpihan maksimal satu baris (untuk operasi splice ringan).
pub fn random_line_snippet(rng: &mut StdRng) -> String {
    match rng.gen_range(0..6u32) {
        0 => random_decl(rng),
        1 => format!("assign fz_y{} = {};", rng.gen_range(0..4u32), random_expr(rng)),
        2 => format!("if (fz_sel{}) begin\n  {}", rng.gen_range(0..2u32), random_body_snippet(rng)),
        // ── Perluasan splice ringan: X/Z, signed cast, NBA, part-select. ──
        3 => format!("assign fz_y{} = {}'x;", rng.gen_range(0..4u32), [1u32, 2, 4, 8].choose(rng).unwrap()),
        4 => format!("assign fz_y{} = signed'({});", rng.gen_range(0..4u32), random_expr(rng)),
        _ => format!("assign fz_y{} = fz_a{}[{} +: {}];", rng.gen_range(0..4u32), rng.gen_range(0..4u32), rng.gen_range(0..8u32), rng.gen_range(1..=4u32)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn snippets_nonempty_and_keywords() {
        let mut rng = StdRng::seed_from_u64(7);
        for _ in 0..50 {
            let d = random_decl(&mut rng);
            let b = random_body_snippet(&mut rng);
            let l = random_line_snippet(&mut rng);
            assert!(!d.is_empty());
            assert!(!b.is_empty());
            assert!(!l.is_empty());
        }
    }

    #[test]
    fn keyword_lists_cover() {
        assert!(is_keyword("module"));
        assert!(is_keyword("always_ff"));
        assert!(is_keyword("$clog2"));
        assert!(!is_keyword("not_a_keyword_xyz"));
    }
}