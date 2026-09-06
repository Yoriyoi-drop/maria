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

/// EMI check: original vs variant dead-code — sinyal *common* harus sama.
/// Sinyal internal tambahan (`_fuzz_dn`) muncul hanya di variant → dibandingkan
/// hanya sinyal yang ada di kedua sisi (semantik EMI: output lestari).
pub fn emi_check(source: &str, cfg: &FuzzConfig) -> DiffVerdict {
    if oracle::has_nondeterministic_src(source) {
        return DiffVerdict::Skip;
    }
    let variant = emi_variant(source);
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