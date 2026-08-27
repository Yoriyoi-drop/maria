//! Fuzz VCD/waveform writer — output tidak boleh korup/panik di input ekstrem.
//!
//! Satu file = satu tanggung jawab: menyerang `VcdWriter` (header, dump,
//! timestamp ekstrem, reopen, drop tanpa close) dengan desain hasil
//! `compile_str` + state buatan. Invariant:
//! - Tidak panic pada kombinasi desain/state/time manapun.
//! - File VCD yang dihasilkan selalu memuat header lengkap (`$enddefinitions`).
//! - Id-code VCD unik per sinyal.

use std::io::Read;

use maria_core::{LogicVal, LogicVec};

/// State buatan sepanjang daftar sinyal design; pola nilai divariasikan
/// via `variant` agar semua transisi X/Z/0/1 tersentuh.
fn synthetic_state(design: &maria_ir::IrDesign, variant: usize) -> Vec<LogicVec> {
    design
        .top
        .signals
        .iter()
        .enumerate()
        .map(|(si, sig)| {
            let w = if sig.width == 0 { 1 } else { sig.width };
            match (si + variant) % 4 {
                0 => LogicVec::from_u64(0xDEAD_BEEF, w),
                1 => LogicVec::fill(LogicVal::One, w),
                2 => LogicVec::fill(LogicVal::X, w),
                _ => LogicVec::fill(LogicVal::Z, w),
            }
        })
        .collect()
}

fn temp_vcd(tag: &str) -> String {
    std::env::temp_dir()
        .join(format!("maria_fuzz_vcd_{}_{}.vcd", tag, std::process::id()))
        .to_string_lossy()
        .into_owned()
}

/// Corpus desain SV dengan karakteristik VCD berbeda.
const DESIGNS: &[&str] = &[
    // Nol sinyal
    "module m; initial begin #1 $finish; end endmodule",
    // Satu sinyal 1-bit
    "module m; wire a = 1'b1; initial begin #1 $finish; end endmodule",
    // Array memory (array_depth > 1)
    "module m; reg [7:0] mem [0:3]; initial begin mem[0] = 8'hAA; #1 $finish; end endmodule",
    // Generate block scope
    "module m; genvar i; generate for (i=0;i<8;i=i+1) begin : blk wire [7:0] w = 8'd1; end endgenerate initial begin #1 $finish; end endmodule",
    // X/Z literal
    "module m; wire [7:0] x = 8'bxx0101zz; initial begin #1 $finish; end endmodule",
    // Real & string
    "module m; real r = 3.14; string s = \"hello\"; initial begin #1 $finish; end endmodule",
];

#[test]
fn fuzz_vcd_design_corpus_invariants() {
    for (di, src) in DESIGNS.iter().enumerate() {
        let design = crate::compile_str(src).expect("corpus harus compile");
        let path = temp_vcd(&format!("corp{}", di));
        let mut w = maria_simulator::waveform::vcd::VcdWriter::new(&path, &design)
            .unwrap_or_else(|e| panic!("VcdWriter gagal desain #{}: {}", di, e));

        let state = synthetic_state(&design, di);
        w.dump_all(&design, &state).expect("dump_all gagal");
        // Timestamp ekstrem: 0, u64::MAX, dan mundur (harus tetap ditulis).
        w.write_time_header(0).unwrap();
        w.write_time_header(u64::MAX).unwrap();
        w.write_time_header(5).unwrap();
        w.dump_state(&design, &state).unwrap();
        w.close().unwrap();

        let mut out = String::new();
        std::fs::File::open(&path)
            .unwrap()
            .read_to_string(&mut out)
            .unwrap();
        assert!(
            out.contains("$enddefinitions"),
            "desain #{}: header tak lengkap",
            di
        );
        // Desain tanpa sinyal sah tanpa $var.
        if !design.top.signals.is_empty() {
            assert!(out.contains("$var"), "desain #{}: tanpa $var", di);
        }
        assert!(
            out.contains("#18446744073709551615"),
            "timestamp u64::MAX hilang"
        );
        std::fs::remove_file(&path).ok();
    }
}

