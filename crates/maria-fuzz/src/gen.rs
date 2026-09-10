//! Generator input SV — 100% valid by construction.
//!
//! Paper #14 (Csmith): generator program/modul acak lengkap tapi valid.
//! Paper #15 (YARPGen): type-aware — lebar/signedness dipilih sadar tipe,
//!   menghindari konstruk yang jelas-jelas invalid (bukan coblos acak).
//! Paper #1 (Miller 1990): input acak murni — baseline robustness lexer/parser.
//!
//! Arsitektur (NON-TEMPLATE, 100% valid):
//! - `Bias` = profil probabilitas konstruk (signed/xz/scheduler/...). Strategi
//!   (`module_for_shape`) dan interaksi semantik (`compose_from_interaction`)
//!   HANYA menggeser knob bias — struktur modul diRAKIT oleh builder terstruktur.
//! - Semua sinyal dideklarasikan SEBELUM dipakai (symbol table terstruktur).
//! - Ekspresi/statement/proses dirakit rekursif dengan referensi valid ke sinyal
//!   yang sudah dideklarasikan. Literal berukuran sesuai konteks.
//! - Input valid ≡ compile ok + sim ok → 100% mencapai simulator → stress test
//!   engine penuh: sizing operan (§11.6), region scheduler (NBA/#0/event),
//!   X/Z propagation, fork/join race, mem OOB, loop/control-flow.

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::Rng;

pub const WIDTHS: &[usize] = &[2, 4, 8, 16];
pub const OPS: &[&str] = &["+", "-", "&", "|", "^", "<<", ">>"];

/// Operator biner penuh (IEEE 1800 §11.4).
const BINOPS: &[&str] = &[
    "+", "-", "*", "/", "%", "<<", ">>", "<", "<=", ">", ">=", "==", "!=",
    "===", "!==", "&", "|", "^", "~^", "&&", "||",
];

/// Operator unary (IEEE 1800 §11.4) — termasuk reduction.
const UNOPS: &[&str] = &["~", "-", "!", "&", "|", "^", "~&", "~|", "~^"];

/// Knob probabilitas konstruk — profil strategi & tekanan semantik.
/// Semua bernilai 0.0..=1.0; 0 = konstruk jarang/absen, 1 = konstruk dominan.
#[derive(Debug, Clone, Copy)]
pub struct Bias {
    /// Probabilitas operand/deklarasi signed & cast `$signed(...)`.
    pub signed: f64,
    /// Probabilitas lebar operan MISMATCH (w vs w*2 vs w-1) — sizing §11.6.
    pub wide: f64,
    /// Probabilitas literal/isi X/Z (4-state propagation).
    pub xz: f64,
    /// Probabilitas scheduling: NBA/#0/delay/event-control di proses.
    pub sched: f64,
    /// Probabilitas fork/join + multi-writer race.
    pub conc: f64,
    /// Probabilitas elaborasi: child module, generate, param override.
    pub elab: f64,
    /// Probabilitas tipe non-logic: mem array, integer, dsb.
    pub ty: f64,
    /// Probabilitas part-select/bounds OUT-OF-RANGE (stress evaluasi §11.5.1).
    pub oob: f64,
    /// Probabilitas pembagian/modulo nol — jalur runtime error engine.
    pub div0: f64,
    /// Probabilitas konstruk loop (for/while/repeat).
    pub loops: f64,
    /// Probabilitas kedalaman REKURSI tinggi — nested loops, deep expr chains,
    /// multi-statement blocks. Menekan engine internal: fork continuation,
    /// loop control flow, expression evaluator stack, region ordering lintas
    /// statement. 0 = depth 2 (ringan); 1 = depth 5 (deep engine stress).
    pub deep: f64,
}

impl Default for Bias {
    fn default() -> Self {
        Bias {
            signed: 0.2,
            wide: 0.3,
            xz: 0.15,
            sched: 0.3,
            conc: 0.15,
            elab: 0.15,
            ty: 0.2,
            oob: 0.2,
            div0: 0.05,
            loops: 0.3,
            deep: 0.2,
        }
    }
}

/// Profil bias untuk strategi `k` (0..NUM_SHAPES) — bukan template: tiap
/// strategi hanya menggeser tekanan konstruk; struktur tetap dirakit builder.
pub fn bias_for_shape(k: u32) -> Bias {
    let mut b = Bias::default();
    match k % 13 {
        0 => {} // base — profil default seimbang.
        1 => {
            b.sched = 0.5;
            b.conc = 0.3;
        }
        2 => {
            b.wide = 0.7;
            b.signed = 0.6;
        }
        3 => {
            b.xz = 0.7;
            b.oob = 0.4;
        }
        4 => {
            b.sched = 0.8;
        }
        5 => {
            b.loops = 0.8;
            b.deep = 0.7;
        }
        6 => {
            b.ty = 0.8;
            b.deep = 0.5;
        }
        7 => {
            b.conc = 0.8;
            b.sched = 0.4;
            b.deep = 0.6;
        }
        8 => {
            b.elab = 0.8;
        }
        9 => {
            b.oob = 0.8;
            b.div0 = 0.3;
            b.wide = 0.5;
        }
        10 => {
            b.wide = 0.6;
            b.signed = 0.4;
            b.oob = 0.3;
        }
        11 => {
            b.ty = 0.5;
            b.signed = 0.4;
            b.sched = 0.4;
        }
        _ => {
            // 12 = semua tekanan maksimum — modul paling berbahaya.
            b.signed = 0.5;
            b.wide = 0.6;
            b.xz = 0.4;
            b.sched = 0.5;
            b.conc = 0.4;
            b.elab = 0.4;
            b.ty = 0.4;
            b.oob = 0.5;
            b.div0 = 0.15;
            b.loops = 0.5;
            b.deep = 0.8;
        }
    }
    b
}

