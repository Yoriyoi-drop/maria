//! Mutasi struktur-aware.
//!
//! Paper #10 (Superion): mutasi memakai struktur (AST/token), bukan byte.
//!   Di sini: operator diganti sesama golongan (arith↔bitwise, shift),
//!   literals dibalik (0↔1, 1'bx↔1'bz), nilai/shape dipertukarkan.
//! Paper #12 (Code Fragments): splice serpihan corpus nyata ke dalam seed.
//! Paper #9/#11: insertion snippet dari grammar SV.

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::Rng;

use crate::corpus::Corpus;
use crate::grammar;

/// Golongan operator — mutasi tukar antar anggota golongan (Paper #10).
pub const OP_GROUPS: &[&[&str]] = &[
    &["+", "-"],
    &["&", "|", "^"],
    &["<<", ">>"],
    &["==", "!=", ">", "<", ">=", "<="],
];

pub const LITERALS: &[&str] = &["'0", "'1", "'x", "'z", "1'b0", "1'b1", "2'd0", "2'd1", "2'd2"];

/// Terapkan satu mutasi acak pada source. Operator mutasi dipilih random.
pub fn mutate(rng: &mut StdRng, source: &str, corpus: &Corpus) -> String {
    let op = rng.gen_range(0..6u32);
    match op {
        0 => replace_operator(rng, source),
        1 => flip_literal(rng, source),
        2 => splice_corpus_fragment(rng, source, corpus),
        3 => insert_grammar_decl(rng, source),
        4 => insert_grammar_body(rng, source),
        5 => duplicate_line(rng, source),
        _ => source.to_string(),
    }
}

/// Ganti operator ke anggota golongan lain yang ada di seed (Paper #10).
/// Bila tidak ada operator dikenal, sisipkan satu.
pub fn replace_operator(rng: &mut StdRng, source: &str) -> String {
    let mut candidates: Vec<usize> = Vec::new();
    for group in OP_GROUPS {
        for op in *group {
            if let Some(pos) = source.find(op) {
                candidates.push(pos);
            }
        }
    }
    if candidates.is_empty() {
        // Tidak ada operator → sisipkan operasi sederhana di akhir modul.
        let mut s = source.to_string();
        if let Some(end) = s.rfind("endmodule") {
            s.insert_str(end, "  assign fz_q0 = fz_a0 + 1'b1;\n");
        } else {
            s.push_str("  assign fz_q0 = fz_a0 + 1'b1;\n");
        }
        return s;
    }
    let pick = *candidates.choose(rng).unwrap();
    let mut best: Option<(&str, usize)> = None;
    for group in OP_GROUPS {
        for op in *group {
            if let Some(pos) = source[pick..].find(op) {
                let abs = pick + pos;
                if best.map_or(true, |(_, bp)| abs < bp) {
                    best = Some((op, abs));
                }
            }
        }
    }
    let (oldop, pos) = best.unwrap();
    // Pilih pengganti di golongan sama (bukan dirinya sendiri).
    let group = OP_GROUPS
        .iter()
        .find(|g| g.contains(&oldop))
        .copied()
        .unwrap_or(&["+"]);
    let mut cand: Vec<&str> = group.iter().copied().filter(|o| *o != oldop).collect();
    if cand.is_empty() {
        cand = vec!["~"];
    }
    let newop = *cand.choose(rng).unwrap();
    let mut s = source.to_string();
    s.replace_range(pos..pos + oldop.len(), newop);
    s
}

/// Balik literal 4-state (Paper #10: mutasi nilai; Paper #3: eksplorasi
/// edge-case X/Z propagation).
pub fn flip_literal(rng: &mut StdRng, source: &str) -> String {
    let flipped = [("'0", "'1"), ("'1", "'0"), ("'x", "'z"), ("'z", "'x")];
    let picks: Vec<(&str, &str)> = flipped
        .iter()
        .filter(|(a, _)| source.contains(a))
        .copied()
        .collect();
    if picks.is_empty() {
        // Tidak ada literal → sisipkan literal X (stressor 4-state).
        let mut s = source.to_string();
        if let Some(end) = s.rfind("endmodule") {
            s.insert_str(end, "  assign fz_q1 = 1'bx;\n");
        } else {
            s.push_str("  assign fz_q1 = 1'bx;\n");
        }
        return s;
    }
    let (from, to) = *picks.choose(rng).unwrap();
    let mut s = source.to_string();
    if let Some(pos) = s.find(from) {
        s.replace_range(pos..pos + from.len(), to);
    }
    s
}

