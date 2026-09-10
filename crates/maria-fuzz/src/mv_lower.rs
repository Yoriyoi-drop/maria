//! Lowering MV → HDL + gate validasi (task §7).
//!
//! Fuzzer menghasilkan `.mv`; `mv_lower` mengubahnya menjadi HDL SV
//! (svh+sv digabung, baris `` `include `` di-strip — sama persis dgn
//! `maria-tools::transpile_mv_to_inline`) dan mengklasifikasikan hasil:
//!
//! - `Hdl`          — lower sukses (dengan type-check) → HDL siap eksekusi.
//! - `HdlNoCheck`   — `transpile_no_check` (band "expected-invalid", mis.
//!                    lebar-truncation) — tetap menghasilkan HDL utk fuzz
//!                    parser/elaborator maria (jangan buang agresif).
//! - `MvReject(code,msg)` — Maria-MV menolak source (E2001..E2007) — ini
//!                    BUG di maria-mv atau testcase fuzzer, BUKAN bug Maria.
//!
//! Pemisahan klasifikasi ini menjamin pipeline membedakan bug Maria-MV dari
//! bug Maria (task §18/§19.G).
//!
//! 1 file = 1 tanggung jawab: hanya transformasi + klasifikasi.

/// Verdict lower.
#[derive(Debug, Clone, PartialEq)]
pub enum LowerVerdict {
    /// Type-check ok, HDL teremisi.
    Hdl(String),
    /// `--no-check` escape: HDL tetap teremisi (tanpa validasi semantik).
    HdlNoCheck(String),
    /// Maria-MV menolak source (parse/check error) — bukan jalan ke Maria.
    MvReject { code: String, msg: String },
}

/// Gabung svh + sv menjadi satu buffer, strip `` `include `` (definisi bersama
/// sudah di atasnya) — identik dengan `transpile_mv_to_inline`.
pub fn merge_svh_sv(svh: &str, sv: &str) -> String {
    let mut buf = String::new();
    buf.push_str(svh);
    buf.push('\n');
    for line in sv.lines() {
        let t = line.trim_start();
        if t.starts_with("`include") {
            continue;
        }
        buf.push_str(line);
        buf.push('\n');
    }
    buf
}

/// Lower source `.mv` → HDL. `base` = nama file (mis. `fz_mv`).
pub fn lower_mv(src: &str, base: &str) -> LowerVerdict {
    // Jalur utama: transpile dgn type-check (validasi SEBELUM emisi).
    match maria_api::mv::transpile(src, base) {
        Ok(tr) => LowerVerdict::Hdl(merge_svh_sv(&tr.svh, &tr.sv)),
        Err(_) => {
            // Band "expected-invalid": kode yang check() tolak (E2002 lebar,
            // E2003 arah, E2004 NBA) tetap bisa berupa SV valid-ish—lower
            // tanpa check biar parser/elaborator maria tetap di-fuzz.
            match maria_api::mv::transpile_no_check(src, base) {
                Ok(tr) => LowerVerdict::HdlNoCheck(merge_svh_sv(&tr.svh, &tr.sv)),
                Err(e) => LowerVerdict::MvReject {
                    code: e.msg.split(']').next().unwrap_or("E").trim_start_matches('[').to_string(),
                    msg: e.format(),
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COUNTER_MV: &str = r#"
module fz_mv {
    in  clk, rst_n : bit
    in  a, b       : logic[7:0]
    out y          : logic[7:0]
    sig t          : logic[7:0]
    comb {
        t = a + b
        y = t
    }
    seq(clk, rst_n) {
        if (!rst_n) {
            t <= '0
        }
    }
}
"#;

    #[test]
    fn lower_valid_mv_ok() {
        match lower_mv(COUNTER_MV, "fz_mv") {
            LowerVerdict::Hdl(hdl) => {
                assert!(hdl.contains("module fz_mv"));
                assert!(hdl.contains("always_comb"));
                assert!(!hdl.contains("`include"));
            }
            other => panic!("harus Hdl: {:?}", other),
        }
    }

    #[test]
    fn lower_syntax_error_rejects() {
        match lower_mv("module {", "bad") {
            LowerVerdict::MvReject { code, .. } => assert!(!code.is_empty()),
            other => panic!("syntax error harus MvReject: {:?}", other),
        }
    }

    #[test]
    fn lower_width_mismatch_no_check() {
        // E2002 (truncation) ditolak check() tapi lower no-check tetap emisi —
        // kelas "expected-invalid" yang masih bisa fuzz parser maria.
        let src = "module w {\n    in a : logic[7:0]\n    out y : logic[3:0]\n    comb {\n        y = a\n    }\n}\n";
        match lower_mv(src, "w") {
            LowerVerdict::HdlNoCheck(hdl) => assert!(hdl.contains("module w")),
            v => panic!("width-mismatch harus HdlNoCheck, dapat {:?}", v),
        }
    }

    #[test]
    fn merge_strips_include() {
        let svh = "`ifndef X\n`define X\npackage p; endpackage\n`endif\n";
        let sv = "`include \"f.svh\"\nmodule m; endmodule\n";
        let m = merge_svh_sv(svh, sv);
        assert!(m.contains("package p"));
        assert!(m.contains("module m"));
        assert!(!m.contains("`include"));
    }
}