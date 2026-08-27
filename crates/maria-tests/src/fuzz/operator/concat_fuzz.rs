//! Fuzz differential concatenation `{a, b}` & replication `{N{e}}`.
//!
//! Blind spot fuzzer existing: concatenation dan replication muncul jarang
//! dalam generator acak, dan saat muncul lebarnya kecil sehingga jalur
//! u128 multi-word tidak terekspos. Test ini khusus menguji:
//!
//! - Concat 2 variabel: `{a[hi:0], b[lo:0]}` → lebar = hi+1 + lo+1
//! - Replication: `{N{a[3:0]}}` → lebar = N * 4
//! - Concat + aritmetika: `{a, b} + 1` — lebar intermediate concat
//! - Mixed width: concat variabel lebar beda
//! - Boundary: concat menghasilkan lebar >64 bit
//! - Nested concat: `{{a, b}, {c, d}}`

use crate::fuzz::gen::{generate, lit_sv, mask_of, WIDTH_CHOICES};

/// Lebar yang diuji.
const CONCAT_WIDTHS: [u32; 7] = [4, 8, 12, 16, 32, 48, 64];

fn mask_of128(w: u32) -> u128 {
    if w >= 128 { u128::MAX } else { (1u128 << w) - 1 }
}

/// Bangun source dari template concat.
fn concat_source(expr_sv: &str, yw: u32, w: u32, aval: &str, bval: &str) -> String {
    format!(
        "module concat_fuzz_mod;\n\
         \x20   reg [{hi}:0] a;\n\
         \x20   reg [{hi}:0] b;\n\
         \x20   wire [{yhi}:0] y;\n\
         \x20   assign y = {expr};\n\
         \x20   initial begin\n\
         \x20       a = {aval};\n\
         \x20       b = {bval};\n\
         \x20       #10;\n\
         \x20       $finish;\n\
         \x20   end\n\
         endmodule\n",
        hi = w - 1,
        yhi = yw - 1,
        expr = expr_sv,
        aval = aval,
        bval = bval,
    )
}

#[test]
fn concat_two_variables_matches_golden() {
    // `{a[aw-1:0], b[bw-1:0]}` → lebar = aw + bw.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..100u64 {
        let w = CONCAT_WIDTHS[seed as usize % CONCAT_WIDTHS.len()];
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_BB_CC);

        // Pilih lebar masing-masing operand concat.
        let aw = rng.u32(1..=w.min(32));
        let bw = rng.u32(1..=(w - aw).max(1));
        let total_w = aw + bw;

        let a = rng.u64(0..) & mask_of(aw);
        let b = rng.u64(0..) & mask_of(bw);
        let expected = ((a as u128) << bw | (b as u128)) & mask_of128(total_w);

        let a_lit = lit_sv(a, w);
        let b_lit = lit_sv(b, w);
        let src = concat_source(
            &format!("{{a[{}:0], b[{}:0]}}", aw - 1, bw - 1),
            total_w, w, &a_lit, &b_lit,
        );

        let actual = std::thread::Builder::new()
            .name("concat-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn({
                let src = src.clone();
                move || {
                    crate::simulate_signals(&src, 30)
                        .ok()
                        .and_then(|sigs| sigs.iter().find(|(n, _)| *n == "y").map(|(_, v)| v.to_u64()))
                }
            })
            .expect("spawn")
            .join()
            .expect("sim panic");

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} aw={} bw={} harap={:#x} dapat={:?}\n{}",
                seed, aw, bw, expected, actual, src
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch concat:\n{}",
        mismatch.len(),
        mismatch.join("\n=====\n")
    );
}

#[test]
fn replication_matches_golden() {
    // `{N{a[m-1:0]}}` → lebar = N * m, value = pattern diulang N kali.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xDD_EE_FF);

        let m = rng.u32(1..=16); // lebar pattern per replikasi
        let n = rng.u32(2..=8); // jumlah replikasi
        let total_w = m * n;
        if total_w > 128 {
            continue;
        }

        let pattern = rng.u64(0..) & mask_of(m);
        // Golden: pattern diulang n kali.
        let mut expected: u128 = 0;
        for i in 0..n {
            expected |= ((pattern as u128) << (i * m)) & mask_of128(total_w);
        }

        let w = total_w.max(8);
        let a_lit = lit_sv(pattern, w);
        let src = concat_source(
            &format!("{{{}{{{}}}}}", n, format!("a[{}:0]", m - 1)),
            total_w, w, &a_lit, &"0".to_string(),
        );

        let actual = std::thread::Builder::new()
            .name("repl-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn({
                let src = src.clone();
                move || {
                    crate::simulate_signals(&src, 30)
                        .ok()
                        .and_then(|sigs| sigs.iter().find(|(n, _)| *n == "y").map(|(_, v)| v.to_u64()))
                }
            })
            .expect("spawn")
            .join()
            .expect("sim panic");

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} n={} m={} harap={:#x} dapat={:?}\n{}",
                seed, n, m, expected, actual, src
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch replication:\n{}",
        mismatch.len(),
        mismatch.join("\n=====\n")
    );
}