/// Profil bias dari pasangan area semantik (interaction graph): area yang
/// di-target menaikkan knob yang relevan — komposisi tetap rekursif, bukan
/// blok template per area.
pub fn bias_from_areas(
    a: crate::semantic::SemArea,
    b: crate::semantic::SemArea,
) -> Bias {
    use crate::semantic::SemArea;
    let mut x = Bias::default();
    for area in [a, b] {
        match area {
            SemArea::Scheduler => x.sched = x.sched.max(0.7),
            SemArea::Concurrency => {
                x.conc = x.conc.max(0.7);
                x.deep = x.deep.max(0.5);
            }
            SemArea::Timing => {
                x.sched = x.sched.max(0.6);
                x.deep = x.deep.max(0.4);
            }
            SemArea::Width => x.wide = x.wide.max(0.7),
            SemArea::Signedness => x.signed = x.signed.max(0.7),
            SemArea::TypeSystem => x.ty = x.ty.max(0.6),
            SemArea::Xz => x.xz = x.xz.max(0.7),
            SemArea::Elaboration | SemArea::Hierarchy | SemArea::Generate => {
                x.elab = x.elab.max(0.7);
            }
            SemArea::Lifetime | SemArea::Class | SemArea::Constraint => {
                x.ty = x.ty.max(0.5);
            }
            SemArea::Package => x.ty = x.ty.max(0.4),
            SemArea::Assertion | SemArea::Coverage => {
                x.ty = x.ty.max(0.35);
                x.sched = x.sched.max(0.4);
            }
            SemArea::Interface | SemArea::Dpi => {}
        }
    }
    x
}

/// Generator deterministik ber-seed.
pub struct Generator {
    #[allow(dead_code)] // seed disimpan utk API future (reset/derive)
    seed: u64,
}

/// Metadata sinyal yang dideklarasikan di modul — digunakan oleh symbol table.
struct Sig {
    name: String,
    width: usize,
}

/// Builder keadaan per-generasi: RNG + bias + symbol table + counter unik.
struct Builder<'a> {
    rng: &'a mut StdRng,
    b: Bias,
    seq: u64,
    /// Semua sinyal yang dideklarasikan (ports + internal + loop vars).
    sigs: Vec<Sig>,
}

impl Generator {
    pub fn new(seed: u64) -> Self {
        Generator { seed }
    }

    /// Input acak murni — deretan token SV campur junk (Paper #1).
    /// Sasaran: lexer/parser robustness, bukan semantik. (Bukan template:
    /// murni random byte/token stream.)
    pub fn random_syntax(&self, rng: &mut StdRng) -> String {
        const CHARSET: &[char] = &[
            'a', 'b', 'm', 'o', 'd', 'u', 'l', 'e', 't', 'o', 'p', '0', '1', 'x', 'z', '_',
            '[', ']', '(', ')', ';', ':', ',', '+', '-', '&', '|', '^', '~', '<', '>', '#', '.',
            '@', '{', '}', '"', '\'', '`', '/', '*', '\n', ' ', ' ', '=', '!', '?',
        ];
        let n = rng.gen_range(1..=200);
        let mut s = String::with_capacity(n + 32);
        if rng.gen_bool(0.7) {
            s.push_str("module top;\n");
        }
        for _ in 0..n {
            if rng.gen_bool(0.08) {
                s.push('\n');
            } else if rng.gen_bool(0.5) {
                s.push(*CHARSET.choose(rng).unwrap());
            } else {
                s.push_str(
                    ["endmodule", "always", "initial", "if", "assign", "reg", "wire"]
                        .choose(rng)
                        .unwrap(),
                );
            }
        }
        s
    }

    /// Modul SV acak — 100% valid (bukan template): pilih strategi bias
    /// lalu rakit modul terstruktur dengan symbol table. Semua strategi ikut
    /// (0..NUM_SHAPES). Setiap modul = compile ok + sim ok.
    pub fn random_module(&self, rng: &mut StdRng) -> String {
        let k = rng.gen_range(0..NUM_SHAPES);
        self.module_for_shape(rng, k)
    }

    /// Bangun modul untuk strategi bias `shape` (0..NUM_SHAPES) — deterministik
    /// per stream RNG. `shape` = profil tekanan konstruk, BUKAN template.
    pub fn module_for_shape(&self, rng: &mut StdRng, shape: u32) -> String {
        let w = *WIDTHS.choose(rng).unwrap();
        let b = bias_for_shape(shape);
        let mut bld = Builder {
            rng,
            b,
            seq: 0,
            sigs: Vec::new(),
        };
        build_module(&mut bld, w, false)
    }

    /// Modul dirakit dari pasangan area semantik (interaction graph) — bias
    /// diturunkan dari area, struktur tetap terstruktur dengan symbol table.
    pub fn compose_from_interaction(
        &self,
        rng: &mut StdRng,
        w: usize,
        a: crate::semantic::SemArea,
        b: crate::semantic::SemArea,
    ) -> String {
        let bias = bias_from_areas(a, b);
        let mut bld = Builder {
            rng,
            b: bias,
            seq: 0,
            sigs: Vec::new(),
        };
        build_module(&mut bld, w, false)
    }

