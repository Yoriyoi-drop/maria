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

// ──────────────────────────────────────────────────────────────────────
// Oracle metamorfik identitas (GAP-5) — perluasan EMI dari "kode mati tidak
// boleh mengubah output" ke "relasi identitas semantik harus dipertahankan":
//
//   assign y = <rhs>;      ≡      assign y = (<rhs> op 0);
//
// untuk op ∈ {+, -, |, ^}. Identitas eksplisit di SV 4-state (*semua* nilai
// termasuk X/Z): x+0=x, x-0=x, x|0=x, x^0=x. Jika maria mengevaluasi `op 0`
// secara salah (lebar/signedness/order), fingerprint berubah → bug, walaupun
// evaluasi konsisten diri (menutup sebagian ORACLE GAP: oracle ini menangkap
// KESALAHAN SEMANTIK, bukan hanya inkonsistensi internal).
// ──────────────────────────────────────────────────────────────────────

/// Operator identitas — deterministik dari hash source (minimizer memanggil
/// berkali-kali; varian harus pure function dari source, sama dgn EMI).
pub fn meta_style_for(source: &str) -> &'static str {
    let h = source.bytes().fold(0x9e37_79b9u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x1000_0000_01b3)
    });
    match h % 4 {
        0 => "+ 0",
        1 => "- 0",
        2 => "| 0",
        _ => "^ 0",
    }
}

