//! Differential indexed part-select `+:` / `-:` + select out-of-range.
//!
//! Satu file = satu tanggung jawab: invarian semantik select dinamis
//! (IEEE 1800 §11.5.1). Blind spot fuzzer existing: `PartSel(hi, lo)`
//! generator selalu konstan & mayoritas in-range, dan saat out-of-range
//! emas menandai has_x → oracle melewatkan compare numerik sehingga engine
//! yang salah mengisi 0 alih-alih X tidak pernah tertangkap.
//!
//! Pola yang diuji:
//! - `a[base +: W]` / `a[base -: W]` dengan base KONSTAN dan DINAMIS (`b`)
//! - bit-select tunggal `a[idx]` dan part-select `a[hi:lo]` yang
//!   sebagian/semua bitnya di luar deklarasi
//!
//! Semantik emas (§11.5.1): bit terpilih di luar range deklarasi bernilai
//! `x`; bit dalam range diambil dari nilai variabel. Emas membandingkan
//! PER BIT (LogicVal), bukan numerik, sehingga posisi X tepat terverifikasi.

use crate::fuzz::gen::{generate, lit_sv, mask_of};
use maria_core::{LogicVal, LogicVec};

/// Jenis pola select pada satu testcase.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SelKind {
    /// `a[base +: ws]`, base konstan.
    PlusConst,
    /// `a[base +: ws]`, base = sinyal `b` (dinamis).
    PlusDyn,
    /// `a[base -: ws]`, base konstan.
    MinusConst,
    /// `a[base -: ws]`, base = sinyal `b` (dinamis).
    MinusDyn,
    /// `a[idx]` dengan idx ≥ w (out-of-range tunggal).
    BitSelOob,
    /// `a[hi:lo]` dengan hi ≥ w (out-of-range sebagian/penuh).
    PartSelOob,
}

/// Lebar hasil indexed select yang dicoba (kecil agar per-banding eksak).
const SEL_WIDTHS: [u32; 5] = [1, 2, 3, 4, 8];

/// Hitung indeks bit variabel yang dipilih oleh pola. Indeks bisa sangat
/// besar (base hingga 2^64-1) → u128 anti-overflow.
fn selected_indices(kind: SelKind, base: u64, ws: u32, oob_hi: u32, oob_lo: u32) -> Vec<u128> {
    match kind {
        SelKind::PlusConst | SelKind::PlusDyn => {
            let b = base as u128;
            (0..ws as u128).map(|i| b + i).collect()
        }
        SelKind::MinusConst | SelKind::MinusDyn => {
            let b = base as u128;
            let wsx = ws as u128;
            // a[base -: ws] memilih [base : base-ws+1]; base < ws-1 → indeks
            // "negatif" = pasti out-of-range (direpresentasikan nilai besar).
            (0..ws as u128)
                .map(|i| b.wrapping_sub(wsx - 1 - i))
                .collect()
        }
        SelKind::BitSelOob => vec![oob_hi as u128],
        SelKind::PartSelOob => (oob_lo..=oob_hi).map(|i| i as u128).collect(),
    }
}

/// Render ekspresi select ke SV sesuai pola.
fn select_sv(kind: SelKind, base_const: u32, ws: u32, oob_hi: u32, oob_lo: u32) -> String {
    match kind {
        SelKind::PlusConst => format!("a[{} +: {}]", base_const, ws),
        SelKind::PlusDyn => format!("a[b +: {}]", ws),
        SelKind::MinusConst => format!("a[{} -: {}]", base_const, ws),
        SelKind::MinusDyn => format!("a[b -: {}]", ws),
        SelKind::BitSelOob => format!("a[{}]", oob_hi),
        SelKind::PartSelOob => format!("a[{}:{}]", oob_hi, oob_lo),
    }
}

/// Nilai base efektif untuk pola dyn (nilai b sudah ter-mask ke wb).
fn dyn_base(input: &crate::fuzz::gen::GenInput) -> u64 {
    input.b
}

struct CasePlan {
    kind: SelKind,
    ws: u32,
    base_const: u32,
    oob_hi: u32,
    oob_lo: u32,
}

