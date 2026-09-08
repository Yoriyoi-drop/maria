//! Mutasi struktur-aware.
//!
//! Paper #10 (Superion): mutasi memakai struktur (AST/token), bukan byte.
//!   Di sini: operator diganti sesama golongan (arith↔bitwise, shift),
//!   literals dibalik (0↔1, 1'bx↔1'bz), nilai/shape dipertukarkan.
//! Paper #12 (Code Fragments): splice serpihan corpus nyata ke dalam seed.
//! Paper #9/#11: insertion snippet dari grammar SV.
//! Paper #7 (VUzzer): mutasi lebar bit & fitur dataflow.
//! Paper #15 (YARPGen): mutasi type-aware — signed/lebar/value.

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::Rng;

use crate::corpus::Corpus;
use crate::grammar;

/// Regex ringan untuk deteksi lebar bit pada deklarasi `logic [A:B] nama`
/// dan `parameter`/`localparam` — bukan dependensi regex, cukup substring.

/// Ambil semua integer literal dari string.
fn int_literals(s: &str) -> Vec<(usize, usize, u64)> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if (bytes[i] >= b'0' && bytes[i] <= b'9') || bytes[i] == b'"' {
            let start = i;
            let mut j = i;
            while j < bytes.len() && bytes[j] >= b'0' && bytes[j] <= b'9' {
                j += 1;
            }
            if j > start {
                let num = std::str::from_utf8(&bytes[start..j]).unwrap_or("").parse::<u64>().unwrap_or(0);
                out.push((start, j, num));
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Golongan operator — mutasi tukar antar anggota golongan (Paper #10).
pub const OP_GROUPS: &[&[&str]] = &[
    &["+", "-"],
    &["&", "|", "^"],
    &["<<", ">>"],
    &["==", "!=", ">", "<", ">=", "<="],
];

pub const LITERALS: &[&str] = &["'0", "'1", "'x", "'z", "1'b0", "1'b1", "2'd0", "2'd1", "2'd2"];

/// Terapkan satu mutasi acak pada source. Operator mutasi dipilih random.
/// Jumlah operator mutasi (id 0..=12).
pub const NUM_OPS: usize = 13;

/// Statistik & bobot adaptif per operator mutasi (audit GAP-3): op yang
/// sering memicu novelty (fitur eksekusi baru) naik bobotnya; yang mandek
/// meluruh — palet tidak lagi uniform. Bobot awal seragam 1.0.
#[derive(Debug, Clone)]
pub struct OpStats {
    pub attempts: [u64; NUM_OPS],
    pub novels: [u64; NUM_OPS],
    weights: [f64; NUM_OPS],
}

impl Default for OpStats {
    fn default() -> Self {
        OpStats {
            attempts: [0; NUM_OPS],
            novels: [0; NUM_OPS],
            weights: [1.0; NUM_OPS],
        }
    }
}

impl OpStats {
    /// Catat pemilihan op (dipanggil `mutate` saat op dipilih).
    pub fn record_attempt(&mut self, op: usize) {
        if op < NUM_OPS {
            self.attempts[op] += 1;
        }
    }

    /// Catat hasil iterasi utk op: novel → bobot naik (×1.25, cap 20);
    /// tidak → meluruh (×0.995, floor 0.2).
    pub fn record_outcome(&mut self, op: usize, novel: bool) {
        if op >= NUM_OPS {
            return;
        }
        if novel {
            self.novels[op] += 1;
            self.weights[op] = (self.weights[op] * 1.25).min(20.0);
        } else {
            self.weights[op] = (self.weights[op] * 0.995).max(0.2);
        }
    }

    /// Gabung statistik (worker paralel / report merge).
    pub fn merge(&mut self, other: &OpStats) {
        for i in 0..NUM_OPS {
            self.attempts[i] += other.attempts[i];
            self.novels[i] += other.novels[i];
            self.weights[i] = self.weights[i].max(other.weights[i]);
        }
    }
}

/// Pilih op mutasi — berbobot adaptif (floor 0.2 × 13 > 0, total selalu > 0).
fn pick_op(rng: &mut StdRng, stats: &OpStats) -> usize {
    let total: f64 = stats.weights.iter().sum();
    let mut pick = rng.gen_range(0.0..total);
    for (i, w) in stats.weights.iter().enumerate() {
        if pick < *w {
            return i;
        }
        pick -= *w;
    }
    NUM_OPS - 1
}

/// Terapkan SATU operator mutasi (audit GAP-1: rantai ganda 1..4 × 1..3 =
/// 1..12 mutasi per iterasi → mayoritas child rusak-sintaks; mutasi
/// incremental kecil jauh lebih viable — rantai eksternal 1..3 di lib.rs).
/// Pemilihan op berbobot adaptif (`stats`, GAP-3). Kembalikan (op_id, src).
pub fn mutate(
    rng: &mut StdRng,
    source: &str,
    corpus: &Corpus,
    stats: &mut OpStats,
) -> (usize, String) {
    let op = pick_op(rng, stats);
    stats.record_attempt(op);
    let out = match op {
        0 => replace_operator(rng, source),
        1 => flip_literal(rng, source),
        2 => splice_corpus_fragment(rng, source, corpus),
        3 => insert_grammar_decl(rng, source),
        4 => insert_grammar_body(rng, source),
        5 => duplicate_line(rng, source),
        // Mutasi lebar bit (Paper #7 VUzzer — eksplorasi lebar).
        6 => change_width(rng, source),
        // Sisip part-select out-of-range (stress test per-bit §11.5.1).
        7 => insert_partselect_oob(rng, source),
        // Ganti parameter/const value (Paper #15 YARPGen — type-aware).
        8 => tweak_params(rng, source),
        // Manipulasi initial/reset (race condition stressor).
        9 => tweak_initial(rng, source),
        // Property-oracle mirror (Paper #2/#3/#14, oracle #5):
        // `_fz_viol = (lhs !== rhs)` — engine konsisten → selalu 0.
        10 => insert_assert_mirror(rng, source),
        // Property-oracle berbasis assert (Paper #14, oracle #5):
        // `assert (tempA === tempB) else $fatal`. Fail = bug eval.
        11 => insert_assert_oracle(rng, source),
        // Interface-oracle (stress elaborator interface/hierarki/modport).
        12 => insert_interface(rng, source),
        _ => source.to_string(),
    };
    (op, out)
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

/// NEW: ubah lebar deklarasi bit di beberapa tempat (Paper #7 VUzzer —
/// eksplorasi lebar bit). Cari pola `[N-1:0]` / `[M:0]` dan ganti N/M
/// dengan nilai lain (plus/minus kecil). Hindari merusak sintaks parah.
pub fn change_width(rng: &mut StdRng, source: &str) -> String {
    // Cari `[ <num> : 0]` atau `[ <num> - 1 : 0]` di deklarasi sinyal/port.
    // Pola sederhana: cari `:` lalu lihat ada angka di kirinya.
    let bytes = source.as_bytes();
    let mut positions: Vec<(usize, usize, u64)> = Vec::new();
    for (start, end, num) in int_literals(source) {
        // Cek konteks: apakah ini bagian dari `[num-1:0]` atau `[num:0]`?
        // Slice byte-safe: start/end adalah batas char (literal ASCII digit),
        // tapi start-4 / end+4 bisa memotong char multibyte atau melewati
        // akhir string → pakai from_utf8 fallback agar tidak panic.
        let before_start = start.saturating_sub(4);
        let after_end = (end + 4).min(bytes.len());
        let before = std::str::from_utf8(&bytes[before_start..start]).unwrap_or_default();
        let after = std::str::from_utf8(&bytes[end..after_end]).unwrap_or_default();
        // Pola: `- 1 : 0` atau langsung `: 0`
        let is_width = before.contains('-')
            || before.contains(']')
            || after.starts_with(": 0")
            || after.starts_with(":0");
        if is_width && num > 0 && num < 1024 {
            positions.push((start, end, num));
        }
    }
    if positions.is_empty() {
        // Fallback: ganti literal umum di source.
        let literals = int_literals(source);
        if literals.is_empty() {
            return source.to_string();
        }
        let (start, end, num) = *literals.choose(rng).unwrap();
        if num == 0 {
            return source.to_string();
        }
        let delta = rng.gen_range(1..=num.min(8)) as i64;
        let sign = if rng.gen_bool(0.5) { -1 } else { 1 };
        let new_num = (num as i64 + sign * delta).max(1) as u64;
        let mut s = source.to_string();
        s.replace_range(start..end, &new_num.to_string());
        return s;
    }
    let (start, end, num) = *positions.choose(rng).unwrap();
    let delta = rng.gen_range(1..=num.min(8)) as i64;
    let sign = if rng.gen_bool(0.5) { -1 } else { 1 };
    let new_num = (num as i64 + sign * delta).max(1) as u64;
    let mut s = source.to_string();
    s.replace_range(start..end, &new_num.to_string());
    s
}

/// NEW: sisipkan part-select out-of-range (Paper #11/#7 — stress test
/// evaluasi per-bit §11.5.1 yang sebelumnya jadi bug EMI). Sisipkan
/// `assign _fuzz_ps = sig[N+:M]` dengan N/M acak sebelum endmodule.
pub fn insert_partselect_oob(rng: &mut StdRng, source: &str) -> String {
    // Cari nama sinyal yang ada di source.
    let sig_names: Vec<String> = source
        .lines()
        .filter(|l| {
            l.contains("logic")
                || l.contains("wire")
                || l.contains("reg")
                || l.contains("input")
                || l.contains("output")
        })
        .filter_map(|l| {
            // Ambil nama diakhir baris (setelah tipe & [dim]); buang koma
            // pemisah port-list agar nama sinyal utuh (audit GAP-1).
            let mut parts: Vec<&str> = l.split_whitespace().collect();
            parts.retain(|p| {
                !p.is_empty()
                    && !p.starts_with('[')
                    && !p.contains(':')
                    && !p.contains(';')
                    && !p.contains(',')
            });
            let tail = parts.last()?.trim_end_matches(',').to_string();
            if tail.is_empty()
                || tail == "logic" || tail == "wire" || tail == "reg"
                || tail == "input" || tail == "output"
            {
                None
            } else {
                Some(tail)
            }
        })
        .collect();
    let Some(sig) = sig_names.choose(rng) else {
        return source.to_string();
    };
    let msb: u64 = rng.gen_range(4..=32);
    let width: u64 = rng.gen_range(2..=8);
    let w_minus_1 = if width > 0 { width - 1 } else { 0 };
    let assign_name = format!("_fuzz_pa_{}", rng.gen_range(0..99999));
    let snippet = format!(
        "  // fuzz part-select OOB (Paper #7)\n  wire [{w_minus_1}:0] {an};
  assign {an} = {sig}[{msb}:{msb}];
",
        an = assign_name, sig = sig, msb = msb, w_minus_1 = w_minus_1
    );
    let mut s = source.to_string();
    if let Some(end) = s.rfind("endmodule") {
        s.insert_str(end, &snippet);
    } else {
        s.push_str(&snippet);
    }
    s
}

/// NEW: ganti nilai parameter/const (Paper #15 YARPGen — type-aware value).
/// Cari `parameter NAME = VAL` / `localparam NAME = VAL` dan ubah VAL.
pub fn tweak_params(rng: &mut StdRng, source: &str) -> String {
    let literals = int_literals(source);
    if literals.is_empty() {
        return source.to_string();
    }
    let (start, end, num) = *literals.choose(rng).unwrap();
    if num == 0 {
        // Ganti 0 dengan 1 atau sebuah nilai kecil.
        let new_val = if rng.gen_bool(0.5) { 1u64 } else { rng.gen_range(1..=16) };
        let mut s = source.to_string();
        s.replace_range(start..end, &new_val.to_string());
        return s;
    }
    let delta = rng.gen_range(1..=num.min(32)) as i64;
    let sign = if rng.gen_bool(0.5) { -1 } else { 1 };
    let new_num = (num as i64 + sign * delta).max(1) as u64;
    let mut s = source.to_string();
    s.replace_range(start..end, &new_num.to_string());
    s
}

/// NEW: stress test concat dengan variasi lebar (Paper #7 — dataflow).
pub fn stress_concat(rng: &mut StdRng, source: &str) -> String {
    // Cari nama sinyal untuk concat
    let sig_names: Vec<String> = extract_signal_names(source);
    if sig_names.len() < 2 {
        return source.to_string();
    }
    let selected: Vec<&str> = sig_names.iter().take(3.min(sig_names.len())).map(|s| s.as_str()).collect();
    let var_name = format!("_fuzz_concat_{}", rng.gen_range(0..99999));
    let width: u64 = rng.gen_range(1..=32);
    let hw = width - 1;
    let snippet = format!(
        "  // fuzz concat stress (Paper #7)\n  wire [{hw}:0] {vn};\n  assign {vn} = {parts};\n",
        vn = var_name,
        parts = selected.iter().map(|s| format!("{}[7:0]:", s)).collect::<Vec<_>>().join(", ")
    );
    let mut s = source.to_string();
    if let Some(end) = s.rfind("endmodule") {
        s.insert_str(end, &snippet);
    } else {
        s.push_str(&snippet);
    }
    s
}

/// NEW: stress test replication (Paper #7 — dataflow).
pub fn stress_replicate(rng: &mut StdRng, source: &str) -> String {
    let sig_names: Vec<String> = extract_signal_names(source);
    let Some(sig) = sig_names.choose(rng) else {
        return source.to_string();
    };
    let rep: u64 = rng.gen_range(1..=16);
    let width: u64 = rng.gen_range(1..=8);
    let var_name = format!("_fuzz_rep_{}", rng.gen_range(0..99999));
    let total = width * rep - 1;
    let hw = width - 1;
    let snippet = format!(
        "  // fuzz replicate stress (Paper #7)\n  wire [{total}:0] {vn};\n  assign {vn} = {rep}x\'{sig}[{hw}:0];\n",
        total = total,
        vn = var_name,
        rep = rep,
        sig = sig
    );
    let mut s = source.to_string();
    if let Some(end) = s.rfind("endmodule") {
        s.insert_str(end, &snippet);
    } else {
        s.push_str(&snippet);
    }
    s
}

/// Ekstrak nama sinyal dari deklarasi.
fn extract_signal_names(source: &str) -> Vec<String> {
    source
        .lines()
        .filter(|l| {
            l.contains("logic")
                || l.contains("wire")
                || l.contains("reg")
                || l.contains("input")
                || l.contains("output")
        })
        .flat_map(|l| {
            let parts: Vec<&str> = l.split_whitespace().collect();
            parts.iter().filter(|p| {
                **p != ""
                    && !p.starts_with('[')
                    && !p.contains(':')
                    && !p.contains(';')
                    && !p.contains(',') // koma pemisah port-list → nama rusak
                    && **p != "logic"
                    && **p != "wire"
                    && **p != "reg"
                    && **p != "input"
                    && **p != "output"
            }).map(|s| s.trim_end_matches(',').to_string()).collect::<Vec<_>>()
        })
        .collect()
}

/// NEW: manipulasi initial/reset — ubah timing atau nilai di initial block
/// (Paper #18 — race condition stressor / FSM-aware).
pub fn tweak_initial(rng: &mut StdRng, source: &str) -> String {
    let positions: Vec<(usize, usize, u64)> = int_literals(source)
        .into_iter()
        .filter(|(start, _, _)| {
            // Byte-safe: start adalah batas char, tapi start-20 bisa memotong
            // char multibyte → from_utf8 fallback agar tidak panic.
            let ctx_start = start.saturating_sub(20);
            let ctx = std::str::from_utf8(&source.as_bytes()[ctx_start..*start]).unwrap_or_default();
            ctx.contains("initial") || ctx.contains("#")
        })
        .collect();
    if positions.is_empty() {
        // Fallback: sisipkan initial block baru.
        let val = rng.gen_range(1..=255);
        let mut s = source.to_string();
        if let Some(end) = s.rfind("endmodule") {
            s.insert_str(end, &format!(
                "  // fuzz initial tweak (Paper #18)\n  initial begin\n    #{} $display(\"fuzz\");\n  end\n",
                val
            ));
        } else {
            s.push_str(&format!(
                "  initial begin\n    #{} $display(\"fuzz\");\n  end\n",
                val
            ));
        }
        return s;
    }
    let (start, end, num) = *positions.choose(rng).unwrap();
    let delta = rng.gen_range(1..=num.max(10)) as u64;
    let new_val = if rng.gen_bool(0.5) {
        (num + delta).min(9999)
    } else {
        // saturating: num==0 aman, hasil minimal 1 (hindari underflow).
        num.saturating_sub(delta).max(1)
    };
    let mut s = source.to_string();
    s.replace_range(start..end, &new_val.to_string());
    s
}

/// Lekser mini: lebar sinyal dari baris deklarasi `[msb:0]` — mendukung
/// `[7:0]` dan `[16-1:0]` (ekspresi `n-1`). None bila tak bisa dipastikan
/// (konservatif — mirror di-skip daripada salah width → false positive).
fn declared_width(source: &str, name: &str) -> Option<usize> {
    const DECL: &[&str] = &["logic ", "wire ", "reg ", "bit ", "output ", "input ", "inout "];
    for line in source.lines() {
        let t = line.trim_start();
        if !DECL.iter().any(|d| t.starts_with(d)) {
            continue;
        }
        // Token `name` (utuh, bukan substring) di baris ini?
        let tokens: Vec<&str> = line
            .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
            .filter(|s| !s.is_empty())
            .collect();
        if !tokens.contains(&name) {
            continue;
        }
        let Some(open) = line.find('[') else { continue };
        let Some(rel) = line[open + 1..].find(']') else { continue };
        let range = &line[open + 1..open + 1 + rel];
        let Some((msb_s, lsb_s)) = range.split_once(':') else { continue };
        let msb = parse_msb(&msb_s.replace(' ', ""))?;
        let lsb: usize = lsb_s.trim().parse().ok()?;
        if lsb != 0 {
            continue; // hanya [msb:0] — seleksi lain width ambigu
        }
        return Some(msb.saturating_sub(lsb) + 1);
    }
    None
}

fn parse_msb(s: &str) -> Option<usize> {
    if let Some((a, b)) = s.split_once('-') {
        let av: i64 = a.parse().ok()?;
        let bv: i64 = b.parse().ok()?;
        return Some((av - bv).max(0) as usize);
    }
    s.parse::<i64>().ok().map(|v| v.max(0) as usize)
}

/// Property-oracle seed (Paper #2/#3/#14, oracle #5): deteksi inkonsistensi
/// evaluasi ekspresi. Ambil satu baris `assign y = <rhs>;` (y declared, lebar
/// W) lalu tambah blok SELF-CONTAINED — dua temp mengevaluasi rhs yang SAMA
/// di lebar W (konteks assign sama dgn assign asli):
///
/// ```text
/// wire [W-1:0] _fz_rtA_N;  assign _fz_rtA_N = (<rhs>);
/// wire [W-1:0] _fz_rtB_N;  assign _fz_rtB_N = (<rhs>);
/// wire _fz_viol_N;         assign _fz_viol_N = (_fz_rtA_N !== _fz_rtB_N);
/// ```
///
/// Engine konsisten → kedua temp identik → `_fz_viol` selalu 0. 1 = evaluasi
/// ekspresi yang sama memberi hasil beda (bug eval/lebar/order/stale).
/// Self-contained (tidak mereferensikan lhs): minimizer baris tidak bisa
/// menciptakan violasi palsu via penghapusan deklarasi/driver.
pub fn insert_assert_mirror(rng: &mut StdRng, source: &str) -> String {
    let assign_lines: Vec<&str> = source
        .lines()
        .filter(|l| l.contains("assign") && l.contains('=') && l.trim_end().ends_with(';'))
        .collect();
    let Some(line) = assign_lines.choose(rng).copied() else {
        return source.to_string();
    };
    let Some((lhs_raw, rhs_raw)) = line.split_once('=') else {
        return source.to_string();
    };
    let lhs = lhs_raw.replace("assign", "").trim().to_string();
    // LHS: token polos (tanpa selektor/spasi) dengan lebar declared.
    if lhs.is_empty()
        || lhs.contains(' ')
        || lhs.contains('[')
        || lhs.contains('`')
        || lhs.contains('.')
        || lhs.contains('{')
    {
        return source.to_string();
    }
    let Some(width) = declared_width(source, &lhs) else {
        return source.to_string();
    };
    let rhs = rhs_raw.trim().trim_end_matches(';').trim().to_string();
    if rhs.is_empty() || rhs.contains(';') || rhs.contains('$') || rhs.contains('"') {
        return source.to_string();
    }
    let run_id = rng.gen_range(0..99999u32);
    let snippet = format!(
        "  // fuzz property mirror (Paper #2/#3/#14)\n  wire [{w}-1:0] _fz_rtA_{vi};\n  assign _fz_rtA_{vi} = ({rh});\n  wire [{w}-1:0] _fz_rtB_{vi};\n  assign _fz_rtB_{vi} = ({rh});\n  wire _fz_viol_{vi};\n  assign _fz_viol_{vi} = (_fz_rtA_{vi} !== _fz_rtB_{vi});\n",
        w = width,
        vi = run_id,
        rh = rhs
    );
    let mut s = source.to_string();
    if let Some(end) = s.rfind("endmodule") {
        s.insert_str(end, &snippet);
    } else {
        s.push_str(&snippet);
    }
    s
}

/// Property-oracle berbasis `assert` (Paper #14/#2/#3, oracle #5): tanam
/// assertion yang WAJIB benar — dua temp `_fz_atA`/`_fz_atB` mengevaluasi
/// ekspresi SAMA, lalu `assert (_fz_atA === _fz_atB) else $fatal`. Engine
/// konsisten → assertion selalu pass; fail = bug evaluasi (eval/lebar/order).
pub fn insert_assert_oracle(rng: &mut StdRng, source: &str) -> String {
    let assign_lines: Vec<&str> = source
        .lines()
        .filter(|l| l.contains("assign") && l.contains('=') && l.trim_end().ends_with(';'))
        .collect();
    let Some(line) = assign_lines.choose(rng).copied() else {
        return source.to_string();
    };
    let Some((lhs_raw, rhs_raw)) = line.split_once('=') else {
        return source.to_string();
    };
    let lhs = lhs_raw.replace("assign", "").trim().to_string();
    if lhs.is_empty()
        || lhs.contains(' ')
        || lhs.contains('[')
        || lhs.contains('`')
        || lhs.contains('.')
        || lhs.contains('{')
    {
        return source.to_string();
    }
    let Some(width) = declared_width(source, &lhs) else {
        return source.to_string();
    };
    let rhs = rhs_raw.trim().trim_end_matches(';').trim().to_string();
    if rhs.is_empty() || rhs.contains(';') || rhs.contains('$') || rhs.contains('"') {
        return source.to_string();
    }
    let run_id = rng.gen_range(0..99999u32);
    let snippet = format!(
        "  // fuzz assert oracle (Paper #14, oracle #5)\n  wire [{w}-1:0] _fz_atA_{vi};\n  assign _fz_atA_{vi} = ({rh});\n  wire [{w}-1:0] _fz_atB_{vi};\n  assign _fz_atB_{vi} = ({rh});\n  initial begin\n    #1 assert (_fz_atA_{vi} === _fz_atB_{vi}) else $fatal(0, \"fuzzer assert oracle violated\");\n  end\n",
        w = width,
        vi = run_id,
        rh = rhs
    );
    let mut s = source.to_string();
    if let Some(end) = s.rfind("endmodule") {
        s.insert_str(end, &snippet);
    } else {
        s.push_str(&snippet);
    }
    s
}

/// Deteksi sinyal assert-oracle yang sudah tertanam (Paper #18 re-seed).
pub fn has_assert_oracle(source: &str) -> bool {
    source.contains("_fz_atA_") && source.contains("_fz_atB_") && source.contains("assert (")
}

/// Validasi blok assert-oracle masih UTUH: deklarasi `wire [W-1:0] _fz_atA_N`
/// dan `_fz_atB_N` (lebar sama) + `assert (...)` masih ada. Minimizer baris
/// bisa menghapus deklarasi wire → temp jadi implicit net (lebar default) →
/// assert `===` antara lebar beda = 0 selalu, ATAU sinyal tak ada → RT0001
/// (bukan RT7001). Hanya blok utuh yang membuktikan engine mengevaluasi ekspresi
/// identik di lebar sama (fail = bug eval, bukan artefak minimizer).
pub fn has_assert_oracle_temps(source: &str) -> bool {
    if !has_assert_oracle(source) {
        return false;
    }
    let lines: Vec<&str> = source.lines().map(|l| l.trim()).collect();
    // Cari satu `_fz_atA_<digits>` dan pasangannya `_fz_atB_<digits>` yang sama,
    // masing2 punya deklarasi `wire [..-1:0]` tepat satu.
    let decl_of = |name: &str| -> Option<String> {
        let hits: Vec<&str> = lines
            .iter()
            .filter(|l| l.starts_with("wire [") && l.contains(name) && l.ends_with(';') && l.contains("-1:0]"))
            .copied()
            .collect();
        if hits.len() != 1 {
            return None;
        }
        let l = hits[0];
        let s = l.find('[')?;
        let e = l[s..].find(']')? + s;
        Some(l[s..=e].to_string())
    };
    for l in &lines {
        if let Some(rest) = l.strip_prefix("assign _fz_atA_") {
            let id: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if id.is_empty() {
                continue;
            }
            let a = format!("_fz_atA_{}", id);
            let b = format!("_fz_atB_{}", id);
            match (decl_of(&a), decl_of(&b)) {
                (Some(da), Some(db)) if da == db => return true,
                _ => {}
            }
        }
    }
    false
}

/// Tanam interface + instansiasi + koneksi (Paper #10/#12, fitur mahal):
/// airadakan elaborator interface/modport/hierarki 2-level. Sisip di awal file
/// (sebelum module pertama) definisi interface & child yang memakai modport,
/// dan di body module top sediakan koneksi. Self-contained (nama unik) —
/// tidak bergantung pada seed sehingga minimizer tidak bisa merusak sintaks.
/// Men-differential stress: instansiasi interface + hier signal map + modport.
pub fn insert_interface(rng: &mut StdRng, source: &str) -> String {
    let w: usize = [4usize, 8, 16].choose(rng).copied().unwrap_or(8);
    let id = rng.gen_range(0..99_999u32);
    let ifname = format!("fz_bus_{}", id);
    let child = format!("fz_child_{}", id);
    // Interface utuh + child yang memakai modport + top yang instansiasi.
    let block = format!(
        "\ninterface {ifname}; \n  logic [{w}-1:0] data;\n  logic valid;\n  modport m (input data, output valid);\nendinterface : {ifname}\n\n\
         module {child} (\\\n  {ifname}.m ifc\n);\n  assign ifc.valid = ifc.data[0];\nendmodule\n\n"
    );
    let inst = format!(
        "{ifname} fz_bif_{id} ();\n  {child} fz_u_{id} (.ifc(fz_bif_{id}));\n  wire [{w}-1:0] fz_d_{id};\n  assign fz_d_{id} = fz_bif_{id}.data;\n"
    );
    let mut s = String::new();
    s.push_str(&block);
    // Sisipkan instansiasi tepat sebelum `endmodule` pertama, definisi
    // interface di depan.
    let body = if let Some(pos) = source.find("endmodule") {
        let mut t = String::new();
        t.push_str(&source[..pos]);
        t.push_str(&inst);
        t.push_str(&source[pos..]);
        t
    } else {
        source.to_string()
    };
    s.push_str(&body);
    s
}

/// Deteksi interface-oracle (Paper #18 re-seed): ada `interface fz_bus_`.
pub fn has_interface_oracle(source: &str) -> bool {
    source.contains("interface fz_bus_") && source.contains(".ifc(")
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

    #[test]
    fn insert_assert_mirror_adds_violation_signal() {
        let mut rng = StdRng::seed_from_u64(5);
        let out = insert_assert_mirror(&mut rng, SEED);
        assert!(out.contains("_fz_viol_"), "harus ada sinyal violasi: {}", out);
        assert!(out.contains("!== _fz_rtB_"), "harus memuat mirror ekspresi: {}", out);
        assert!(out.contains("endmodule"));
    }

    #[test]
    fn insert_assert_oracle_adds_assert_and_temps() {
        let mut rng = StdRng::seed_from_u64(8);
        let out = insert_assert_oracle(&mut rng, SEED);
        assert!(out.contains("_fz_atA_"), "harus ada temp A: {}", out);
        assert!(out.contains("_fz_atB_"), "harus ada temp B: {}", out);
        assert!(out.contains("assert ("), "harus menanam assertion: {}", out);
        assert!(out.contains("$fatal"), "assert harus punya else $fatal: {}", out);
        assert!(out.contains("endmodule"));
    }

    #[test]
    fn has_assert_oracle_detects_planted() {
        let mut rng = StdRng::seed_from_u64(9);
        let out = insert_assert_oracle(&mut rng, SEED);
        assert!(has_assert_oracle(&out));
        assert!(!has_assert_oracle(SEED));
    }

    #[test]
    fn assert_oracle_temps_requires_complete_decls() {
        let mut rng = StdRng::seed_from_u64(12);
        // Blok utuh (temp2 ter-deklarasi) → true.
        let full = insert_assert_oracle(&mut rng, SEED);
        assert!(has_assert_oracle_temps(&full), "blok utuh harus valid");
        // Minimizer artefak: hilangkan deklarasi `wire [W-1:0] _fz_atB_` →
        // temp jadi implicit net → harus ditolak (bukan proof bug eval).
        let mut dropped = full.clone();
        if let Some(pos) = dropped.find("wire [") {
            if let Some(line_end) = dropped[pos..].find('\n') {
                let real = pos + line_end;
                let _ = real;
            }
        }
        // Hapus baris deklarasi wire pertama (yang berisi "_fz_atA_" wire decl).
        let lines: Vec<String> = dropped.lines().map(|s| s.to_string()).collect();
        let mut kept = Vec::new();
        let mut removed = false;
        for l in &lines {
            if !removed && l.trim_start().starts_with("wire [") && l.contains("_fz_atA_") {
                removed = true;
                continue;
            }
            kept.push(l.clone());
        }
        dropped = kept.join("\n");
        assert!(
            !has_assert_oracle_temps(&dropped),
            "deklarasi temp hilang = artefak minimizer, bukan proof bug"
        );
    }

    #[test]
    fn insert_interface_adds_interface_and_inst() {
        let mut rng = StdRng::seed_from_u64(10);
        let out = insert_interface(&mut rng, SEED);
        assert!(out.contains("interface fz_bus_"), "harus ada interface: {}", out);
        assert!(out.contains("modport"), "harus ada modport: {}", out);
        assert!(out.contains(".ifc("), "harus ada koneksi ifc: {}", out);
        assert!(out.contains("endmodule"));
    }

    #[test]
    fn has_interface_oracle_detects_planted() {
        let mut rng = StdRng::seed_from_u64(11);
        let out = insert_interface(&mut rng, SEED);
        assert!(has_interface_oracle(&out));
        assert!(!has_interface_oracle(SEED));
    }

    #[test]
    fn tweak_initial_no_underflow_on_zero_literal() {
        // num=0 dulu memicu underflow di jalur pengurangan.
        let mut rng = StdRng::seed_from_u64(6);
        let src = "module top;\n  initial begin #0 $display(\"x\"); end\nendmodule\n";
        let out = tweak_initial(&mut rng, src);
        assert!(out.contains("initial"));
    }
}