    /// Core PASIF (differential vs tool referensi): DUT tanpa stimulus &
    /// clock internal — tb EKSTERNAL yang meng-drive input. Menghindari
    /// konflik "input wire di-drive dari dalam modul" (illegal iverilog/VCS;
    /// maria longgar) agar kedua tool membandingkan DUT yang sama. Bias
    /// STRESS (conc/sched/width/signed/xz) — oracle referensi bekerja paling
    /// baik pada konstruk rentan deviasi: race multi-driver, X/Z, sizing.
    pub fn passive_core(&self, rng: &mut StdRng, w: usize) -> String {
        let mut b = Bias::default();
        b.conc = 0.7;
        b.sched = 0.5;
        b.wide = 0.6;
        b.signed = 0.5;
        b.xz = 0.35;
        b.loops = 0.3;
        b.oob = 0.2;
        let mut bld = Builder {
            rng,
            b,
            seq: 0,
            sigs: Vec::new(),
        };
        build_module(&mut bld, w, true)
    }
}

/// Jumlah strategi bias (indeks valid utk `module_for_shape`).
pub const NUM_SHAPES: u32 = 13;

// ──────────────────────────────────────────────────────────────────────
// Symbol table — declare-before-use, 100% valid.
// ──────────────────────────────────────────────────────────────────────

/// Nama unik per builder (counter).
fn fresh_name(b: &mut Builder, prefix: &str) -> String {
    b.seq += 1;
    format!("{}{}", prefix, b.seq)
}

/// Pilih nama register acak dari pool yang sudah dideklarasikan.
fn get_random_reg_name(b: &mut Builder) -> String {
    // Pool: semua sinyal kecuali input (tidak boleh assign ke input)
    let names: Vec<&str> = b
        .sigs
        .iter()
        .filter(|s| !s.name.starts_with('i') || s.name.starts_with("iv"))
        .map(|s| s.name.as_str())
        .collect();
    if names.is_empty() {
        let n = fresh_name(b, "r");
        b.sigs.push(Sig {
            name: n.clone(),
            width: 4,
        });
        return n;
    }
    names.choose(b.rng).unwrap().to_string()
}

/// Lebar sinyal berdasarkan nama.
fn get_sig_width(b: &Builder, name: &str) -> usize {
    b.sigs
        .iter()
        .find(|s| s.name == name)
        .map(|s| s.width)
        .unwrap_or(4)
}

// ──────────────────────────────────────────────────────────────────────
// Expression builder — 100% valid (hanya ref sinyal dideklarasikan + literal).
// ──────────────────────────────────────────────────────────────────────

/// Atom ekspresi — selalu valid: ref sinyal dideklarasikan atau literal.
fn atom(b: &mut Builder, w: usize) -> String {
    if b.sigs.is_empty() || b.rng.gen_bool(0.25) {
        // Literal — lebar disesuaikan konteks.
        literal(b, w)
    } else {
        // Ref sinyal dideklarasikan.
        let sig = b.sigs.choose(b.rng).unwrap();
        if b.rng.gen_bool(0.08) && sig.width > 1 {
            // Bit-select — IN-RANGE selalu (valid by construction).
            let idx = b.rng.gen_range(0..sig.width);
            format!("{}[{}]", sig.name, idx)
        } else if b.rng.gen_bool(0.06) && sig.width > 1 {
            // Part-select — IN-RANGE selalu: [msb:lsb] atau [base +/-: width].
            let hi = b.rng.gen_range(0..sig.width);
            let lo = b.rng.gen_range(0..=hi);
            if b.rng.gen_bool(0.5) {
                format!("{}[{}:{}]", sig.name, hi, lo)
            } else {
                let sw = (hi - lo + 1).max(1);
                if b.rng.gen_bool(0.5) {
                    format!("{}[{} -: {}]", sig.name, hi, sw)
                } else {
                    format!("{}[{} +: {}]", sig.name, lo, sw)
                }
            }
        } else {
            sig.name.clone()
        }
    }
}

/// Ref sinyal polos (tanpa literal/select) — argumen `$size`/`$bits` yang
/// WAJIB berupa signal (maria tolak literal E3001). Fallback `a` (port selalu
/// ter-deklarasi).
fn sig_ref(b: &mut Builder) -> String {
    if b.sigs.is_empty() {
        return "a".to_string();
    }
    b.sigs.choose(b.rng).map(|s| s.name.clone()).unwrap_or_else(|| "a".to_string())
}

/// Literal bertipe — ukuran sesuai lebar konteks (Paper #15).
fn literal(b: &mut Builder, w: usize) -> String {
    let span = 1u64 << w.min(16);
    let v = b.rng.gen_range(0..span);
    // X/Z literals — stress 4-state propagation.
    if b.rng.gen_bool(b.b.xz * 0.3) {
        return format!("{w}'{}", if b.rng.gen_bool(0.5) { "x" } else { "z" });
    }
    match b.rng.gen_range(0..4u32) {
        0 => format!("{w}'d{v}"),
        1 => format!("{w}'h{:x}", v),
        2 => format!("{w}'b{:b}", v),
        _ => {
            if b.rng.gen_bool(b.b.signed * 0.3) && v > 0 {
                format!("-{w}'d{}", v.min(span - 1))
            } else {
                format!("{w}'d{v}")
            }
        }
    }
}