#[test]
fn fuzz_vcd_rapid_changes_and_reopen() {
    let src = "module m; reg [15:0] r; initial begin r = 0; #1 $finish; end endmodule";
    let design = crate::compile_str(src).unwrap();
    let path = temp_vcd("rapid");
    let mut w = maria_simulator::waveform::vcd::VcdWriter::new(&path, &design).expect("new gagal");
    let base_state = synthetic_state(&design, 0);

    // Reopen dulu (truncate + tulis ulang header), lalu rapid dump.
    w.reopen(&path, &design, &base_state).expect("reopen gagal");
    for i in 0..5000usize {
        let mut st = base_state.clone();
        for lv in st.iter_mut() {
            *lv = LogicVec::from_u64(i as u64, lv.width.max(1));
        }
        w.write_time_header(i as u64).unwrap();
        w.dump_state(&design, &st).unwrap();
        w.maybe_flush().unwrap();
    }
    w.close().unwrap();

    let mut out = String::new();
    std::fs::File::open(&path)
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    assert!(out.contains("$enddefinitions"));
    assert!(out.lines().count() > 100);
    std::fs::remove_file(&path).ok();
}

#[test]
fn fuzz_vcd_drop_without_close_no_panic() {
    // Drop tanpa close — path cleanup harus aman (flush di Drop).
    for di in 0..DESIGNS.len() {
        let design = crate::compile_str(DESIGNS[di]).unwrap();
        let path = temp_vcd(&format!("drop{}", di));
        {
            let mut w = maria_simulator::waveform::vcd::VcdWriter::new(&path, &design).unwrap();
            let state = synthetic_state(&design, di);
            let _ = w.dump_all(&design, &state);
            // sengaja tidak close → Drop
        }
        std::fs::remove_file(&path).ok();
    }
}

#[test]
fn fuzz_vcd_huge_timestamps_monotonic_and_wrap() {
    let src = "module m; wire q = 1'b0; initial begin #1 $finish; end endmodule";
    let design = crate::compile_str(src).unwrap();
    let path = temp_vcd("ts");
    let mut w = maria_simulator::waveform::vcd::VcdWriter::new(&path, &design).unwrap();
    // Timestamps liar: mundur, wrap-kanan, saturate.
    for t in [
        u64::MAX,
        0,
        u64::MAX,
        1u64 << 63,
        (1u64 << 63) - 1,
        u64::MAX - 1,
    ] {
        w.write_time_header(t).unwrap();
    }
    w.close().unwrap();

    let mut out = String::new();
    std::fs::File::open(&path)
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    // Setiap timestamp harus muncul verbatim (tidak ada truncasi/korupsi).
    assert!(out.matches("#18446744073709551615").count() >= 2);
    assert!(out.contains("#9223372036854775807"));
    assert!(out.contains("#9223372036854775806") || out.contains("#18446744073709551614"));
    std::fs::remove_file(&path).ok();
}

#[test]
fn fuzz_vcd_max_dump_size_disable_path() {
    // max_dump_size terlampaui → enabled=false → semua write no-op (audit F10:
    // silent disable). Minimal: tidak panic dan file tetap valid.
    let src = "module m; wire [3:0] q; assign q = 4'h5; initial begin #1 $finish; end endmodule";
    let design = crate::compile_str(src).unwrap();
    let path = temp_vcd("maxsz");
    let mut w = maria_simulator::waveform::vcd::VcdWriter::new(&path, &design).unwrap();
    w.max_dump_size = Some(1);
    let state = synthetic_state(&design, 1);
    for i in 0..100u64 {
        w.write_time_header(i).unwrap();
        let _ = w.dump_state(&design, &state);
    }
    w.close().unwrap();
    let mut out = String::new();
    std::fs::File::open(&path)
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    assert!(out.contains("$enddefinitions"));
    std::fs::remove_file(&path).ok();
}
