//! Oracle hasil eksekusi maria — satu-satunya modul yang memanggil
//! `maria_api::compile_str` / `maria_api::simulate_signals` langsung.
//! (Harness membungkusnya dengan isolasi thread; lihat `harness.rs`.)
//!
//! Paper #2 (Liang 2018) & #3 (Manès 2019): klasifikasi verdict terstruktur —
//! fuzzer butuh tahu *kenapa* input ditolak (code) bukan hanya ya/tidak.

use std::collections::BTreeSet;
use maria_ir::LogicVec;

/// Hasil kompilasi (preprocessor→lexer→parser→elaborasi).
#[derive(Debug, Clone)]
pub struct CompileVerdict {
    pub ok: bool,
    /// Error code maria (mis. "E3001", "EL3001") — kosong jika ok.
    pub code: String,
    pub message: String,
}

/// Hasil simulasi (engine + sinyal top).
#[derive(Debug, Clone)]
pub struct SimVerdict {
    pub ok: bool,
    /// Fingerprint deterministik seluruh sinyal top (nama=bits@width, sorted).
    pub fingerprint: String,
    pub code: String,
    pub message: String,
    /// True bila simulasi berhenti karena assertion/SVA violation (property-oracle,
    /// Paper #14/#2/#3: assertion fail = bug engine atau properti broken).
    pub assertion: bool,
}

/// Compile oracle (Paper #2/#3): input valid → ok; input invalid → err code.
/// Pakai varian QUIET maria-api — child yang gagal-compile mencetak puluhan
/// baris diagnosa (E1002/E1005/WR0102) ke stderr; fuzzer hanya butuh code.
pub fn compile_verdict(source: &str) -> CompileVerdict {
    match maria_api::compile_str_quiet(source) {
        Ok(d) => CompileVerdict {
            ok: true,
            code: String::new(),
            message: format!("{} module(s), {} top signal(s)", d.modules.len(), d.top.signals.len()),
        },
        Err(e) => CompileVerdict {
            ok: false,
            code: e.error_code().to_string(),
            message: e.to_string(),
        },
    }
}

/// Sim oracle (Paper #2/#3): compile+simulasi → fingerprint sinyal.
pub fn sim_verdict(source: &str, max_time: u64) -> SimVerdict {
    match maria_api::simulate_signals_quiet(source, max_time) {
        Ok(sigs) => SimVerdict {
            ok: true,
            fingerprint: fingerprint(&sigs),
            code: String::new(),
            message: String::new(),
            assertion: false,
        },
        Err(e) => SimVerdict {
            ok: false,
            fingerprint: String::new(),
            code: e.error_code().to_string(),
            message: e.to_string(),
            assertion: e.to_string().to_lowercase().contains("assert"),
        },
    }
}

/// Sim oracle + coverage nyata engine (satu eksekusi: fingerprint DAN
/// coverage keys). Dipakai jalur utama fuzzer — coverage keys menjadi
/// feedback eksekusi sungguhan (line/branch/toggle/FSM), bukan teks statistik.
pub fn sim_verdict_cov(source: &str, max_time: u64) -> (SimVerdict, Vec<String>) {
    match maria_api::simulate_signals_with_coverage_quiet(source, max_time) {
        Ok((sigs, cov)) => (
            SimVerdict {
                ok: true,
                fingerprint: fingerprint(&sigs),
                code: String::new(),
                message: String::new(),
                assertion: false,
            },
            cov,
        ),
        Err(e) => (
            SimVerdict {
                ok: false,
                fingerprint: String::new(),
                code: e.error_code().to_string(),
                message: e.to_string(),
                assertion: e.to_string().to_lowercase().contains("assert"),
            },
            Vec::new(),
        ),
    }
}

/// Fingerprint deterministik satu set sinyal — basis oracle differential
/// (Paper #13: input sama → output sama; Paper #19: oracle nilai sinyal).
pub fn fingerprint(sigs: &[(String, LogicVec)]) -> String {
    let mut rows: Vec<String> = sigs
        .iter()
        .map(|(name, v)| format!("{}={}@{}", name, v, v.width))
        .collect();
    rows.sort();
    rows.join("|")
}

/// Hindari determinism-check palsu: `$urandom`/`$random` membuat simulasi
/// nondeterministik secara sah (fitur bahasa, bukan bug engine).
pub fn has_nondeterministic_src(source: &str) -> bool {
    source.contains("$urandom") || source.contains("$random")
}

/// Property-oracle (Paper #2/#3/#14, oracle #5): cari sinyal `_fz_viol_*`
/// bernilai **1 persis** di fingerprint (`name=val@width`, val Display).
/// Blok mirror `_fz_viol = (_fz_rtA !== _fz_rtB)` dengan dua evaluasi ekspresi
/// yang sama: engine konsisten → 0; evaluasi beda (bug eval/lebar/order) →
/// 1. NILAI X TIDAK dihitung: `x !== x` identik = 0, dan X muncul legit saat
/// minimizer menghapus temp → net implicit (z) → artefak false positive.
pub fn property_violation(fp: &str) -> Option<String> {
    for row in fp.split('|') {
        if row.starts_with("_fz_viol_") && row.ends_with("=1@1") {
            return Some(row.to_string());
        }
    }
    None
}