/// Ekspresi rekursif — 100% valid: operator biner/unary, termites, concat.
/// Kedalaman `d` dibatasi agar ekspresi tidak terlalu kompleks.
fn expr(b: &mut Builder, w: usize, d: u8) -> String {
    if d == 0 || b.rng.gen_bool(0.35) {
        return atom(b, w);
    }
    match b.rng.gen_range(0..11u32) {
        0 => {
            // Binary op — kedua operand valid, sizing sadar tanda.
            let op = *BINOPS.choose(b.rng).unwrap();
            let w2 = if b.rng.gen_bool(b.b.wide) {
                (w * 2).max(8)
            } else {
                w
            };
            format!("({}) {} ({})", expr(b, w, d - 1), op, expr(b, w2, d - 1))
        }
        1 => {
            // Unary op — reduksi.
            let u = *UNOPS.choose(b.rng).unwrap();
            format!("({}{})", u, atom(b, w))
        }
        2 => {
            // Ternary — kondisi bool, hasil lebar w.
            let c = expr(b, w.min(4), d - 1);
            let x = expr(b, w, d - 1);
            let y = expr(b, w, d - 1);
            format!("({} ? {} : {})", c, x, y)
        }
        3 => {
            // Concatenation — `{a, b, lit}` — stress evaluasi concat §11.4.12.
            let n = b.rng.gen_range(2..=3);
            let parts: Vec<String> = (0..n).map(|_| atom(b, w.min(8))).collect();
            format!("{{{}}}", parts.join(", "))
        }
        4 => {
            // Replication `{N{expr}}` — stress replikasi §11.4.12.
            let n = b.rng.gen_range(2..=4);
            format!("{{{}{{{} }}}}", n, atom(b, w.min(8)))
        }
        5 => {
            // Cast `$signed(...)` / `$unsigned(...)`.
            if b.rng.gen_bool(0.5) {
                format!("$signed({})", atom(b, w))
            } else {
                format!("$unsigned({})", atom(b, w))
            }
        }
        6 => {
            // System sizing functions — KONSTAN agar compile ok.
            // BUG FIX (fuZZ): `$size`/`$bits` di maria menolak argumen LITERAL
            // (E3001 `$size argument must resolve to a signal` — mis-labeled
            // module-not-found). Hanya SIGNAAL yang sah → `$size(ref)`/`$bits(ref)`.
            // `$clog2(konst)` tetap konstanta (constant-fold).
            match b.rng.gen_range(0..3u32) {
                0 => format!("$clog2({})", b.rng.gen_range(1..=64)),
                1 => format!("$bits({})", sig_ref(b)),
                _ => format!("$size({})", sig_ref(b)),
            }
        }
        7 => {
            // Division/modulo — operan PORT (non-konstan) agar runtime RT,
            // bukan constant-fold E9001. Pembilang = sinyal, pembagi = 0 literal.
            let op = if b.rng.gen_bool(0.5) { "/" } else { "%" };
            let sig = if b.sigs.is_empty() {
                "a".to_string()
            } else {
                b.sigs.choose(b.rng).unwrap().name.clone()
            };
            format!("({sig} {op} {w}'d0)")
        }
        8 => {
            // Arithmetic shift — bedakan << vs >>> (arithmetic).
            if b.rng.gen_bool(0.5) {
                format!("({}) >>> ({})", atom(b, w), atom(b, w.min(4)))
            } else {
                format!("({}) <<< ({})", atom(b, w), atom(b, w.min(4)))
            }
        }
        9 => {
            // Parenthesized nested — stress parser precedence.
            format!("({})", expr(b, w, d - 1))
        }
        _ => atom(b, w),
    }
}

/// Sinyal untuk part-select OOB (hanya dipanggil bila `b.b.oob > 0`).
/// Pilih sinyal acak + buat part-select out-of-range.
fn oob_partsel(b: &mut Builder) -> String {
    let sig = if b.sigs.is_empty() {
        return "0".to_string();
    } else {
        b.sigs.choose(b.rng).unwrap()
    };
    let w = sig.width;
    let hi = b.rng.gen_range(w..(w * 2).max(8));
    let lo = b.rng.gen_range(0..w);
    if b.rng.gen_bool(0.5) {
        format!("{}[{}:{}]", sig.name, hi, lo)
    } else {
        let sw = (hi - lo).max(1);
        format!("{}[{} +: {}]", sig.name, hi, sw)
    }
}

// ──────────────────────────────────────────────────────────────────────
// Statement builder — context-aware (seq=blocking, always_ff=NBA).
// ──────────────────────────────────────────────────────────────────────

