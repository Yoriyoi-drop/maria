//! Oracle differential — deteksi penyimpangan semantik (bukan sekadar crash).
//!
//! Paper #13 (EMI, Le/Afshari/Su): equivalence modulo inputs — sisipan kode
//! mati (dead code) TIDAK boleh mengubah output. Input sama → output sama.
//! Paper #19 (DifuzzRTL): oracle nilai sinyal — fingerprint sinyal top
//! dibandingkan antar eksekusi.

use crate::harness;
use crate::oracle;
use crate::FuzzConfig;

#[derive(Debug, Clone, PartialEq)]
pub enum DiffVerdict {
    /// Kedua eksekusi identik (baik).
    Same,
    /// Penyimpangan terdeteksi — detail untuk bug report.
    Mismatch(String),
    /// Tidak bisa dibandingkan (compile gagal / nondeterministik sah).
    Skip,
}

/// Determinism check (Paper #13/#19): sumber sama dieksekusi 2x →
/// fingerprint sinyal harus identik. Menangkap state global bocor antar run.
pub fn determinism_check(source: &str, cfg: &FuzzConfig) -> DiffVerdict {
    if oracle::has_nondeterministic_src(source) {
        return DiffVerdict::Skip;
    }
    let f1 = harness::fingerprint_isolated(source, cfg.max_time, cfg.hang_ms);
    let f2 = harness::fingerprint_isolated(source, cfg.max_time, cfg.hang_ms);
    match (f1, f2) {
        (Some(a), Some(b)) if a == b => DiffVerdict::Same,
        (Some(a), Some(b)) => DiffVerdict::Mismatch(format!("run1 vs run2:\n  {}\n  {}", a, b)),
        _ => DiffVerdict::Skip,
    }
}

/// EMI variant: sisipkan kode mati (unused wire + assign) tepat sebelum
/// `endmodule`. Aman utk modul apa pun — tidak menyentuh sinyal/modul lain.
/// Output tidak boleh berubah (Paper #13 dead-code equivalence).
pub fn emi_variant(source: &str) -> String {
    const DEAD: &str = "  // EMI dead-code (Paper #13)\n  wire [7:0] _fuzz_dn;\n  assign _fuzz_dn = 8'h00;\n";
    let mut s = source.to_string();
    if let Some(end) = s.rfind("endmodule") {
        s.insert_str(end, DEAD);
    } else {
        s.push_str(DEAD);
    }
    s
}

/// Gaya variant EMI (Paper #13: equivalent mutants — kode mati TIDAK boleh
/// mengubah output; ekspresi ekivalen juga tidak).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EmiStyle {
    /// unused wire + assign konstanta (variant asli).
    DeadWire,
    /// unused always_comb yang membaca/menulis sinyal fuzz internal.
    DeadAlways,
    /// equivalent expression: tukar operan operator komutatif
    /// (`a + b` → `b + a`) — simetris utk seluruh nilai 4-state.
    ExprSwap,
}

/// Pilih style EMI deterministik dari konten source (hash) — minimizer butuh
/// variant yang PURE function dari source (kandidat di-minimize memanggil
/// emi_check berkali-kali; variant harus sama utk source sama).
pub fn emi_style_for(source: &str) -> EmiStyle {
    let h = source.bytes().fold(0x9e37_79b9u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x1000_0000_01b3)
    });
    match h % 3 {
        0 => EmiStyle::DeadWire,
        1 => EmiStyle::DeadAlways,
        _ => EmiStyle::ExprSwap,
    }
}

/// Bangun variant sesuai style.
pub fn emi_variant_ex(source: &str, style: EmiStyle) -> String {
    match style {
        EmiStyle::DeadWire => emi_variant(source),
        EmiStyle::DeadAlways => {
            const DEAD: &str = "  // EMI dead-code (Paper #13)\n  wire [7:0] _fuzz_dn;\n  logic _fuzz_dq;\n  always_comb begin\n    _fuzz_dq = 1'b0;\n    if (_fuzz_dn[0] && _fuzz_dn[7]) _fuzz_dq = 1'b1;\n  end\n";
            let mut s = source.to_string();
            if let Some(end) = s.rfind("endmodule") {
                s.insert_str(end, DEAD);
            } else {
                s.push_str(DEAD);
            }
            s
        }
        EmiStyle::ExprSwap => emi_expr_swap(source),
    }
}

/// Operator biner komutatif aman dtukar operannya (a op b == b op a utk
/// semua nilai 4-state, incl. X/Z) — Paper #13 equivalent mutant.
const COMMUTATIVE_OPS: &[u8] = b"+&|^*";
const MULTI_OPS: &[&str] = &["&&", "||"];

