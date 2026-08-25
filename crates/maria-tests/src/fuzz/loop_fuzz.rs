//! Differential statement loop — `for` + `break`/`continue` + akumulator.
//!
//! Satu file = satu tanggung jawab: invarian eksekusi loop prosedural.
//! Mesin kontrol-alur loop (FlowControl Break/Continue per iterasi) adalah
//! jalur engine yang rapuh dan belum tersentuh fuzzer lain. Pola-pola di
//! sini punya model emas Rust yang sepele diverifikasi:
//! - Akumulasi modular (`r = r + a[i]`)
//! - Shift-in serial (`r = {r[rw-2:0], a[i]}`)
//! - Early-exit `break` (hitung leading-zero)
//! - Skip `continue` (hitung nol × konstanta)
//! - Loop bersarang (perkalian via penjumlahan berulang)

use super::gen::{lit_sv, mask_of, WIDTH_CHOICES};

/// Simulasi + baca reg `r` (None = error/sinyal hilang). Thread stack besar.
fn sim_r(src: &str) -> Option<u64> {
    let src = src.to_string();
    let handle = std::thread::Builder::new()
        .name("loop-fuzz-sim".to_string())
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            crate::simulate_signals(&src, 60)
                .ok()
                .and_then(|sigs| sigs.iter().find(|(n, _)| *n == "r").map(|(_, v)| v.to_u64()))
        })
        .expect("spawn loop-fuzz-sim");
    handle.join().expect("sim panic")
}

/// Pilih lebar variabel ≥ 8 agar index loop 0..N selalu valid.
fn pick_w(rng: &mut fastrand::Rng) -> u32 {
    let candidates: Vec<u32> = WIDTH_CHOICES.iter().copied().filter(|&c| c >= 8).collect();
    candidates[rng.usize(0..candidates.len())]
}

fn module_src(w: u32, a_lit: &str, b_lit: &str, body: &str) -> String {
    format!(
        "module loop_fuzz_mod;\n\
         \x20   reg [{hi}:0] a;\n\
         \x20   reg [{hi}:0] b;\n\
         \x20   reg [{hi}:0] r;\n\
         \x20   integer i;\n\
         \x20   integer j;\n\
         \x20   initial begin\n\
         \x20       a = {a_lit};\n\
         \x20       b = {b_lit};\n\
         {body}\
         \x20       #10 $finish;\n\
         \x20   end\n\
         endmodule\n",
        hi = w - 1,
        a_lit = a_lit,
        b_lit = b_lit,
        body = body,
    )
}

fn check_case(
    seed: u64,
    src: &str,
    expected: u64,
    mask: u64,
    mismatches: &mut Vec<String>,
    checked: &mut u32,
) {
    let actual = sim_r(src);
    if actual != Some(expected & mask) {
        mismatches.push(format!(
            "seed={} exp={:#x} act={:?}\n{}",
            seed, expected, actual, src
        ));
    }
    *checked += 1;
}

#[test]
fn loop_for_accumulate_shift_break_continue_match_golden() {
    let mut mismatches = Vec::new();
    let mut checked = 0u32;
    for seed in 0..90u64 {
        let mut rng = fastrand::Rng::with_seed(seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(11));
        let w = pick_w(&mut rng);
        let mask = mask_of(w);
        let a = rng.u64(0..) & mask;
        let b = rng.u64(0..) & mask;
        let n = rng.usize(3..=8) as u64;
        let a_lit = lit_sv(a, w);
        let b_lit = lit_sv(b, w);

        // ── Varian A: akumulasi modular ──
        // r = Σ a[i] (mod 2^w), i = 0..n
        {
            let body = format!(
                "        r = {};\n        for (i = 0; i < {}; i = i + 1) begin\n\
                 \x20           r = r + a[i];\n        end\n",
                lit_sv(0, w),
                n
            );
            let expected: u64 = (0..n).map(|i| (a >> i) & 1).sum();
            let src = module_src(w, &a_lit, &b_lit, &body);
            check_case(seed * 10, &src, expected, mask, &mut mismatches, &mut checked);
        }

        // ── Varian B: shift-in serial ──
        // r = 0; tiap iterasi r = {r[w-2:0], a[i]} → n bit pertama a masuk
        // dari LSB; hasil = (a & mask(n)) diputar ke posisi rendah.
        if w >= 2 {
            let body = format!(
                "        r = {};\n        for (i = 0; i < {}; i = i + 1) begin\n\
                 \x20           r = {{r[{}:0], a[i]}};\n        end\n",
                lit_sv(0, w),
                n,
                w - 2
            );
            // Golden: bit i dari a menempati posisi (n-1-i) — shift-in LSB.
            let mut expected = 0u64;
            for i in 0..n {
                let bit = (a >> i) & 1;
                expected |= bit << (n - 1 - i);
            }
            let src = module_src(w, &a_lit, &b_lit, &body);
            check_case(seed * 10 + 1, &src, expected, mask, &mut mismatches, &mut checked);
        }

        // ── Varian C: break — hitung leading-zero dari bit 0 ──
        {
            let body = format!(
                "        r = 0;\n        for (i = 0; i < {}; i = i + 1) begin\n\
                 \x20           if (a[i]) break;\n            r = r + 1;\n        end\n",
                n
            );
            let expected = (0..n).take_while(|&i| (a >> i) & 1 == 0).count() as u64;
            let src = module_src(w, &a_lit, &b_lit, &body);
            check_case(seed * 10 + 2, &src, expected, mask, &mut mismatches, &mut checked);
        }

        // ── Varian D: continue — jumlah bit-nol × 2 ──
        {
            let body = format!(
                "        r = 0;\n        for (i = 0; i < {}; i = i + 1) begin\n\
                 \x20           if (b[i]) continue;\n            r = r + 2;\n        end\n",
                n
            );
            let zeros = (0..n).filter(|&i| (b >> i) & 1 == 0).count() as u64;
            let src = module_src(w, &a_lit, &b_lit, &body);
            check_case(seed * 10 + 3, &src, zeros * 2, mask, &mut mismatches, &mut checked);
        }

        // ── Varian E: loop bersarang — perkaliaan berulang ──
        // r += popcount(b) untuk setiap bit-1 a di [0..n_outer)
        {
            let n_outer = rng.usize(2..=4) as u64;
            let n_inner = rng.usize(2..=4) as u64;
            let body = format!(
                "        r = 0;\n        for (i = 0; i < {}; i = i + 1) begin\n\
                 \x20           for (j = 0; j < {}; j = j + 1) begin\n\
                 \x20               if (a[i]) r = r + 1;\n\
                 \x20               if (b[j]) r = r + 1;\n            end\n        end\n",
                n_outer, n_inner
            );
            let ones_a = (0..n_outer).filter(|&i| (a >> i) & 1 == 1).count() as u64;
            let ones_b = (0..n_inner).filter(|&j| (b >> j) & 1 == 1).count() as u64;
            let expected = ones_a * n_inner + ones_b * n_outer;
            let src = module_src(w, &a_lit, &b_lit, &body);
            check_case(seed * 10 + 4, &src, expected, mask, &mut mismatches, &mut checked);
        }
    }
    assert!(checked > 200, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatches.is_empty(),
        "{} mismatch loop:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}