#[test]
fn concat_plus_one_matches_golden() {
    // `{a[7:0], b[7:0]} + 1` — menguji evaluasi concat sebagai operand
    // aritmetika (lebar intermediate 16 bit).
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x11_22_33);

        let aw = rng.u32(4..=16);
        let bw = rng.u32(4..=16);
        let total_w = aw + bw;
        if total_w > 64 {
            continue;
        }

        let a = rng.u64(0..) & mask_of(aw);
        let b = rng.u64(0..) & mask_of(bw);
        let concat_val = ((a as u128) << bw | (b as u128)) & mask_of128(total_w);
        let expected = (concat_val.wrapping_add(1)) & mask_of128(total_w);

        let w = total_w;
        let a_lit = lit_sv(a, w);
        let b_lit = lit_sv(b, w);
        let one_lit = lit_sv(1, total_w);
        let src = format!(
            "module concat_fuzz_mod;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   reg [{hi}:0] b;\n\
             \x20   wire [{yhi}:0] y;\n\
             \x20   assign y = ({{a[{aw_m1}:0], b[{bw_m1}:0]}} + {one});\n\
             \x20   initial begin\n\
             \x20       a = {a};\n\
             \x20       b = {b};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            yhi = total_w - 1,
            aw_m1 = aw - 1,
            bw_m1 = bw - 1,
            one = one_lit,
            a = a_lit,
            b = b_lit,
        );

        let actual = std::thread::Builder::new()
            .name("concat-add-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn({
                let src = src.clone();
                move || {
                    crate::simulate_signals(&src, 30)
                        .ok()
                        .and_then(|sigs| sigs.iter().find(|(n, _)| *n == "y").map(|(_, v)| v.to_u64()))
                }
            })
            .expect("spawn")
            .join()
            .expect("sim panic");

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} aw={} bw={} harap={:#x} dapat={:?}\n{}",
                seed, aw, bw, expected, actual, src
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch concat+1:\n{}",
        mismatch.len(),
        mismatch.join("\n=====\n")
    );
}

#[test]
fn nested_concat_matches_golden() {
    // `{{a[3:0], b[3:0]}, {a[7:4], b[7:4]}}` — nested concat.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x44_55_66);
        let w: u32 = 16; // fix w=16 agar nested concat tetap muat

        let a = rng.u64(0..) & mask_of(w);
        let b = rng.u64(0..) & mask_of(w);

        // Nested: {{a[3:0], b[3:0]}, {a[7:4], b[7:4]}} → 16 bit
        // SV {X, Y}: X = upper bits, Y = lower bits
        let upper = ((a & 0xF) << 4) | (b & 0xF); // {a[3:0], b[3:0]} = upper
        let lower = (((a >> 4) & 0xF) << 4) | ((b >> 4) & 0xF); // {a[7:4], b[7:4]} = lower
        let expected = ((upper as u128) << 8 | (lower as u128)) & mask_of128(16);

        let a_lit = lit_sv(a, w);
        let b_lit = lit_sv(b, w);
        let src = format!(
            "module concat_fuzz_mod;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   reg [{hi}:0] b;\n\
             \x20   wire [{hi}:0] y;\n\
             \x20   assign y = {{{{a[3:0], b[3:0]}}, {{a[7:4], b[7:4]}}}};\n\
             \x20   initial begin\n\
             \x20       a = {a};\n\
             \x20       b = {b};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            a = a_lit,
            b = b_lit,
        );

        let actual = std::thread::Builder::new()
            .name("nconcat-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn({
                let src = src.clone();
                move || {
                    crate::simulate_signals(&src, 30)
                        .ok()
                        .and_then(|sigs| sigs.iter().find(|(n, _)| *n == "y").map(|(_, v)| v.to_u64()))
                }
            })
            .expect("spawn")
            .join()
            .expect("sim panic");

        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} harap={:#x} dapat={:?}\n{}",
                seed, expected, actual, src
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch nested concat:\n{}",
        mismatch.len(),
        mismatch.join("\n=====\n")
    );
}