/// EMI equivalent-expression: tukar operan operator komutatif PERTAMA yang
/// ditemukan (`2 * 3 + a` → `2 * 3 + a` tidak berubah; `a + b` → `b + a`).
/// Deterministik (match pertama). Operand harus token identifier murni
/// (`[A-Za-z0-9_]`) dan tempat mu bukan di dalam komentar `//`/`/* */`.
pub fn emi_expr_swap(source: &str) -> String {
    let bytes = source.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let is_space = |c: u8| c == b' ' || c == b'\t';
    let mut i = 0usize;
    while i < bytes.len() {
        // Multi-char operator dua karakter (&&/||) — cek dulu sebelum char.
        let op_len = if bytes.get(i..i + 2).map(|w| w == b"&&" || w == b"||").unwrap_or(false) {
            2
        } else if COMMUTATIVE_OPS.contains(&bytes[i]) {
            1
        } else {
            i += 1;
            continue;
        };
        let op_at = i;
        let op_end = i + op_len;
        // Operan kiri: mundur dari op, lewati spasi/tab, harus ident murni.
        let mut li = i.saturating_sub(1);
        while li > 0 && is_space(bytes[li]) {
            li -= 1;
        }
        if !is_ident(bytes[li]) {
            i = op_end;
            continue;
        }
        let mut lstart = li;
        while lstart > 0 && is_ident(bytes[lstart - 1]) {
            lstart -= 1;
        }
        // Operan kanan: lewati spasi/tab, harus ident murni.
        let mut ri = op_end;
        while ri < bytes.len() && is_space(bytes[ri]) {
            ri += 1;
        }
        if ri >= bytes.len() || !is_ident(bytes[ri]) {
            i = op_end;
            continue;
        }
        let mut rend = ri + 1;
        while rend < bytes.len() && is_ident(bytes[rend]) {
            rend += 1;
        }
        // Guard token utuh: kedua operan harus identifier/number MANDIRI
        // (bukan member `obj.a`, bukan literal `2'b10`, bukan seleksi
        // `b[3:0]`, bukan argumen fungsi `f(x)`) — kalau tidak, swap bisa
        // merusak sintaks atau mengubah semantik.
        if lstart > 0 {
            let lb = bytes[lstart - 1];
            if matches!(lb, b'.' | b':' | b'\'' | b'[' | b'(' | b'#' | b'`') {
                i = op_end;
                continue;
            }
        }
        if rend < bytes.len() {
            let rb = bytes[rend];
            if is_ident(rb) || matches!(rb, b'.' | b':' | b'\'' | b'[' | b'(' | b'#' | b'`') {
                i = op_end;
                continue;
            }
        }
        // Lewati komentar `//` di baris yang sama (sebelum op) dan `/* ... */`.
        let line_start = source[..lstart].rfind('\n').map(|p| p + 1).unwrap_or(0);
        if source[line_start..lstart].contains("//") {
            i = op_end;
            continue;
        }
        if let Some(ci) = source[..lstart].rfind("/*") {
            let close = source[..lstart].rfind("*/");
            if close.map_or(true, |c| c < ci) {
                i = op_end;
                continue;
            }
        }
        // SWAP: `L op R` → `R op L`.
        let left = &source[lstart..li + 1];
        let right = &source[ri..rend];
        let mut s = source.to_string();
        let op_text = &source[op_at..op_end];
        s.replace_range(lstart..rend, &format!("{} {} {}", right, op_text, left));
        return s;
    }
    // Tidak ada operator komutatif aman → fallback dead-code.
    emi_variant(source)
}

/// EMI check: original vs variant dead-code — sinyal *common* harus sama.
/// Sinyal internal tambahan (`_fuzz_dn`) muncul hanya di variant → dibandingkan
/// hanya sinyal yang ada di kedua sisi (semantik EMI: output lestari).
/// Variant dipilih deterministik dari konten source (DeadWire/DeadAlways/
/// ExprSwap) — lihat `emi_style_for`.
pub fn emi_check(source: &str, cfg: &FuzzConfig) -> DiffVerdict {
    if oracle::has_nondeterministic_src(source) {
        return DiffVerdict::Skip;
    }
    let variant = emi_variant_ex(source, emi_style_for(source));
    let f_orig = harness::fingerprint_isolated(source, cfg.max_time, cfg.hang_ms);
    let f_var = harness::fingerprint_isolated(&variant, cfg.max_time, cfg.hang_ms);
    match (f_orig, f_var) {
        (Some(a), Some(b)) => match compare_common(&a, &b) {
            None => DiffVerdict::Same,
            Some(detail) => DiffVerdict::Mismatch(format!("orig vs emi-variant: {}", detail)),
        },
        _ => DiffVerdict::Skip,
    }
}