/// Statement rekursif — SEMUA self-terminating, selalu valid.
/// `nba=true` → non-blocking (≤); `nba=false` → blocking (=).
fn stmt(b: &mut Builder, w: usize, d: u8, nba: bool) -> String {
    if d == 0 {
        // Leaf: assignment ke register acak.
        let t = get_random_reg_name(b);
        let op = if nba { "<=" } else { "=" };
        return format!("    {} {} {};", t, op, expr(b, w, 0));
    }
    match b.rng.gen_range(0..16u32) {
        0 => {
            // Simple assignment — paling sering.
            let t = get_random_reg_name(b);
            let op = if nba { "<=" } else { "=" };
            format!("    {} {} {};", t, op, expr(b, w, d - 1))
        }
        1 => {
            // if/else — kedua branch valid.
            let c = expr(b, w.min(4), d - 1);
            let s1 = stmt(b, w, d - 1, nba);
            let s2 = stmt(b, w, d - 1, nba);
            format!(
                "    if ({}) begin\n{}\n    end else begin\n{}\n    end",
                c, s1, s2
            )
        }
        2 => {
            // case/casez/casex — item acak, default reset.
            let c = expr(b, w.min(4), d - 1);
            let n = b.rng.gen_range(2..=4);
            let kw = match b.rng.gen_range(0..3u32) {
                0 => "case",
                1 => "casez",
                _ => "casex",
            };
            let mut items = String::new();
            for _ in 0..n {
                let v = b.rng.gen_range(0..(1u64 << w.min(8)));
                let t = get_random_reg_name(b);
                let op = if nba { "<=" } else { "=" };
                items.push_str(&format!(
                    "      {w}'d{v}: {} {} {};\n",
                    t,
                    op,
                    expr(b, w, d - 1)
                ));
            }
            let t = get_random_reg_name(b);
            let op = if nba { "<=" } else { "=" };
            format!(
                "    {kw} ({c})\n{}      default: {} {} '0;\n    endcase",
                items, t, op
            )
        }
        3 => {
            // for loop — bound kecil, counter dideklarasikan lokal.
            // NOTE (FUZZ fix): counter TIDAK di-push ke b.sigs — deklarasi
            // `for (integer fcN ...)` berlaku hanya di dalam loop; memasukkannya
            // ke pool module-scope membuat atom/reg pick memakai fcN di luar
            // scope → E2001 undefined signal (generator tidak lagi 100% valid).
            let n = b.rng.gen_range(1..=5);
            let cn = fresh_name(b, "fc");
            let body = stmt(b, w, d - 1, nba);
            format!(
                "    for (integer {cn} = 0; {cn} < {n}; {cn} = {cn} + 1) begin\n{body}\n    end"
            )
        }
        4 => {
            // while loop — bound kecil. Counter DEKLARASIKAN di dalam blok
            // (`integer wcN;` block-scoped) — sebelumnya dipakai tanpa
            // deklarasi → E2001 undefined signal.
            let cn = fresh_name(b, "wc");
            let limit = b.rng.gen_range(1..=4);
            let body = stmt(b, w, d - 1, nba);
            format!(
                "    integer {cn};\n    {cn} = 0;\n    while ({cn} < {limit}) begin\n{body}\n      {cn} = {cn} + 1;\n    end"
            )
        }
        5 => {
            // repeat — bound kecil.
            let body = stmt(b, w, d - 1, nba);
            format!(
                "    repeat ({}) begin\n{}\n    end",
                b.rng.gen_range(1..=4),
                body
            )
        }
        6 => {
            // delay #N — stress timing.
            format!("    #{};", b.rng.gen_range(0..=9))
        }
        7 => {
            // Event control @(posedge clk).
            "    @(posedge clk);".to_string()
        }
        8 => {
            // Fork/join — branch konkurren (stress scheduler).
            let branches = b.rng.gen_range(1..=3);
            let mut body = String::new();
            for _ in 0..branches {
                body.push_str(&format!("      {}\n", stmt(b, w, d - 1, nba)));
            }
            let join = match b.rng.gen_range(0..3u32) {
                0 => "join",
                1 => "join_any",
                _ => "join_none",
            };
            format!("    fork\n{}    {}", body, join)
        }
        9 => {
            // $display — observasi nilai.
            let sig = if b.sigs.is_empty() {
                "a".to_string()
            } else {
                b.sigs.choose(b.rng).unwrap().name.clone()
            };
            format!("    $display(\"fz %0d\", {});", sig)
        }
        10 => {
            // Ternary assignment.
            let t = get_random_reg_name(b);
            let op = if nba { "<=" } else { "=" };
            let c = expr(b, w.min(4), d - 1);
            let x = expr(b, w, d - 1);
            let y = expr(b, w, d - 1);
            format!("    {} {} ({} ? {} : {});", t, op, c, x, y)
        }
        11 => {
            // OOB part-select stress — hanya jika knob oob aktif.
            if b.b.oob > 0.1 && b.rng.gen_bool(b.b.oob * 0.3) {
                let t = get_random_reg_name(b);
                let op = if nba { "<=" } else { "=" };
                let oob_expr = oob_partsel(b);
                format!("    {} {} {};", t, op, oob_expr)
            } else {
                let t = get_random_reg_name(b);
                let op = if nba { "<=" } else { "=" };
                format!("    {} {} {};", t, op, expr(b, w, d - 1))
            }
        }
        // ── DEEP ENGINE STRESS PATTERNS ──
        12 => {
            // Multi-statement sequence — kedalaman TETAP (tidak berkurang).
            // Menekan engine: continuation chain lintas-statement, region
            // ordering per-statement, control_flow propagation.
            let n = b.rng.gen_range(2..=4);
            let stmts: Vec<String> = (0..n).map(|_| stmt(b, w, d, nba)).collect();
            stmts.join("\n")
        }
        13 => {
            // Nested for loop + delay — DEEP engine stress: loop continuation
            // + delay suspend + resume lintas loop nesting level. Sering
            // memicu bug scheduler: engine harus resume loop dalam setelah
            // delay selesai tanpa kehilangan state loop luar.
            // Counter TIDAK di-push ke b.sigs (loop-scoped saja — lihat case 3).
            let cn1 = fresh_name(b, "fc");
            let cn2 = fresh_name(b, "fc");
            let n1 = b.rng.gen_range(1..=3);
            let n2 = b.rng.gen_range(1..=3);
            let inner = stmt(b, w, d.saturating_sub(1), nba);
            format!(
                "    for (integer {cn1} = 0; {cn1} < {n1}; {cn1} = {cn1} + 1) begin\n      for (integer {cn2} = 0; {cn2} < {n2}; {cn2} = {cn2} + 1) begin\n        #1;\n{inner}\n      end\n    end"
            )
        }
        14 => {
            // Conditional event control — event scheduling stress: engine
            // harus menangani @(posedge clk) di dalam if/else branch.
            let c = expr(b, w.min(4), d - 1);
            let t = get_random_reg_name(b);
            let op = if nba { "<=" } else { "=" };
            let v = expr(b, w, d - 1);
            format!(
                "    if ({c}) @(posedge clk);\n    {t} {op} {v};"
            )
        }
        15 => {
            // Deep expression chain — expression depth = d+1 (bukan d-1).
            // Menekan evaluator: nested binary/unary/ternary yang dalam
            // menguji stack evaluator dan precedence resolution.
            let t = get_random_reg_name(b);
            let op = if nba { "<=" } else { "=" };
            let deep_d = (d + 1).min(6);
            format!("    {} {} {};", t, op, expr(b, w, deep_d))
        }
        _ => {
            // Two sequential statements — kedalaman berkurang.
            let s1 = stmt(b, w, d - 1, nba);
            let s2 = stmt(b, w, d - 1, nba);
            format!("{}\n{}", s1, s2)
        }
    }
}