fn plan_case(seed: u64, w: u32, rng: &mut fastrand::Rng) -> CasePlan {
    let kind = match rng.usize(0..6) {
        0 => SelKind::PlusConst,
        1 => SelKind::PlusDyn,
        2 => SelKind::MinusConst,
        3 => SelKind::MinusDyn,
        4 => SelKind::BitSelOob,
        _ => SelKind::PartSelOob,
    };
    let ws = SEL_WIDTHS[rng.usize(0..SEL_WIDTHS.len())];
    // Base konstan: sekitar boundary w (in-range penuh, straddle, dan
    // jauh di atas) agar kedua dunia tercakup.
    let offsets: [i64; 9] = [-8, -4, -2, -1, 0, 1, 2, 4, 8];
    let off = offsets[rng.usize(0..offsets.len())];
    let base_const = (w as i64 + off).clamp(0, u32::MAX as i64 - ws as i64 - 1) as u32;
    // Out-of-range konstan: hi di [w .. w+7], lo < hi.
    let oob_hi = w + rng.u32(0..8);
    let oob_lo = if kind == SelKind::PartSelOob && rng.bool() {
        rng.u32(0..=w.min(oob_hi))
    } else {
        rng.u32(0..=oob_hi)
    };
    CasePlan {
        kind,
        ws,
        base_const,
        oob_hi,
        oob_lo,
    }
}

fn source(input: &crate::fuzz::gen::GenInput, plan: &CasePlan) -> String {
    let w = input.w;
    let expr_sv = select_sv(
        plan.kind,
        plan.base_const,
        plan.ws,
        plan.oob_hi,
        plan.oob_lo,
    );
    let yw = match plan.kind {
        SelKind::BitSelOob => 1,
        SelKind::PartSelOob => plan.oob_hi.saturating_sub(plan.oob_lo) + 1,
        _ => plan.ws,
    };
    format!(
        "module partsel_fuzz_mod;\n\
         \x20   reg [{hi}:0] a;\n\
         \x20   reg [{bhi}:0] b;\n\
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
        bhi = input.wb - 1,
        yhi = yw - 1,
        expr = expr_sv,
        aval = lit_sv(input.a, w),
        bval = lit_sv(input.b, input.wb),
    )
}

/// Emas per-bit: LogicVal sepanjang lebar hasil.
fn golden_bits(input: &crate::fuzz::gen::GenInput, plan: &CasePlan) -> (Vec<LogicVal>, u32) {
    let w = input.w as u128;
    let (idxs, yw) = match plan.kind {
        SelKind::BitSelOob => (selected_indices(plan.kind, 0, 1, plan.oob_hi, 0), 1),
        SelKind::PartSelOob => (
            selected_indices(plan.kind, 0, 0, plan.oob_hi, plan.oob_lo),
            plan.oob_hi.saturating_sub(plan.oob_lo) + 1,
        ),
        _ => {
            let base = match plan.kind {
                SelKind::PlusConst | SelKind::MinusConst => plan.base_const as u64,
                _ => dyn_base(input),
            };
            (selected_indices(plan.kind, base, plan.ws, 0, 0), plan.ws)
        }
    };
    let mut out = Vec::with_capacity(idxs.len());
    // Semantik §11.5.1 + realita Icarus (differential ground truth):
    // part-select SEBAGIAN di luar deklarasi → bit luar batas x, bit
    // dalam batas tetap nilai asli (`a[7:2]` pada reg [3:0] = xxxxxx10 →
    // xxxx10 menurut Icarus). Dulu seluruh hasil dipaksa x sehingga
    // reduction kehilangan dominasi 0/1 (ditemukan saat verifikasi fix
    // guided_fuzz seed=111666772).
    for idx in idxs {
        // `a` u64: bit ≥64 pasti 0 (stimulus ter-mask) — jangan shift.
        if idx >= w {
            out.push(LogicVal::X);
            continue;
        }
        let bit = if idx >= 64 { 0 } else { (input.a >> idx) & 1 };
        out.push(if bit == 1 {
            LogicVal::One
        } else {
            LogicVal::Zero
        });
    }
    (out, yw)
}

/// Apakah vektor memuat bit tak-diketahui (X/Z)?
fn has_unknown(v: &LogicVec) -> bool {
    v.bits
        .iter()
        .any(|b| matches!(b, LogicVal::X | LogicVal::Z))
}

fn sim_y(src: String) -> Option<LogicVec> {
    std::thread::Builder::new()
        .name("partsel-fuzz-sim".to_string())
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            crate::simulate_signals(&src, 30)
                .ok()?
                .into_iter()
                .find(|(n, _)| n == "y")
                .map(|(_, v)| v)
        })
        .expect("spawn partsel-fuzz-sim")
        .join()
        .expect("sim panic")
}