/// Splice serpihan corpus nyata (#12) — sisip sebelum `endmodule`.
pub fn splice_corpus_fragment(rng: &mut StdRng, source: &str, corpus: &Corpus) -> String {
    let Some(frag) = corpus.random_fragment(rng) else {
        return insert_grammar_body(rng, source);
    };
    let mut s = source.to_string();
    if let Some(end) = s.rfind("endmodule") {
        s.insert_str(end, &format!("  // fuzz fragment (Paper #12)\n  {}\n", frag));
    } else {
        s.push_str(&format!("  // fuzz fragment (Paper #12)\n  {}\n", frag));
    }
    s
}

/// Insert deklarasi dari grammar (#11/#15).
pub fn insert_grammar_decl(rng: &mut StdRng, source: &str) -> String {
    let decl = grammar::random_decl(rng);
    let mut s = source.to_string();
    if let Some(end) = s.rfind("endmodule") {
        s.insert_str(end, &format!("  {}\n", decl));
    } else {
        s.push_str(&format!("  {}\n", decl));
    }
    s
}

/// Insert blok prosedural dari grammar (#9/#11) — sebelum `endmodule`.
pub fn insert_grammar_body(rng: &mut StdRng, source: &str) -> String {
    let snip = grammar::random_body_snippet(rng);
    let mut s = source.to_string();
    if let Some(end) = s.rfind("endmodule") {
        s.insert_str(end, &format!("  {}\n", snip));
    } else {
        s.push_str(&format!("  {}\n", snip));
    }
    s
}

/// Duplikasi baris acak (mutasi dasar fuzzing, AFL lineage; Paper #3).
pub fn duplicate_line(rng: &mut StdRng, source: &str) -> String {
    let lines: Vec<&str> = source.lines().collect();
    if lines.len() < 2 {
        return source.to_string();
    }
    let target = lines.choose(rng).copied().unwrap_or("").trim();
    if target.is_empty() {
        return source.to_string();
    }
    // Jangan duplikasi baris `endmodule`/`module` — merusak struktur parah.
    if target.starts_with("endmodule") || target.starts_with("module") {
        return source.to_string();
    }
    let mut s = source.to_string();
    if let Some(pos) = s.find(target) {
        s.insert_str(pos + target.len(), &format!("\n{}", target));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    const SEED: &str = "module top;\n  logic [7:0] y;\n  assign y = a + b;\nendmodule\n";

    #[test]
    fn replace_operator_changes_something() {
        let mut rng = StdRng::seed_from_u64(1);
        let out = replace_operator(&mut rng, SEED);
        assert_ne!(out, SEED, "mutasi harus mengubah source");
        assert!(out.contains("endmodule"));
    }

    #[test]
    fn insert_body_keeps_module() {
        let mut rng = StdRng::seed_from_u64(2);
        let out = insert_grammar_body(&mut rng, SEED);
        assert!(out.starts_with("module top"));
        assert!(out.contains("endmodule"));
        assert!(out.len() > SEED.len());
    }

    #[test]
    fn duplicate_line_grows_source() {
        let mut rng = StdRng::seed_from_u64(3);
        let out = duplicate_line(&mut rng, SEED);
        assert!(out.len() >= SEED.len());
    }

    #[test]
    fn no_operator_seed_gets_assign() {
        let mut rng = StdRng::seed_from_u64(4);
        let plain = "module top;\nendmodule\n";
        let out = replace_operator(&mut rng, plain);
        assert!(out.contains("assign"));
    }
}