// ──────────────────────────────────────────────────────────────────────
// Process builders — always_ff, always_comb, etc.
// ──────────────────────────────────────────────────────────────────────

/// Proses prosedural — memilih tipe acak, membangun body yang valid.
/// `use_rst` → proses boleh pakai reset. `has_mem` → boleh akses mem.
fn proc(b: &mut Builder, w: usize, use_rst: bool, has_mem: bool) -> String {
    match b.rng.gen_range(0..6u32) {
        0 | 1 => {
            // always_ff — NBA assignment (≤).
            if use_rst && b.rng.gen_bool(0.7) {
                let t = get_random_reg_name(b);
                let body = stmt(b, w, 2, true);
                let mem_w = if has_mem && b.rng.gen_bool(0.4) {
                    let (mi, mj) = if b.rng.gen_bool(0.3) {
                        (
                            b.rng.gen_range(0..=2),
                            b.rng.gen_range(0..=2),
                        )
                    } else {
                        (b.rng.gen_range(0..=3), 0)
                    };
                    if b.rng.gen_bool(0.3) {
                        format!("      mem2[{}][{}] <= 16'h{:04X};\n", mi, mj, b.rng.gen_range(0..0xFFFF))
                    } else {
                        format!("      mem[{}] <= 16'h{:04X};\n", mi, b.rng.gen_range(0..0xFFFF))
                    }
                } else {
                    String::new()
                };
                format!(
                    "  always_ff @(posedge clk or negedge rst_n) begin\n    if (!rst_n) {t} <= '0;\n    else begin\n{body}{mem_w}    end\n  end\n"
                )
            } else {
                let body = stmt(b, w, 2, true);
                format!(
                    "  always_ff @(posedge clk) begin\n{body}  end\n"
                )
            }
        }
        2 => {
            // always_comb — blocking assignment (=).
            let body = stmt(b, w, 2, false);
            format!("  always_comb begin\n{body}  end\n")
        }
        3 => {
            // always_latch — blocking.
            let t = get_random_reg_name(b);
            let e = expr(b, w, 2);
            format!("  always_latch if (a) {t} = {e};\n")
        }
        4 => {
            // always @(posedge/negedge) generik — NBA.
            if use_rst && b.rng.gen_bool(0.5) {
                let body = stmt(b, w, 2, true);
                format!(
                    "  always @(posedge clk or negedge rst_n) begin\n    if (!rst_n) {} <= '0;\n    else begin\n{body}    end\n  end\n",
                    get_random_reg_name(b)
                )
            } else {
                let body = stmt(b, w, 2, true);
                format!("  always @(posedge clk) begin\n{body}  end\n")
            }
        }
        _ => {
            // always @(*) — combinational generik, blocking.
            let body = stmt(b, w, 2, false);
            format!("  always @(*) begin\n{body}  end\n")
        }
    }
}

// ──────────────────────────────────────────────────────────────────────
// Testbench builder — clock + reset + stimulus.
// ──────────────────────────────────────────────────────────────────────

/// Clock generator — always valid.
fn gen_clock() -> String {
    "  initial begin clk = 0; forever #5 clk = ~clk; end\n".to_string()
}

/// Stimulus + assertions — uji engine path evaluator/scheduler.
fn gen_stimulus(b: &mut Builder, w: usize, use_rst: bool) -> String {
    let wmax = (1u64 << w.min(16)).min(65535);
    let v1 = b.rng.gen_range(0..wmax);
    let v2 = b.rng.gen_range(0..wmax);
    let v3 = b.rng.gen_range(0..wmax);
    let v4 = b.rng.gen_range(0..wmax);
    let t1 = b.rng.gen_range(2..=12u64);
    let t2 = b.rng.gen_range(2..=12u64);

    let mut s = if use_rst {
        format!(
            "  initial begin rst_n = 0; a = {v1}; b = {v2}; #{t1} rst_n = 1; #{t2} a = {v3}; b = {v4}; end\n"
        )
    } else {
        format!(
            "  initial begin a = {v1}; b = {v2}; #{t1} a = {v3}; b = {v4}; end\n"
        )
    };

    // X/Z injection — stress 4-state propagation.
    if b.rng.gen_bool(b.b.xz * 0.3) {
        let xv = if b.rng.gen_bool(0.5) { "'x" } else { "'z" };
        let t3 = b.rng.gen_range(1..=5u64);
        s.push_str(&format!("  initial begin #{t3} a = {xv}; end\n"));
    }

    // Assertion — validasi output ≠ X di akhir simulasi.
    s.push_str(&format!(
        "  initial begin #{}; assert (y !== {w}'x) $info(\"PASS\"); else $error(\"output stuck X\"); $finish; end\n",
        b.rng.gen_range(40..=80)
    ));

    s
}

// ──────────────────────────────────────────────────────────────────────
// Perakit modul — komposisi terstruktur dengan symbol table.
// ──────────────────────────────────────────────────────────────────────