#[test]
fn partsel_indexed_and_oob_match_golden_per_bit() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    let mut in_range_cases = 0u32;
    let mut x_cases = 0u32;

    for seed in 0..160u64 {
        let input = generate(seed.wrapping_mul(6_421_151).wrapping_add(7));
        if input.w < 4 {
            continue;
        }
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAB_C0_DE);
        let plan = plan_case(seed, input.w, &mut rng);
        let (expected, yw) = golden_bits(&input, &plan);
        let src = source(&input, &plan);

        if expected.iter().any(|b| *b == LogicVal::X) {
            x_cases += 1;
        } else {
            in_range_cases += 1;
        }

        let actual = match sim_y(src.clone()) {
            Some(v) => v,
            None => {
                mismatch.push(format!("sim gagal:\n{}", src));
                continue;
            }
        };
        if actual.width as u32 != yw {
            mismatch.push(format!(
                "lebar y salah: harap {} dapat {}\n{}",
                yw, actual.width, src
            ));
            continue;
        }
        for (i, exp) in expected.iter().enumerate() {
            let got = actual.bits.get(i).copied().unwrap_or(LogicVal::X);
            if got != *exp {
                mismatch.push(format!(
                    "seed={} {:?} bit{} harap {:?} dapat {:?}\n{}",
                    seed, plan.kind, i, exp, got, src
                ));
                break;
            }
        }
        checked += 1;
    }
    assert!(checked > 60, "terlalu sedikit kasus (checked={})", checked);
    // Kedua dunia harus tercakup — guard agar test tak meluntur jadi
    // sepihak in-range saja atau X saja.
    assert!(
        in_range_cases >= 15,
        "kasus in-range kurang ({})",
        in_range_cases
    );
    assert!(x_cases >= 15, "kasus out-of-range kurang ({})", x_cases);
    assert!(
        mismatch.is_empty(),
        "{} mismatch partsel:\n{}",
        mismatch.len(),
        mismatch.join("\n---\n")
    );
}

#[test]
fn partsel_dyn_base_matches_constant_equivalent() {
    // Metamorphic: base dinamis `a[b +: ws]` dengan b diketahui HARUS sama
    // dengan `a[<nilai-b> +: ws]` base konstan (jalur evaluasi berbeda di
    // engine: ExprPartSelect vs konstanta elaborator).
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let input = generate(seed.wrapping_mul(9_113_177).wrapping_add(13));
        if input.w < 8 || input.w > 64 {
            continue;
        }
        let ws = SEL_WIDTHS[seed as usize % SEL_WIDTHS.len()];
        let bval = input.b & mask_of(input.wb);
        let dyn_src = format!(
            "module partsel_fuzz_mod;\n\
             \x20   reg [{hi}:0] a;\n\
             \x20   reg [{bhi}:0] b;\n\
             \x20   wire [{wsm}:0] y;\n\
             \x20   assign y = a[b +: {ws}];\n\
             \x20   initial begin\n\
             \x20       a = {aval};\n\
             \x20       b = {bval};\n\
             \x20       #10 $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = input.w - 1,
            bhi = input.wb - 1,
            wsm = ws - 1,
            ws = ws,
            aval = lit_sv(input.a, input.w),
            bval = lit_sv(bval, input.wb),
        );
        let const_src = dyn_src.replace(
            &format!("a[b +: {}]", ws),
            &format!("a[{} +: {}]", bval, ws),
        );
        let (yd, yc) = match (sim_y(dyn_src.clone()), sim_y(const_src.clone())) {
            (Some(d), Some(c)) => (d, c),
            _ => {
                mismatch.push(format!("sim gagal\n{}\n---\n{}", dyn_src, const_src));
                continue;
            }
        };
        if yd.to_u64() != yc.to_u64() || has_unknown(&yd) != has_unknown(&yc) {
            mismatch.push(format!(
                "seed={} dyn={} unk={} bits={:?} || const={} unk={} bits={:?}\n{}\n---\n{}",
                seed,
                yd.to_u64(),
                has_unknown(&yd),
                &yd.bits,
                yc.to_u64(),
                has_unknown(&yc),
                &yc.bits,
                dyn_src,
                const_src
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} ketidakcocokan dyn vs const:\n{}",
        mismatch.len(),
        mismatch.join("\n---\n")
    );
}
