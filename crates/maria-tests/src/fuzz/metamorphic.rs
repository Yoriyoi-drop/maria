//! Metamorphic testing — transformasi input yang WAJIB tidak mengubah
//! hasil semantik.
//!
//! Satu file = satu tanggung jawab: invarian transformasi source-to-source.
//! Jika hasil simulasi berubah tanpa alasan valid → BUG / INVESTIGATE.
//!
//! Transformasi:
//! 1. Whitespace & komentar ekstra
//! 2. Rename identifier konsisten (a→sig_a, y→out_y)
//! 3. Kurung redundan di sekeliling RHS
//! 4. Literal ekuivalen: 8'd5 ↔ 8'b101 ↔ 8'h5A ↔ 8'o132

use super::gen::generate;

/// Simulasi lalu ambil nilai sinyal `name` (None = sim error/sinyal hilang).
fn sim_signal(src: &str, max_time: u64, name: &str) -> Option<u64> {
    crate::simulate_signals(src, max_time)
        .ok()?
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.to_u64())
}

/// Tambah komentar + whitespace bebas tanpa mengubah token stream.
fn add_noise(src: &str) -> String {
    let mut out = String::new();
    for line in src.lines() {
        out.push_str("   // noise\r\n");
        out.push_str("  ");
        out.push_str(line);
        out.push_str("  \n");
    }
    out.push_str("/* trailing\nmultiline comment */\n");
    out
}

/// Rename konsisten semua kemunculan identifier utuh.
fn rename_ident(src: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < src.len() {
        if src[i..].starts_with(from)
            && (i == 0 || !is_ident_byte(bytes[i - 1]))
            && (i + from.len() == src.len() || !is_ident_byte(bytes[i + from.len()]))
        {
            out.push_str(to);
            i += from.len();
        } else {
            // Byte-per-byte aman: identifier ASCII, byte lain disalin utuh
            // sebagai char Latin-1 hanya untuk perbandingan — gunakan push_str
            // dari slice agar UTF-8 multi-byte tetap utuh.
            let ch_len = utf8_len(bytes[i]);
            out.push_str(&src[i..i + ch_len]);
            i += ch_len;
        }
    }
    out
}

fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else {
        4
    }
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

#[test]
fn metamorphic_whitespace_comments_invariant() {
    for seed in [1u64, 7, 42, 100, 999, 12_345, 77_777, 313_131] {
        let input = generate(seed);
        let base = input.to_source();
        let noisy = add_noise(&base);
        assert_eq!(
            sim_signal(&base, 20, "y"),
            sim_signal(&noisy, 20, "y"),
            "seed={} whitespace/comment mengubah hasil",
            seed
        );
    }
}

#[test]
fn metamorphic_rename_invariant() {
    for seed in [2u64, 8, 43, 101, 1_000, 50_505] {
        let input = generate(seed);
        let base = input.to_source();
        let expected = input.expr.eval(input.w, input.a, input.b);
        // a → sig_a dulu, lalu y → out_y (rename kedua tidak menyentuh sig_a).
        let renamed = rename_ident(&rename_ident(&base, "a", "sig_a"), "y", "out_y");
        assert_eq!(
            sim_signal(&base, 20, "y"),
            sim_signal(&renamed, 20, "out_y"),
            "seed={} rename mengubah hasil:\n{}",
            seed,
            renamed
        );
        assert_eq!(
            sim_signal(&renamed, 20, "out_y"),
            Some(expected),
            "seed={} rename kehilangan nilai y",
            seed
        );
    }
}

#[test]
fn metamorphic_redundant_parens_invariant() {
    for seed in [3u64, 9, 44, 102, 2_000] {
        let input = generate(seed);
        let base = input.to_source();
        let expr_sv = input.expr.to_sv(input.w);
        let parenthesized = base.replace(
            &format!("assign y = {};", expr_sv),
            &format!("assign y = (({}));", expr_sv),
        );
        assert_ne!(parenthesized, base, "transformasi gagal diterapkan");
        assert_eq!(
            sim_signal(&base, 20, "y"),
            sim_signal(&parenthesized, 20, "y"),
            "seed={} kurung redundan mengubah hasil",
            seed
        );
    }
}

#[test]
fn metamorphic_literal_form_invariant() {
    let variants = [
        "module m; wire [7:0] w = 8'd90; assign y = w; initial #1 $finish; endmodule",
        "module m; wire [7:0] w = 8'b01011010; assign y = w; initial #1 $finish; endmodule",
        "module m; wire [7:0] w = 8'h5A; assign y = w; initial #1 $finish; endmodule",
        "module m; wire [7:0] w = 8'o132; assign y = w; initial #1 $finish; endmodule",
        "module m; wire [7:0] w = 'sd90; assign y = w; initial #1 $finish; endmodule",
        "module m; wire [7:0] w = 8'd0000_090; assign y = w; initial #1 $finish; endmodule",
    ];
    let results: Vec<Option<u64>> = variants
        .iter()
        .map(|v| sim_signal(v, 10, "y"))
        .collect();
    for (i, r) in results.iter().enumerate() {
        eprintln!("[lit-form] {} => {:?}", variants[i], r);
    }
    for (i, r) in results.iter().enumerate().skip(1) {
        assert_eq!(results[0], *r, "literal form {} ≠ form 0", i);
    }
    assert_eq!(results[0], Some(90));
}

#[test]
fn metamorphic_expr_eval_matches_sim_on_generated() {
    // Differential model-vs-sim batch besar (regression net pipeline).
    let mask = |w: u32| if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
    let mut mismatch = Vec::new();
    for seed in 0..120u64 {
        let input = generate(seed.wrapping_mul(7919));
        if input.expr.eval_has_x(input.w, input.a, input.b) {
            continue;
        }
        let src = input.to_source();
        let expected = input.expr.eval(input.w, input.a, input.b) & mask(input.w);
        let actual = sim_signal(&src, 20, "y");
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} expr=`{}` exp={:#x} act={:?}\n{}",
                input.seed,
                input.expr.to_sv(input.w),
                expected,
                actual,
                src
            ));
        }
    }
    assert!(
        mismatch.is_empty(),
        "{} mismatch:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
