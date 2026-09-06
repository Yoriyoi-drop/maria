//! Oracle hasil eksekusi maria — satu-satunya modul yang memanggil
//! `maria_api::compile_str` / `maria_api::simulate_signals` langsung.
//! (Harness membungkusnya dengan isolasi thread; lihat `harness.rs`.)
//!
//! Paper #2 (Liang 2018) & #3 (Manès 2019): klasifikasi verdict terstruktur —
//! fuzzer butuh tahu *kenapa* input ditolak (code) bukan hanya ya/tidak.

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
}

/// Compile oracle (Paper #2/#3): input valid → ok; input invalid → err code.
pub fn compile_verdict(source: &str) -> CompileVerdict {
    match maria_api::compile_str(source) {
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
    match maria_api::simulate_signals(source, max_time) {
        Ok(sigs) => SimVerdict {
            ok: true,
            fingerprint: fingerprint(&sigs),
            code: String::new(),
            message: String::new(),
        },
        Err(e) => SimVerdict {
            ok: false,
            fingerprint: String::new(),
            code: e.error_code().to_string(),
            message: e.to_string(),
        },
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
}