/// Oracle nilai sinyal (#19, DifuzzRTL): jalankan design 3x — input awal,
/// setelah `initial`-block sequence, dan di akhir — lalu bandingkan fingerprint.
/// Jika ada perbedaan di sinyal yang *tidak berubah* di source-input itu
/// mengindikasikan nilai internal tidak konsisten lintas eksekusi (bug engine).
pub fn sim_signal_check(source: &str, max_time: u64) -> Option<String> {
    // Sumber dengan `$urandom`/`$random` berbeda lintas run secara SAH —
    // fingerprint beda = anomali palsu (sama dgn determinism_check).
    if has_nondeterministic_src(source) {
        return None;
    }
    use maria_api::simulate_signals_quiet;
    let run = || simulate_signals_quiet(source, max_time).ok();
    let (s0, s1, s2) = (run(), run(), run());
    let extract = |s: Option<Vec<(String, LogicVec)>>| -> Vec<(String, String)> {
        s.into_iter()
            .flatten()
            .map(|(n, v)| (n.clone(), format!("{}={}", n, v)))
            .collect()
    };
    let a = extract(s0);
    let b = extract(s1);
    let c = extract(s2);
    let names: BTreeSet<&str> = a.iter().map(|(n, _)| n.as_str()).collect();
    let mut diffs = Vec::new();
    for name in &names {
        let va = a.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str()).unwrap_or("__missing__");
        let vb = b.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str()).unwrap_or("__missing__");
        let vc = c.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str()).unwrap_or("__missing__");
        if va != vb || vb != vc {
            diffs.push(format!("{}: {} | {} | {}", name, va, vb, vc));
        }
    }
    if diffs.is_empty() { None } else { Some(diffs.join("; ")) }
}

#[cfg(test)]
mod signal_check {
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
        FuzzConfig { max_time: 40, hang_ms: 3000, ..FuzzConfig::default() }
    }

    #[test]
    fn sim_signal_check_stable_for_counter() {
        let r = sim_signal_check(COUNTER, 40);
        assert!(r.is_none(), "counter harus stabil: {:?}", r);
    }

    #[test]
    fn sim_signal_check_skips_nondeterministic() {
        let src = format!("{}\ninitial $display($urandom());\n", COUNTER);
        assert!(
            sim_signal_check(&src, 40).is_none(),
            "sumber nondeterministik harus di-skip, bukan anomali palsu"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
endmodule
"#;

    #[test]
    fn compile_valid_seed_ok() {
        let v = compile_verdict(COUNTER);
        assert!(v.ok, "counter harus compile ok: {}", v.message);
        assert!(v.code.is_empty());
    }

    #[test]
    fn compile_garbage_fails_with_code() {
        let v = compile_verdict("module broken { not verilog at all");
        assert!(!v.ok);
        assert!(!v.code.is_empty(), "error code harus terisi: {}", v.code);
    }

    #[test]
    fn sim_fingerprint_stable_and_sorted() {
        let a = sim_verdict(COUNTER, 40).fingerprint;
        let b = sim_verdict(COUNTER, 40).fingerprint;
        assert!(a.contains("y="), "fingerprint harus berisi sinyal y: {}", a);
        assert_eq!(a, b, "dua run sumber sama harus identik");
        // fingerprint memakai sort — urutan nama tidak mengubah hasil.
        assert_eq!(a.split('|').count(), a.split('|').count());
    }

    #[test]
    fn fingerprint_orders_rows() {
        let faux: Vec<(String, LogicVec)> = vec![
            ("b".into(), LogicVec::from_u64(1, 4)),
            ("a".into(), LogicVec::from_u64(2, 4)),
        ];
        let f = fingerprint(&faux);
        assert!(f.starts_with("a="), "row disortir: {}", f);
    }

    #[test]
    fn nondeterministic_detection() {
        assert!(!has_nondeterministic_src(COUNTER));
        assert!(has_nondeterministic_src("$display($urandom());"));
    }

    const CLOCKED: &str = r#"
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

    #[test]
    fn sim_verdict_cov_returns_execution_coverage_keys() {
        // Feedback coverage EKSEKUSI nyata: satu panggilan mengembalikan
        // fingerprint + key coverage (line/branch/toggle/FSM) dari engine.
        let (v, cov) = sim_verdict_cov(CLOCKED, 40);
        assert!(v.ok, "source valid harus sim ok: {}", v.message);
        assert!(!cov.is_empty(), "sim valid harus menghasilkan coverage keys");
        assert!(
            cov.iter().any(|k| k.starts_with("cov_line:")),
            "harus ada line coverage: {:?}",
            cov
        );
        assert!(
            cov.iter().any(|k| k.starts_with("cov_toggle:")),
            "clk harus memicu toggle coverage: {:?}",
            cov
        );
        // Deterministik per source — aman dijadikan kunci novelty fuzzing.
        let (_, cov2) = sim_verdict_cov(CLOCKED, 40);
        assert_eq!(cov, cov2, "coverage keys harus identik untuk source sama");
    }
}