/// Bangun varian metamorfik-identitas: ganti `assign lhs = rhs;` pertama
/// yang AMAN (lhs sederhana tanpa selektor, rhs satu-baris tanpa `;$"/`)
/// menjadi `assign lhs = (rhs op 0);`. Deterministik (baris aman pertama).
/// None bila tidak ada assign yang memenuhi syarat.
pub fn meta_identity_variant(source: &str) -> Option<String> {
    let op = meta_style_for(source);
    let mut out = String::with_capacity(source.len() + 8);
    let mut replaced = false;
    for line in source.lines() {
        if !replaced {
            let t = line.trim();
            if t.starts_with("assign") && t.contains('=') && t.trim_end().ends_with(';') {
                let (lhs_raw, rhs_raw) = t.split_once('=').unwrap_or(("", ""));
                let lhs = lhs_raw.replace("assign", "").trim().to_string();
                let rhs = rhs_raw.trim().trim_end_matches(';').trim().to_string();
                let safe = !lhs.is_empty()
                    && !lhs.contains(' ')
                    && !lhs.contains('[')
                    && !lhs.contains('`')
                    && !lhs.contains('.')
                    && !lhs.contains('{')
                    && !rhs.is_empty()
                    && !rhs.contains(';')
                    && !rhs.contains('$')
                    && !rhs.contains('"');
                if safe {
                    out.push_str(&format!("assign {} = ({} {});", lhs, rhs, op));
                    out.push('\n');
                    replaced = true;
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if replaced {
        Some(out)
    } else {
        None
    }
}

/// Oracle metamorfik-identitas: original vs varian `(rhs op 0)` — sinyal
/// common harus identik. Penyimpangan = bug evaluasi (identity dilanggar).
pub fn meta_identity_check(source: &str, cfg: &FuzzConfig) -> DiffVerdict {
    if oracle::has_nondeterministic_src(source) {
        return DiffVerdict::Skip;
    }
    let Some(variant) = meta_identity_variant(source) else {
        return DiffVerdict::Skip;
    };
    let f_orig = harness::fingerprint_isolated(source, cfg.max_time, cfg.hang_ms);
    let f_var = harness::fingerprint_isolated(&variant, cfg.max_time, cfg.hang_ms);
    match (f_orig, f_var) {
        (Some(a), Some(b)) => match compare_common(&a, &b) {
            None => DiffVerdict::Same,
            Some(detail) => DiffVerdict::Mismatch(format!("orig vs meta-identity: {}", detail)),
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
///
/// Sinyal INTERNAL artefak fuzzer diabaikan: prefix `fz_`, `_fz_`, `_fuzz_`
/// (semuanya ditanam oracle generator/grammar/assert-mirror/interface; `fz_q1`,
/// `fz_y`, `_fz_atA/B`, `_fz_viol`, `_fuzz_pa`, `fz_bif`, `fz_d`, dsb).
/// Dead-code menambah net yang utk source malformed (multi-driver / implicit
/// net) mengubah net-topology → `z` vs `x` beda, PADAHAL bukan bug engine.
/// EMI bug sejati = dead-code mengubah sinyal TOP nyata (`y`, `r`, `flag`,
/// sinyal user non-`fz_`). Nota: kalau user menulis sinyal sendiri berawalan
/// `fz_`, EMI tak bisa membedakannya — konvensi internal fuzzer.
fn is_internal_artifact(name: &str) -> bool {
    let base = name.rsplit('.').next().unwrap_or(name);
    base.starts_with("fz_") || base.starts_with("_fz_") || base.starts_with("_fuzz")
}

/// Bandingkan hanya sinyal yang ada di kedua sisi, mengabaikan artefak internal.
fn compare_common(a: &str, b: &str) -> Option<String> {
    let ma = fingerprint_map(a);
    let mb = fingerprint_map(b);
    let mut diffs: Vec<String> = Vec::new();
    for (name, va) in &ma {
        if is_internal_artifact(name) {
            continue;
        }
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

    #[test]
    fn internal_artifact_filter() {
        // Sinyal top nyata (user) → tidak di-filter.
        assert!(!is_internal_artifact("y"));
        assert!(!is_internal_artifact("r"));
        assert!(!is_internal_artifact("tl_h_i"));
        // Artefak internal fuzzer (oracle/gen/grammar) → di-filter.
        assert!(is_internal_artifact("fz_q1"));
        assert!(is_internal_artifact("_fz_atA_7"));
        assert!(is_internal_artifact("_fz_viol_7"));
        assert!(is_internal_artifact("_fuzz_pa_123"));
        assert!(is_internal_artifact("fz_u_5.fz_d_14800"));
        assert!(is_internal_artifact("fz_bif_3"));
        // Top-level output bersih terhadap internal.
        let ma = "y=00@2|r=00@2|fz_q1=1@1|_fuzz_pa_5=zzz@3";
        let mb = "y=00@2|r=00@2|fz_q1=x@1|_fuzz_pa_5=001@3";
        assert_eq!(compare_common(ma, mb), None, "hanya artefak beda → bukan bug EMI");
    }

    #[test]
    fn meta_identity_variant_changes_source() {
        let v = meta_identity_variant(COUNTER).expect("counter punya assign aman");
        assert_ne!(v, COUNTER);
        assert!(v.contains("0);"), "variant harus memuat `op 0`: {}", v);
    }

    #[test]
    fn meta_identity_same_for_counter() {
        // Soundness: `(r op 0) ≡ r` utk semua nilai — validasi relasi identitas
        // pada design nyata (harus Same, bukan false positive).
        let cfg = cfg();
        assert_eq!(meta_identity_check(COUNTER, &cfg), DiffVerdict::Same);
    }

    #[test]
    fn meta_identity_sensitive_to_semantic_change() {
        // Sensitivitas: `(a + 1)` BUKAN identitas → fingerprint harus beda
        // (bukti oracle benar-benar mendeteksi perubahan semantik).
        let cfg = cfg();
        let src = "module top(input logic [3:0] a, output logic [3:0] y);\n  assign y = a;\n  initial begin a = 4'd5; end\nendmodule\n";
        let bad = src.replace("assign y = a;", "assign y = (a + 1'b1);");
        let f1 = harness::fingerprint_isolated(src, cfg.max_time, cfg.hang_ms);
        let f2 = harness::fingerprint_isolated(&bad, cfg.max_time, cfg.hang_ms);
        match (f1, f2) {
            (Some(a), Some(b)) => assert_ne!(a, b, "a vs a+1 harus beda"),
            _ => panic!("fingerprint gagal utk program sederhana"),
        }
    }
}