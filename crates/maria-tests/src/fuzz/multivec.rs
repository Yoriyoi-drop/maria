//! Differential multi-vektor — stimulasi BERURUTAN pada modul yang sama.
//!
//! Satu file = satu tanggung jawab: invarian re-evaluasi kontinu. Blind spot
//! fuzzer lama: tiap modul hanya mengevaluasi `y` SATU kali (a/b di-drive
//! sekali di `initial`) sehingga bug jalur re-evaluasi tidak mungkin
//! terdeteksi — mis. sensitivitas `assign` yang melewatkan perubahan sinyal,
//! cache nilai basi, atau update parsial antar proses.
//!
//! Pola: a/b didrive k vektor berurutan; setelah tiap langkah, nilai `y`
//! di-capture ke reg `m<i>`; nilai akhir semua `m<i>` dibandingkan dengan
//! model emas `Expr::eval` untuk vektor tersebut.

use super::gen::{generate, lit_sv, mask_of, GenInput};

/// Jumlah langkah stimulasi (vektor pertama = input.a/b generator, sisanya
/// acak dari seed turunan).
const STEPS: usize = 8;

/// Source modul multi-vektor: tiap langkah `#2 m<i> = y;`.
fn multivec_source(input: &GenInput, vecs: &[(u64, u64)]) -> String {
    let w = input.w;
    let expr_sv = input.expr.to_sv(w);
    let mut decls = String::new();
    let mut steps = String::new();
    let wb = input.wb;
    for i in 0..vecs.len() {
        if i > 0 {
            decls.push_str("    \n");
        }
        decls.push_str(&format!("    reg [{}:0] m{};", w - 1, i));
        steps.push_str(&format!(
            "        a = {};\n        b = {};\n        #2 m{} = y;\n",
            lit_sv(vecs[i].0, w),
            lit_sv(vecs[i].1, wb),
            i
        ));
    }
    format!(
        "module fuzz_mod;\n\
         \x20   reg [{hi}:0] a;\n\
         \x20   reg [{hi}:0] b;\n\
         \x20   wire [{hi}:0] y;\n\
         \x20   assign y = {expr};\n\
         {decls}\n\
         \x20   initial begin\n\
         {steps}\
         \x20       #2 $finish;\n\
         \x20   end\n\
         endmodule\n",
        hi = w - 1,
        expr = expr_sv,
        decls = decls,
        steps = steps,
    )
}

#[test]
fn multivec_sequential_stimulus_matches_oracle() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..60u64 {
        let input = generate(seed.wrapping_mul(104_729).wrapping_add(17));
        // Vektor pertama dari generator; sisanya acak deterministik.
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xA5A5_5A5A);
        let m = mask_of(input.w);
        let mb = mask_of(input.wb);
        let mut vecs: Vec<(u64, u64)> = Vec::with_capacity(STEPS + 1);
        vecs.push((input.a, input.b));
        for _ in 0..STEPS {
            vecs.push((rng.u64(0..) & m, rng.u64(0..) & mb));
        }
        // Skip bila model emas tak comparable (X / intermediate >128 bit)
        // pada vektor mana pun — invariant panic/determinism tetap terlatih.
        if vecs
            .iter()
            .any(|&(a, b)| input.expr.eval_has_x(input.w, a, b))
        {
            continue;
        }
        if input.expr.max_width(input.w as u64) > 128 {
            continue;
        }
        let src = multivec_source(&input, &vecs);
        // Total waktu: (STEPS+1)*2 unit + margin $finish.
        let max_time = ((STEPS + 2) * 3 + 10) as u64;
        // Jalankan di thread stack besar (pola suite): parser rekursif pada
        // ekspresi ber-kurung dalam melebihi stack thread test default.
        let sigs = {
            let src = src.clone();
            std::thread::Builder::new()
                .name("multivec-sim".to_string())
                .stack_size(256 * 1024 * 1024)
                .spawn(move || crate::simulate_signals(&src, max_time))
                .expect("spawn multivec-sim")
                .join()
                .expect("sim panic")
        };
        let sigs = match sigs {
            Ok(s) => s,
            Err(e) => {
                mismatch.push(format!("seed={} sim error: {:?}\n{}", seed, e, src));
                continue;
            }
        };
        for (i, &(a, b)) in vecs.iter().enumerate() {
            let expected = input.expr.eval(input.w, a, b) & mask_of(input.w);
            let name = format!("m{}", i);
            let actual = sigs.iter().find(|(n, _)| *n == name).map(|(_, v)| v.to_u64());
            if actual != Some(expected) {
                mismatch.push(format!(
                    "seed={} step={} a={:#x} b={:#x} expr=`{}` exp={:#x} act={:?}\n{}",
                    input.seed,
                    i,
                    a,
                    b,
                    input.expr.to_sv(input.w),
                    expected,
                    actual,
                    src
                ));
            }
        }
        checked += 1;
    }
    assert!(
        checked > 30,
        "terlalu sedikit vektor tereksekusi (checked={}) — generator rusak?",
        checked
    );
    assert!(
        mismatch.is_empty(),
        "{} mismatch re-evaluasi:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
