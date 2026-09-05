//! Differential statement `case` — semantik pencocokan prioritas + wildcard
//! X/Z pada `case`/`casex`/`casez`.
//!
//! Satu file = satu tanggung jawab: invarian seleksi cabang case. Belum ada
//! fuzzer yang menyentuh jalur statement-level ini (semuanya ekspresi murni).
//!
//! Semantik emas (IEEE 1800 §12.5):
//! - `case`: pencocokan bit-exact 4-state — item dengan bit x/z TIDAK PERNAH
//!   cocok dengan selector yang diketahui (perbandingan literal x ≠ 0/1).
//! - `casex`: bit x/z pada ITEM = wildcard (don't care).
//! - `casez`: hanya bit z (= `?`) pada ITEM = wildcard; x dibandingkan
//!   literal.
//! - Prioritas: item pertama yang cocok menang; tidak ada yang cocok →
//!   `default`.
//!
//! Selector memakai generator ekspresi yang sama (`gen_node`) sehingga
//! jalur evaluasi bersarang ikut terlatih di posisi baru (operand case).

use crate::fuzz::gen::{generate, mask_of};

/// Jenis case yang diuji.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CaseKind {
    Normal,
    Casex,
    Casez,
}

impl CaseKind {
    fn keyword(self) -> &'static str {
        match self {
            CaseKind::Normal => "case",
            CaseKind::Casex => "casex",
            CaseKind::Casez => "casez",
        }
    }
}

/// Satu item case: nilai + mask bit-x + mask bit-z (selebar w).
struct CaseItem {
    val: u64,
    xm: u64,
    zm: u64,
}

/// Render literal 4-state `{w}'b...` dari val/xm/zm.
fn item_sv(it: &CaseItem, w: u32) -> String {
    let mut bits = String::with_capacity(w as usize);
    for i in (0..w).rev() {
        let b = 1u64 << i;
        if it.xm & b != 0 {
            bits.push('x');
        } else if it.zm & b != 0 {
            bits.push('z');
        } else {
            bits.push(if it.val & b != 0 { '1' } else { '0' });
        }
    }
    format!("{}'b{}", w, bits)
}

/// Apakah item cocok dengan selector (diketahui penuh) menurut semantik kind.
fn item_matches(sel: u64, it: &CaseItem, kind: CaseKind) -> bool {
    let mut b = 1u64;
    while b != 0 && b <= sel.max(it.val).max(it.xm).max(it.zm) {
        let s = sel & b;
        let wildcard = match kind {
            CaseKind::Normal => false,
            CaseKind::Casex => it.xm & b != 0 || it.zm & b != 0,
            CaseKind::Casez => it.zm & b != 0,
        };
        if !wildcard {
            // Selector selalu diketahui (seed X di-skip) — bit item x/z tak
            // pernah sama dengan bit selector yang diketahui.
            if it.xm & b != 0 || it.zm & b != 0 || (it.val & b) != s {
                return false;
            }
        }
        b <<= 1;
        if b == 0 {
            break;
        }
    }
    true
}

fn source(
    expr_sv: &str,
    w: u32,
    yw: u32,
    kind: CaseKind,
    items: &[CaseItem],
    aval: &str,
    bval: &str,
) -> String {
    let mut body = format!("        {} ({})\n", kind.keyword(), expr_sv);
    for (i, it) in items.iter().enumerate() {
        body.push_str(&format!(
            "            {}: y = {};\n",
            item_sv(it, w),
            crate::fuzz::gen::lit_sv((i + 1) as u64, yw)
        ));
    }
    body.push_str(&format!(
        "            default: y = {};\n",
        crate::fuzz::gen::lit_sv(0, yw)
    ));
    body.push_str("        endcase\n");
    body.push_str("    end\n");
    format!(
        "module case_fuzz_mod;\n\
         \x20   reg [{hi}:0] a;\n\
         \x20   reg [{hi}:0] b;\n\
         \x20   reg [{yhi}:0] y;\n\
         \x20   always @(*) begin\n\
         {body}\
         \x20   initial begin\n\
         \x20       a = {aval};\n\
         \x20       b = {bval};\n\
         \x20       #10;\n\
         \x20       $finish;\n\
         \x20   end\n\
         endmodule\n",
        hi = w - 1,
        yhi = yw - 1,
        body = body,
        aval = aval,
        bval = bval,
    )
}