/// Urai fingerprint `name=bits@width|name=...` → map nama → nilai.
fn fingerprint_map(fp: &str) -> std::collections::HashMap<String, String> {
    fp.split('|')
        .filter(|row| row.contains('='))
        .map(|row| {
            let (name, value) = row.split_once('=').unwrap_or((row, ""));
            (name.to_string(), value.to_string())
        })
        .collect()
}

/// Bandingkan hanya sinyal yang ada di kedua sisi.
/// None = identik; Some(detail) = daftar penyimpangan.
fn compare_common(a: &str, b: &str) -> Option<String> {
    let ma = fingerprint_map(a);
    let mb = fingerprint_map(b);
    let mut diffs: Vec<String> = Vec::new();
    for (name, va) in &ma {
        if let Some(vb) = mb.get(name) {
            if va != vb {
                diffs.push(format!("{}: {} vs {}", name, va, vb));
            }
        }
    }
    if diffs.is_empty() {
        None
    } else {
        Some(diffs.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FuzzConfig;

    const COUNTER: &str = r#"
module top(input logic clk, input logic rst_n,
           input logic [3:0] a, input logic [3:0] b,
           output logic [3:0] y);
  logic [3:0] r;
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) r <= '0;
    else r <= a + b;
  end
  assign y = r;
  initial begin clk = 0; forever #5 clk = ~clk; end
  initial begin rst_n = 0; a = 1; b = 2; #7 rst_n = 1; #3 a = 3; b = 4; end
endmodule
"#;

    fn cfg() -> FuzzConfig {
        FuzzConfig {
            max_time: 40,
            hang_ms: 2000,
            ..FuzzConfig::default()
        }
    }

    #[test]
    fn emi_variant_valid_and_larger() {
        let v = emi_variant(COUNTER);
        assert!(v.len() > COUNTER.len());
        assert!(v.contains("_fuzz_dn"));
        assert!(v.trim_end().ends_with("endmodule"));
    }

    #[test]
    fn emi_style_deterministic_per_source() {
        // Minimizer memanggil emi_check berulang — variant harus pure function.
        for _ in 0..10 {
            let a = emi_style_for(COUNTER);
            let b = emi_style_for(COUNTER);
            assert_eq!(a, b, "style harus deterministik utk source sama");
        }
    }

    #[test]
    fn emi_expr_swap_reorders_first_commutative_op() {
        // variant harus menukar `a + b` → `b + a` dan hasil sim tetap sama.
        let ab = "module top(input logic [3:0] a, input logic [3:0] b, output logic [3:0] y);\n  assign y = a + b;\nendmodule\n";
        let v = emi_expr_swap(ab);
        assert!(v.contains("b + a"), "operan harus tertukar: {}", v);
        let cfg = cfg();
        // fingerprints identik (commutativity & width sama) — DiffVerdict::Same.
        let f_orig = harness::fingerprint_isolated(ab, cfg.max_time, cfg.hang_ms);
        let f_var = harness::fingerprint_isolated(&v, cfg.max_time, cfg.hang_ms);
        match (f_orig, f_var) {
            (Some(a), Some(b)) => assert_eq!(a, b, "a+b vs b+a harus identik"),
            _ => panic!("fingerprint gagal utk ekspresi sederhana"),
        }
    }

    #[test]
    fn emi_all_styles_preserve_counter_semantics() {
        let cfg = cfg();
        for style in [
            EmiStyle::DeadWire,
            EmiStyle::DeadAlways,
            EmiStyle::ExprSwap,
        ] {
            let v = emi_variant_ex(COUNTER, style);
            assert!(v.len() > COUNTER.len() || v != COUNTER, "variant {:?} harus beda", style);
            assert_eq!(emi_check(COUNTER, &cfg), DiffVerdict::Same, "style {:?} merusak counter", style);
        }
    }

    #[test]
    fn emi_dead_always_variant_contains_consult() {
        let v = emi_variant_ex(COUNTER, EmiStyle::DeadAlways);
        assert!(v.contains("_fuzz_dq"));
        assert!(v.contains("always_comb"));
    }

    #[test]
    fn determinism_same_for_counter() {
        assert_eq!(determinism_check(COUNTER, &cfg()), DiffVerdict::Same);
    }

    #[test]
    fn emi_same_for_counter() {
        // Dead-code insertion wajib menghasilkan fingerprint identik.
        assert_eq!(emi_check(COUNTER, &cfg()), DiffVerdict::Same);
    }

    #[test]
    fn nondeterministic_skipped() {
        let src = format!("{}\ninitial $display($urandom());\n", COUNTER);
        assert_eq!(determinism_check(&src, &cfg()), DiffVerdict::Skip);
    }
}