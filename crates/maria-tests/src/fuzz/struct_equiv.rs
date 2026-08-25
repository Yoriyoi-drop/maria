//! Metamorphic struktural — ekspresi yang sama dihitung lewat 3 gaya penugasan
//! kombinasional yang berbeda dan WAJIB menghasilkan nilai identik.
//!
//! Satu file = satu tanggung jawab: invarian gaya-penugasan. Tidak ada model
//! emas eksternal — ketiga varian saling menjadi saksi (self-consistency):
//! 1. `assign y = E;`          — continuous assignment (net).
//! 2. `always @(*) y = E;`     — proses kombinational, blocking assign (reg).
//! 3. `always @(*) y <= E;`    — proses kombinational, nonblocking assign.
//!
//! Ketidakcocokan = bug sensitivitas/scheduling/penanganan reg-vs-net.

use super::gen::generate;

/// Nilai sinyal terakhir dari simulasi (None = error/sinyal hilang).
/// Thread stack besar — engine rekursif dalam (ekspresi depth 5).
fn sim_signal(src: &str, name: &str) -> Option<u64> {
    let src = src.to_string();
    let name = name.to_string();
    let handle = std::thread::Builder::new()
        .name("struct-equiv-sim".to_string())
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            crate::simulate_signals(&src, 30)
                .ok()?
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.to_u64())
        })
        .expect("spawn struct-equiv-sim");
    handle.join().expect("sim panic")
}

/// Bangun modul varian `kind`: 0 = assign, 1 = always @* blocking,
/// 2 = always @* nonblocking.
fn variant_source(expr_sv: &str, w: u32, aval: &str, bval: &str, kind: usize) -> String {
    let (y_decl, driver) = match kind {
        0 => (
            format!("    wire [{}:0] y;", w - 1),
            format!("    assign y = {};", expr_sv),
        ),
        1 => (
            format!("    reg [{}:0] y;", w - 1),
            format!(
                "    always @(*) begin\n        y = {};\n    end\n",
                expr_sv
            ),
        ),
        _ => (
            format!("    reg [{}:0] y;", w - 1),
            format!(
                "    always @(*) begin\n        y <= {};\n    end\n",
                expr_sv
            ),
        ),
    };
    format!(
        "module equiv_mod;\n\
         \x20   reg [{hi}:0] a;\n\
         \x20   reg [{hi}:0] b;\n\
         {y_decl}\n\
         {driver}\
         \x20   initial begin\n\
         \x20       a = {aval};\n\
         \x20       b = {bval};\n\
         \x20       #10;\n\
         \x20       $finish;\n\
         \x20   end\n\
         endmodule\n",
        hi = w - 1,
        y_decl = y_decl,
        driver = driver,
        aval = aval,
        bval = bval,
    )
}

#[test]
fn structural_equiv_assign_always_blocking_nonblocking() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..50u64 {
        let input = generate(seed.wrapping_mul(265_443_5761).wrapping_add(91));
        // X / intermediate >128 bit: hasil tak comparable antar gaya
        // (mis. reg vs net bisa beda pada X) — skip, invariant tetap jalan.
        if input.expr.eval_has_x(input.w, input.a, input.b) {
            continue;
        }
        if input.expr.max_width(input.w as u64) > 128 {
            continue;
        }
        let expr_sv = input.expr.to_sv(input.w);
        let aval = super::gen::lit_sv(input.a, input.w);
        let bval = super::gen::lit_sv(input.b, input.wb);
        let vals: Vec<Option<u64>> = (0..3)
            .map(|k| sim_signal(&variant_source(&expr_sv, input.w, &aval, &bval, k), "y"))
            .collect();
        if vals.iter().any(|v| v.is_none()) {
            // Salah satu varian gagal compile/sim — catat sebagai mismatch
            // potensial hanya jika varian lain sukses (asimetri = bug).
            if vals[0].is_some() || vals[1].is_some() || vals[2].is_some() {
                mismatch.push(format!(
                    "seed={} varian gagal sebagian: {:?}\n{}",
                    input.seed,
                    vals,
                    variant_source(&expr_sv, input.w, &aval, &bval, 0)
                ));
            }
            continue;
        }
        if vals[0] != vals[1] || vals[0] != vals[2] {
            mismatch.push(format!(
                "seed={} w={} expr=`{}` assign={:?} blocking={:?} nonblocking={:?}\n{}",
                input.seed,
                input.w,
                expr_sv,
                vals[0],
                vals[1],
                vals[2],
                variant_source(&expr_sv, input.w, &aval, &bval, 0)
            ));
        }
        checked += 1;
    }
    assert!(checked > 25, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} ketidakcocokan struktural:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