/// Lebar register tag `y`: cukup menampung ID cabang 0..=n_items TANPA
/// tabrakan — dulu y selebar w sehingga pada w kecil konstanta cabang
/// saling alias (`lit_sv(2,1)` = `1'b0`) dan emas tak comparable.
fn tag_width(w: u32, n_items: usize) -> u32 {
    let mut yw = 1u32;
    while (1u128 << yw) <= n_items as u128 {
        yw += 1;
    }
    yw.max(w)
}

#[test]
fn case_statement_priority_and_wildcards_match_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..100u64 {
        let input = generate(seed.wrapping_mul(154_858_63).wrapping_add(29));
        if input.w > 64 {
            continue;
        }
        // Selector harus diketahui penuh agar model 2-state comparable.
        if input.expr.eval_has_x(input.w, input.a, input.b) {
            continue;
        }
        // Selector yang lebar self-determined-nya MELEBIHI w (concat) akan
        // ter-truncate di emas — skip (kasus tumbuh-lebar butuh model
        // mutual-extension selector vs item yang belum ada).
        if input.expr.max_width(input.w as u64) > input.w as u64 {
            continue;
        }
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xCA5E_0001);
        let kind = match rng.usize(0..3) {
            0 => CaseKind::Normal,
            1 => CaseKind::Casex,
            _ => CaseKind::Casez,
        };
        let n_items = rng.usize(2..=4);
        let mut items = Vec::with_capacity(n_items);
        let wmask = crate::fuzz::gen::mask_of(input.w);
        for _ in 0..n_items {
            // Nilai item di-mask ke lebar w — literal hanya merender w bit,
            // jadi bit di atasnya tak boleh ikut dibandingkan model emas.
            let val = rng.u64(0..) & wmask;
            // ~45% item mengandung wildcard x/z (posisi acak).
            if rng.usize(0..100) < 45 {
                let mut xm = 0u64;
                let mut zm = 0u64;
                for i in 0..input.w.min(64) {
                    let r = rng.u64(0..10);
                    if r == 0 {
                        xm |= 1u64 << i;
                    } else if r == 1 {
                        zm |= 1u64 << i;
                    }
                }
                items.push(CaseItem { val, xm, zm });
            } else {
                items.push(CaseItem { val, xm: 0, zm: 0 });
            }
        }
        // Model emas: item pertama yang cocok menang; else default (y=0).
        let sel = input.expr.eval(input.w, input.a, input.b) & mask_of(input.w);
        let expected = items
            .iter()
            .position(|it| item_matches(sel, it, kind))
            .map_or(0, |i| i + 1) as u64;
        let yw = tag_width(input.w, n_items);
        let src = source(
            &input.expr.to_sv(input.w),
            input.w,
            yw,
            kind,
            &items,
            &crate::fuzz::gen::lit_sv(input.a, input.w),
            &crate::fuzz::gen::lit_sv(input.b, input.wb),
        );
        let actual = std::thread::Builder::new()
            .name("case-fuzz-sim".to_string())
            .stack_size(256 * 1024 * 1024)
            .spawn({
                let src = src.clone();
                move || {
                    crate::simulate_signals(&src, 30).ok().and_then(|sigs| {
                        sigs.iter()
                            .find(|(n, _)| *n == "y")
                            .map(|(_, v)| v.to_u64())
                    })
                }
            })
            .expect("spawn case-fuzz-sim")
            .join()
            .expect("sim panic");
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} {:?} sel={:#x} exp={} act={:?}\n{}",
                input.seed, kind, sel, expected, actual, src
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch case:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn case_plain_equivalent_to_if_else_chain() {
    // Metamorphic struktural: `case` biasa (item tanpa wildcard, selector
    // diketahui) ≡ rantai `if/else if` dengan `===`. Dua jalur penurunan
    // berbeda di elaborator — divergensi = inkonsistensi pipeline.
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..60u64 {
        let input = generate(seed.wrapping_mul(2_654_353).wrapping_add(41));
        if input.w > 64 {
            continue;
        }
        if input.expr.eval_has_x(input.w, input.a, input.b) {
            continue;
        }
        if input.expr.max_width(input.w as u64) > input.w as u64 {
            continue;
        }
        let mut rng = fastrand::Rng::with_seed(seed ^ 0x1F_E1_5E);
        let n_items = rng.usize(2..=4);
        let wmask = mask_of(input.w);
        let items: Vec<CaseItem> = (0..n_items)
            .map(|_| CaseItem {
                // Di-mask ke w — literal hanya merender w bit.
                val: rng.u64(0..) & wmask,
                xm: 0,
                zm: 0,
            })
            .collect();
        let expr_sv = input.expr.to_sv(input.w);
        let aval = crate::fuzz::gen::lit_sv(input.a, input.w);
        let bval = crate::fuzz::gen::lit_sv(input.b, input.w);
        let sel = input.expr.eval(input.w, input.a, input.b) & mask_of(input.w);
        let expected = items
            .iter()
            .position(|it| item_matches(sel, it, CaseKind::Normal))
            .map_or(0, |i| i + 1) as u64;

        let yw = tag_width(input.w, n_items);
        let mut case_body = format!("        case ({})\n", expr_sv);
        for (i, it) in items.iter().enumerate() {
            case_body.push_str(&format!(
                "            {}: y = {};\n",
                item_sv(it, input.w),
                crate::fuzz::gen::lit_sv((i + 1) as u64, yw)
            ));
        }
        case_body.push_str(&format!(
            "            default: y = {};\n        endcase\n",
            crate::fuzz::gen::lit_sv(0, yw)
        ));

        let mut if_body = String::new();
        for (i, it) in items.iter().enumerate() {
            // Rantai: tiap cabang TIDAK menutup `end` sendiri — penutup jadi
            // prefiks cabang berikutnya (`end else if`), terakhir ditutup
            // oleh `end else`.
            let head = if i == 0 {
                "        if"
            } else {
                "        end else if"
            };
            if_body.push_str(&format!(
                "{} ((({}) === ({}))) begin\n            y = {};\n",
                head,
                expr_sv,
                item_sv(it, input.w),
                crate::fuzz::gen::lit_sv((i + 1) as u64, yw)
            ));
        }
        if_body.push_str(&format!(
            "        end else begin\n            y = {};\n        end\n",
            crate::fuzz::gen::lit_sv(0, yw)
        ));

        for variant in [case_body, if_body] {
            let src = format!(
                "module case_fuzz_mod;\n\
                 \x20   reg [{hi}:0] a;\n\
                 \x20   reg [{hi}:0] b;\n\
                 \x20   reg [{yhi}:0] y;\n\
                 \x20   always @(*) begin\n\
                 {variant}\
                 \x20   end\n\
                 \x20   initial begin\n\
                 \x20       a = {aval};\n\
                 \x20       b = {bval};\n\
                 \x20       #10;\n\
                 \x20       $finish;\n\
                 \x20   end\n\
                 endmodule\n",
                hi = input.w - 1,
                yhi = yw - 1,
                variant = variant,
                aval = aval,
                bval = bval,
            );
            let actual = std::thread::Builder::new()
                .name("case-equiv-sim".to_string())
                .stack_size(256 * 1024 * 1024)
                .spawn({
                    let src = src.clone();
                    move || {
                        crate::simulate_signals(&src, 30).ok().and_then(|sigs| {
                            sigs.iter()
                                .find(|(n, _)| *n == "y")
                                .map(|(_, v)| v.to_u64())
                        })
                    }
                })
                .expect("spawn case-equiv-sim")
                .join()
                .expect("sim panic");
            if actual != Some(expected) {
                mismatch.push(format!(
                    "seed={} exp={} act={:?}\n{}",
                    input.seed, expected, actual, src
                ));
            }
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} ketidakcocokan case vs if-else:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
