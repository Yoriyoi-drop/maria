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
pub fn mutate(rng: &mut StdRng, source: &str, corpus: &Corpus) -> String {
    // Chain length: semakin panjang semakin banyak variasi per iterasi
    // (Paper #2/#3 eksplorasi luas, Paper #12 fragment splice).
    // Kadang 1, kadang 2-4 untuk eksplorasi lebih dalam.
    let chain = match rng.gen_range(0..100u32) {
        n if n < 25 => 1,   // 25%: chain 1
        n if n < 50 => 2,   // 25%: chain 2
        n if n < 75 => 3,   // 25%: chain 3
        _ => 4,             // 25%: chain 4
    };
    let mut out = source.to_string();
    for _ in 0..chain {
        let op = rng.gen_range(0..11u32);
        out = match op {
            0 => replace_operator(rng, &out),
            1 => flip_literal(rng, &out),
            2 => splice_corpus_fragment(rng, &out, corpus),
            3 => insert_grammar_decl(rng, &out),
            4 => insert_grammar_body(rng, &out),
            5 => duplicate_line(rng, &out),
            // NEW: mutasi lebar bit (Paper #7 VUzzer — eksplorasi lebar).
            6 => change_width(rng, &out),
            // NEW: sisip part-select out-of-range (stress test per-bit §11.5.1).
            7 => insert_partselect_oob(rng, &out),
            // NEW: ganti parameter/const value (Paper #15 YARPGen — type-aware).
            8 => tweak_params(rng, &out),
            // NEW: manipulasi initial/reset (race condition stressor).
            9 => tweak_initial(rng, &out),
            // NEW: property-oracle — mirror ekspresi assign ke sinyal violasi
            // (Paper #2/#3/#14; oracle #5 property): `_fz_viol = (lhs !== rhs)`.
            // Engine konsisten → selalu 0; 1 = bug semantik eval.
            10 => insert_assert_mirror(rng, &out),
            _ => out.to_string(),
        };
    }
    out
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
            // Ambil nama diakhir baris (setelah tipe & [dim])
            let mut parts: Vec<&str> = l.split_whitespace().collect();
            parts.retain(|p| !p.is_empty() && !p.starts_with('[') && !p.contains(':') && !p.contains(';'));
            let tail = parts.last()?.to_string();
            if tail == "logic" || tail == "wire" || tail == "reg" || tail == "input" || tail == "output" {
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
                    && **p != "logic"
                    && **p != "wire"
                    && **p != "reg"
                    && **p != "input"
                    && **p != "output"
            }).map(|s| s.to_string()).collect::<Vec<_>>()
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
    fn tweak_initial_no_underflow_on_zero_literal() {
        // num=0 dulu memicu underflow di jalur pengurangan.
        let mut rng = StdRng::seed_from_u64(6);
        let src = "module top;\n  initial begin #0 $display(\"x\"); end\nendmodule\n";
        let out = tweak_initial(&mut rng, src);
        assert!(out.contains("initial"));
    }
}