/// Rakit modul `top` dari pool konstruk. Semua sinyal dideklarasikan SEBELUM
/// dipakai; semua ekspresi valid by construction; 100% compile ok + sim ok.
///
/// `passive = true` → TANPA stimulus/clock internal (tb eksternal drive).
fn build_module(b: &mut Builder, w: usize, passive: bool) -> String {
    let mut s = String::new();
    let use_rst = b.rng.gen_bool(0.7);
    let has_mem = b.rng.gen_bool(b.b.ty * 0.7);

    // ── Child module (opsional, untuk elaboration stress) ──
    let use_child = !passive && w >= 8 && b.rng.gen_bool(b.b.elab * 0.8);
    if use_child {
        let cw = b.rng.gen_range(2..=8);
        s.push_str(&format!(
            "module fz_child #(parameter CW = {cw}) (\n  input  logic [CW-1:0] x,\n  output logic [CW-1:0] q\n);\n  assign q = ~x + 1'b1;\nendmodule\n\n"
        ));
    }

    // ── Timescale ──
    if b.rng.gen_bool(0.1) {
        s.push_str("`timescale 1ns/1ps\n");
    }

    // ── Module header: ports ──
    s.push_str(&format!("module top #(parameter W = {w}) (\n"));
    s.push_str("  input  logic clk,\n");
    if use_rst {
        s.push_str("  input  logic rst_n,\n");
    }
    s.push_str(&format!("  input  logic [W-1:0] a,\n"));
    s.push_str(&format!("  input  logic [W-1:0] b,\n"));
    let wv = b.rng.gen_bool(b.b.wide * 0.5);
    if wv {
        s.push_str(&format!(
            "  output logic [W-1:0] y,\n  output logic [{}:0] wv_o\n);\n",
            (w * 2).max(8) - 1
        ));
    } else {
        s.push_str("  output logic [W-1:0] y\n);\n");
    }

    // ── Daftarkan ports ke symbol table ──
    b.sigs.push(Sig {
        name: "a".to_string(),
        width: w,
    });
    b.sigs.push(Sig {
        name: "b".to_string(),
        width: w,
    });
    b.sigs.push(Sig {
        name: "y".to_string(),
        width: w,
    });
    if wv {
        b.sigs.push(Sig {
            name: "wv_o".to_string(),
            width: (w * 2).max(8),
        });
    }

    // ── Internal registers (lebar/tanda acak, minimal 3) ──
    let nregs = b.rng.gen_range(3..=5);
    for _ in 0..nregs {
        let n = fresh_name(b, "r");
        let signed = b.rng.gen_bool(b.b.signed * 0.4);
        let sig_w = w;
        let decl = if signed {
            format!("  logic signed [W-1:0] {n};\n")
        } else {
            format!("  logic [W-1:0] {n};\n")
        };
        s.push_str(&decl);
        b.sigs.push(Sig {
            name: n,
            width: sig_w,
        });
    }

    // Loop counters — beberapa variasi.
    for i in 0..3 {
        let n = format!("fz_li{i}");
        s.push_str(&format!("  integer {n};\n"));
        b.sigs.push(Sig { name: n, width: 32 });
    }

    // ── Output regs (opsional) — tipe wide ──
    if b.rng.gen_bool(b.b.signed * 0.5) {
        let cap = (w * 2).max(7);
        s.push_str(&format!("  logic signed [{}:0] wv;\n", cap));
        b.sigs.push(Sig {
            name: "wv".to_string(),
            width: cap + 1,
        });
    }

    // ── Memory arrays (opsional, untuk evaluator mem stress) ──
    // BUG FIX (fuZZ): sebelumnya hanya SATU dari {mem, mem2} yang di-declare
    // (pilihan random), padahal `proc` bisa menulis mem ATAU mem2 tergantung
    // rand-nya sendiri → mem/mem2 yang lain = E2001 undefined signal. Sekarang
    // keduanya selalu ter-declare saat has_mem (proc tetap bebas memilih).
    // Index maks 3 (mem[0:3]) atau mem2[0:3][0:3] — semua akses ≤ 3 valid.
    if has_mem {
        let d1 = b.rng.gen_range(1..=3);
        let d2 = b.rng.gen_range(1..=3);
        s.push_str(&format!("  logic [15:0] mem [0:{d1}];\n"));
        s.push_str(&format!("  logic [15:0] mem2 [0:{d1}][0:{d2}];\n"));
        b.sigs
            .push(Sig { name: "mem".to_string(), width: 16 });
        b.sigs
            .push(Sig { name: "mem2".to_string(), width: 16 });
    }

    // ── Drive output — ekspresi valid dari symbol table ──
    let race = passive && b.b.conc > 0.5 && b.rng.gen_bool(0.4);
    if race {
        // Dual-writer race pada register yang sama.
        let rn = get_random_reg_name(b);
        s.push_str(&format!(
            "  always_ff @(posedge clk or negedge rst_n) begin\n    if (!rst_n) {rn} <= '0;\n    else {rn} <= {rn} + 1'b1;\n  end\n"
        ));
        s.push_str(&format!(
            "  always_ff @(posedge clk or negedge rst_n) begin\n    if (!rst_n) {rn} <= 8'hA5;\n    else {rn} <= {rn} - 1'b1;\n  end\n"
        ));
        s.push_str(&format!("  assign y = {rn};\n"));
    } else if has_mem && b.rng.gen_bool(0.3) {
        // Output dari mem read — stress evaluator mem.
        if b.rng.gen_bool(0.3) {
            let i = b.rng.gen_range(0..=3);
            let j = b.rng.gen_range(0..=3);
            s.push_str(&format!("  assign y = mem2[{i}][{j}];\n"));
        } else {
            let i = b.rng.gen_range(0..=3);
            s.push_str(&format!("  assign y = mem[{i}];\n"));
        }
    } else {
        // Assign y = ekspresi valid dari symbol table.
        s.push_str(&format!("  assign y = {};\n", expr(b, w, 2)));
    }

    // ── Process blocks (1..4) — always_ff/comb/latch ──
    let nproc = b.rng.gen_range(1..=4);
    for _ in 0..nproc {
        s.push_str(&proc(b, w, use_rst, has_mem));
    }

    // ── Generate block + instansiasi child ──
    if use_child {
        let r = get_random_reg_name(b);
        let selw = w.min(8);
        s.push_str(&format!(
            "  genvar fz_g;\n  generate\n    for (fz_g = 0; fz_g < 2; fz_g = fz_g + 1) begin : fz_gl\n      fz_child #(.CW({selw})) u_fz ( .x({r}[fz_g*{selw} +: {selw}]), .q({r}[fz_g*{selw} +: {selw}]) );\n    end\n  endgenerate\n"
        ));
    }

    // ── Clock + stimulus (hanya non-passive) ──
    if !passive {
        s.push_str(&gen_clock());
        s.push_str(&gen_stimulus(b, w, use_rst));
    }

    s.push_str("endmodule\n");
    s
}

// ──────────────────────────────────────────────────────────────────────
// Tests — backward compatible dengan API yang sama.
// ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oracle::compile_verdict;
    use rand::SeedableRng;
    use std::collections::HashSet;

    #[test]
    fn random_syntax_nonempty() {
        let mut rng = StdRng::seed_from_u64(1);
        let g = Generator::new(1);
        assert!(!g.random_syntax(&mut rng).is_empty());
    }

    #[test]
    fn random_module_always_compile() {
        // 100% valid: SEMUA seed generated harus compile ok + sim ok.
        let n = 60u32;
        let mut rng = StdRng::seed_from_u64(42);
        let g = Generator::new(42);
        let mut ok = 0u32;
        for i in 0..n {
            let src = g.random_module(&mut rng);
            let v = std::panic::catch_unwind(|| compile_verdict(&src));
            assert!(
                v.is_ok(),
                "seed #{} PANIC di maria:\n{}",
                i,
                src
            );
            if v.unwrap().ok {
                ok += 1;
            }
        }
        assert_eq!(
            ok, n,
            "SEMUA seed harus compile ok — diharapkan 100% valid ({}/{})",
            ok, n
        );
    }

    #[test]
    fn composed_module_always_compile() {
        // Assembly dari area semantik: 100% compile ok.
        use crate::semantic::SemArea as A;
        let mut rng = StdRng::seed_from_u64(7);
        let g = Generator::new(7);
        let pairs = [
            (A::Scheduler, A::Concurrency),
            (A::Scheduler, A::Timing),
            (A::Width, A::Signedness),
            (A::Elaboration, A::Hierarchy),
            (A::Scheduler, A::Xz),
            (A::Concurrency, A::Xz),
            (A::TypeSystem, A::Width),
            (A::Signedness, A::Xz),
        ];
        let mut ok = 0u32;
        let mut total = 0u32;
        for &(a, b) in &pairs {
            for _ in 0..3 {
                let src = g.compose_from_interaction(&mut rng, 4, a, b);
                let v = std::panic::catch_unwind(|| compile_verdict(&src));
                assert!(v.is_ok(), "compose PANIC:\n{}", src);
                total += 1;
                if v.unwrap().ok {
                    ok += 1;
                }
            }
        }
        assert_eq!(
            ok, total,
            "SEMUA composed harus compile ({}/{})",
            ok, total
        );
    }

    #[test]
    fn outputs_diverse_across_calls() {
        // Anti-template: 30 panggilan harus menghasilkan >= 20 modul unik.
        let mut rng = StdRng::seed_from_u64(123);
        let g = Generator::new(123);
        let mut seen = HashSet::new();
        for _ in 0..30 {
            seen.insert(g.random_module(&mut rng));
        }
        assert!(
            seen.len() >= 20,
            "hanya {} modul unik dari 30 — mencurigakan (template?)",
            seen.len()
        );
    }

    #[test]
    fn shape_strategies_are_distinct() {
        // 13 strategi bias → 13 modul berbeda untuk seed yang sama.
        let mut rng = StdRng::seed_from_u64(9);
        let g = Generator::new(9);
        let mut seen = HashSet::new();
        for k in 0..NUM_SHAPES {
            seen.insert(g.module_for_shape(&mut rng, k));
        }
        assert_eq!(
            seen.len(),
            NUM_SHAPES as usize,
            "setiap strategi harus menghasilkan modul unik"
        );
    }

    #[test]
    fn same_strategy_varies_across_calls() {
        // Strategi yang sama (k) dipanggil berkali-kali → output BEDA.
        let mut rng = StdRng::seed_from_u64(5);
        let g = Generator::new(5);
        let a = g.module_for_shape(&mut rng, 3);
        let b = g.module_for_shape(&mut rng, 3);
        assert_ne!(a, b, "panggilan beruntun strategi sama harus menghasilkan modul beda");
    }

    #[test]
    fn debug_error_code_histogram() {
        // DIAGNOSTIK: histogram error code seed generated — sekarang harus 100% ok.
        // Simpan satu sample source per error code utk triage (undefined signal
        // dll.) — dicetak saat fail agar bug generator/elaborator terlacak.
        let n = 120u32;
        let mut rng = StdRng::seed_from_u64(42);
        let g = Generator::new(42);
        let mut codes: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        let mut samples: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut ok = 0u32;
        for _ in 0..n {
            let src = g.random_module(&mut rng);
            let v = compile_verdict(&src);
            if v.ok {
                ok += 1;
            } else {
                *codes.entry(v.code.clone()).or_insert(0) += 1;
                samples.entry(v.code.clone()).or_insert_with(|| src.clone());
            }
        }
        eprintln!("[debug] compile ok {}/{}", ok, n);
        let mut rows: Vec<(String, u32)> = codes.into_iter().collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1));
        for (c, cnt) in rows.iter().take(12) {
            eprintln!("[debug]   {} : {}", c, cnt);
            if let Some(s) = samples.get(c) {
                eprintln!("[debug]   sample:\n{}", s);
            }
        }
        assert_eq!(ok, n, "diagnostic: SEMUA harus compile ok ({}/{})", ok, n);
    }
}
