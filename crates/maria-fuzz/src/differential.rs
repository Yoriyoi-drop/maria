//! Oracle differential — deteksi penyimpangan semantik (bukan sekadar crash).
//!
//! Paper #13 (EMI, Le/Afshari/Su): equivalence modulo inputs — sisipan kode
//! mati (dead code) TIDAK boleh mengubah output. Input sama → output sama.
//! Paper #19 (DifuzzRTL): oracle nilai sinyal — fingerprint sinyal top
//! dibandingkan antar eksekusi.

use crate::harness;
use crate::oracle;
use crate::FuzzConfig;

#[derive(Debug, Clone, PartialEq)]
pub enum DiffVerdict {
    /// Kedua eksekusi identik (baik).
    Same,
    /// Penyimpangan terdeteksi — detail untuk bug report.
    Mismatch(String),
    /// Tidak bisa dibandingkan (compile gagal / nondeterministik sah).
    Skip,
}

/// Determinism check (Paper #13/#19): sumber sama dieksekusi 2x →
/// fingerprint sinyal harus identik. Menangkap state global bocor antar run.
pub fn determinism_check(source: &str, cfg: &FuzzConfig) -> DiffVerdict {
    if oracle::has_nondeterministic_src(source) {
        return DiffVerdict::Skip;
    }
    let f1 = harness::fingerprint_isolated(source, cfg.max_time, cfg.hang_ms);
    let f2 = harness::fingerprint_isolated(source, cfg.max_time, cfg.hang_ms);
    match (f1, f2) {
        (Some(a), Some(b)) if a == b => DiffVerdict::Same,
        (Some(a), Some(b)) => DiffVerdict::Mismatch(format!("run1 vs run2:\n  {}\n  {}", a, b)),
        _ => DiffVerdict::Skip,
    }
}

/// EMI variant: sisipkan kode mati (unused wire + assign) tepat sebelum
/// `endmodule`. Aman utk modul apa pun — tidak menyentuh sinyal/modul lain.
/// Output tidak boleh berubah (Paper #13 dead-code equivalence).
pub fn emi_variant(source: &str) -> String {
    const DEAD: &str = "  // EMI dead-code (Paper #13)\n  wire [7:0] _fuzz_dn;\n  assign _fuzz_dn = 8'h00;\n";
    let mut s = source.to_string();
    if let Some(end) = s.rfind("endmodule") {
        s.insert_str(end, DEAD);
    } else {
        s.push_str(DEAD);
    }
    s
}

/// Gaya variant EMI (Paper #13: equivalent mutants — kode mati TIDAK boleh
/// mengubah output; ekspresi ekivalen juga tidak).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EmiStyle {
    /// unused wire + assign konstanta (variant asli).
    DeadWire,
    /// unused always_comb yang membaca/menulis sinyal fuzz internal.
    DeadAlways,
    /// equivalent expression: tukar operan operator komutatif
    /// (`a + b` → `b + a`) — simetris utk seluruh nilai 4-state.
    ExprSwap,
}

/// Pilih style EMI deterministik dari konten source (hash) — minimizer butuh
/// variant yang PURE function dari source (kandidat di-minimize memanggil
/// emi_check berkali-kali; variant harus sama utk source sama).
pub fn emi_style_for(source: &str) -> EmiStyle {
    let h = source.bytes().fold(0x9e37_79b9u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x1000_0000_01b3)
    });
    match h % 3 {
        0 => EmiStyle::DeadWire,
        1 => EmiStyle::DeadAlways,
        _ => EmiStyle::ExprSwap,
    }
}

/// Bangun variant sesuai style.
pub fn emi_variant_ex(source: &str, style: EmiStyle) -> String {
    match style {
        EmiStyle::DeadWire => emi_variant(source),
        EmiStyle::DeadAlways => {
            const DEAD: &str = "  // EMI dead-code (Paper #13)\n  wire [7:0] _fuzz_dn;\n  logic _fuzz_dq;\n  always_comb begin\n    _fuzz_dq = 1'b0;\n    if (_fuzz_dn[0] && _fuzz_dn[7]) _fuzz_dq = 1'b1;\n  end\n";
            let mut s = source.to_string();
            if let Some(end) = s.rfind("endmodule") {
                s.insert_str(end, DEAD);
            } else {
                s.push_str(DEAD);
            }
            s
        }
        EmiStyle::ExprSwap => emi_expr_swap(source),
    }
}

/// Operator biner komutatif aman dtukar operannya (a op b == b op a utk
/// semua nilai 4-state, incl. X/Z) — Paper #13 equivalent mutant.
const COMMUTATIVE_OPS: &[u8] = b"+&|^*";

/// EMI equivalent-expression: tukar operan operator komutatif PERTAMA yang
/// ditemukan (`2 * 3 + a` → `2 * 3 + a` tidak berubah; `a + b` → `b + a`).
/// Deterministik (match pertama). Operand harus token identifier murni
/// (`[A-Za-z0-9_]`) dan tempat mu bukan di dalam komentar `//`/`/* */`.
pub fn emi_expr_swap(source: &str) -> String {
    let bytes = source.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let is_space = |c: u8| c == b' ' || c == b'\t';
    let mut i = 0usize;
    while i < bytes.len() {
        // Multi-char operator dua karakter (&&/||) — cek dulu sebelum char.
        let op_len = if bytes.get(i..i + 2).map(|w| w == b"&&" || w == b"||").unwrap_or(false) {
            2
        } else if COMMUTATIVE_OPS.contains(&bytes[i]) {
            1
        } else {
            i += 1;
            continue;
        };
        let op_at = i;
        let op_end = i + op_len;
        // Operan kiri: mundur dari op, lewati spasi/tab, harus ident murni.
        let mut li = i.saturating_sub(1);
        while li > 0 && is_space(bytes[li]) {
            li -= 1;
        }
        if !is_ident(bytes[li]) {
            i = op_end;
            continue;
        }
        let mut lstart = li;
        while lstart > 0 && is_ident(bytes[lstart - 1]) {
            lstart -= 1;
        }
        // Operan kanan: lewati spasi/tab, harus ident murni.
        let mut ri = op_end;
        while ri < bytes.len() && is_space(bytes[ri]) {
            ri += 1;
        }
        if ri >= bytes.len() || !is_ident(bytes[ri]) {
            i = op_end;
            continue;
        }
        let mut rend = ri + 1;
        while rend < bytes.len() && is_ident(bytes[rend]) {
            rend += 1;
        }
        // Guard token utuh: kedua operan harus identifier/number MANDIRI
        // (bukan member `obj.a`, bukan literal `2'b10`, bukan seleksi
        // `b[3:0]`, bukan argumen fungsi `f(x)`) — kalau tidak, swap bisa
        // merusak sintaks atau mengubah semantik.
        if lstart > 0 {
            let lb = bytes[lstart - 1];
            if matches!(lb, b'.' | b':' | b'\'' | b'[' | b'(' | b'#' | b'`') {
                i = op_end;
                continue;
            }
        }
        if rend < bytes.len() {
            let rb = bytes[rend];
            if is_ident(rb) || matches!(rb, b'.' | b':' | b'\'' | b'[' | b'(' | b'#' | b'`') {
                i = op_end;
                continue;
            }
        }
        // Lewati komentar `//` di baris yang sama (sebelum op) dan `/* ... */`.
        let line_start = source[..lstart].rfind('\n').map(|p| p + 1).unwrap_or(0);
        if source[line_start..lstart].contains("//") {
            i = op_end;
            continue;
        }
        if let Some(ci) = source[..lstart].rfind("/*") {
            let close = source[..lstart].rfind("*/");
            if close.map_or(true, |c| c < ci) {
                i = op_end;
                continue;
            }
        }
        // SWAP: `L op R` → `R op L`.
        let left = &source[lstart..li + 1];
        let right = &source[ri..rend];
        let mut s = source.to_string();
        let op_text = &source[op_at..op_end];
        s.replace_range(lstart..rend, &format!("{} {} {}", right, op_text, left));
        return s;
    }
    // Tidak ada operator komutatif aman → fallback dead-code.
    emi_variant(source)
}

/// EMI check: original vs variant dead-code — sinyal *common* harus sama.
/// Sinyal internal tambahan (`_fuzz_dn`) muncul hanya di variant → dibandingkan
/// hanya sinyal yang ada di kedua sisi (semantik EMI: output lestari).
/// Variant dipilih deterministik dari konten source (DeadWire/DeadAlways/
/// ExprSwap) — lihat `emi_style_for`.
pub fn emi_check(source: &str, cfg: &FuzzConfig) -> DiffVerdict {
    if oracle::has_nondeterministic_src(source) {
        return DiffVerdict::Skip;
    }
    let variant = emi_variant_ex(source, emi_style_for(source));
    let f_orig = harness::fingerprint_isolated(source, cfg.max_time, cfg.hang_ms);
    let f_var = harness::fingerprint_isolated(&variant, cfg.max_time, cfg.hang_ms);
    match (f_orig, f_var) {
        (Some(a), Some(b)) => match compare_common(&a, &b) {
            None => DiffVerdict::Same,
            Some(detail) => DiffVerdict::Mismatch(format!("orig vs emi-variant: {}", detail)),
        },
        _ => DiffVerdict::Skip,
    }
}

// ──────────────────────────────────────────────────────────────────────
// Oracle metamorfik identitas (GAP-5) — perluasan EMI dari "kode mati tidak
// boleh mengubah output" ke "relasi identitas semantik harus dipertahankan":
//
//   assign y = <rhs>;      ≡      assign y = (<rhs> op 0);
//
// untuk op ∈ {+, -, |, ^}. Identitas eksplisit di SV 4-state (*semua* nilai
// termasuk X/Z): x+0=x, x-0=x, x|0=x, x^0=x. Jika maria mengevaluasi `op 0`
// secara salah (lebar/signedness/order), fingerprint berubah → bug, walaupun
// evaluasi konsisten diri (menutup sebagian ORACLE GAP: oracle ini menangkap
// KESALAHAN SEMANTIK, bukan hanya inkonsistensi internal).
// ──────────────────────────────────────────────────────────────────────

/// Relasi identitas metamorfik (GAP-5):
///
///   assign y = rhs;      ≡      assign y = (rhs op 0);   op ∈ {|, ^}
///
/// HANYA op bitwise tanpa carry — identitas eksplisit di SV 4-state
/// (x|0=x, x^0=x utk SEMUA nilai termasuk X/Z).
///
/// ⚠️ `+ 0` / `- 0` DIKECUALIKAN: pada 4-state, `x + 0 ≠ x` bila x memuat
/// unknown, karena carry/borrow X menjalar ke seluruh bit (IEEE 1800 §11.4.3:
/// hasil aritmetika semua-X — perilaku engine maria memberi
/// `(~x + 0)` = xxxxxxxx saat `~x` = 1xxxxxxx: BENAR, bukan bug).
/// Mengikutkan +0/-0 di oracle = false positive (terbukti temuan seed77).
/// ──────────────────────────────────────────────────────────────────────

/// Operator identitas — deterministik dari hash source (minimizer memanggil
/// berkali-kali; varian harus pure function dari source, sama dgn EMI).
pub fn meta_style_for(source: &str) -> &'static str {
    let h = source.bytes().fold(0x9e37_79b9u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x1000_0000_01b3)
    });
    match h % 2 {
        0 => "| 0",
        _ => "^ 0",
    }
}

/// Bangun varian metamorfik-identitas: ganti `assign lhs = rhs;` pertama
/// yang AMAN (lhs sederhana tanpa selektor, rhs satu-baris tanpa `;$"/`)
/// menjadi `assign lhs = (rhs op 0);`. Deterministik (baris aman pertama).
/// None bila tidak ada assign yang memenuhi syarat.
pub fn meta_identity_variant(source: &str) -> Option<String> {
    let op = meta_style_for(source);
    let mut out = String::with_capacity(source.len() + 8);
    let mut replaced = false;
    for line in source.lines() {
        if !replaced {
            let t = line.trim();
            if t.starts_with("assign") && t.contains('=') && t.trim_end().ends_with(';') {
                let (lhs_raw, rhs_raw) = t.split_once('=').unwrap_or(("", ""));
                let lhs = lhs_raw.replace("assign", "").trim().to_string();
                let rhs = rhs_raw.trim().trim_end_matches(';').trim().to_string();
                let safe = !lhs.is_empty()
                    && !lhs.contains(' ')
                    && !lhs.contains('[')
                    && !lhs.contains('`')
                    && !lhs.contains('.')
                    && !lhs.contains('{')
                    && !rhs.is_empty()
                    && !rhs.contains(';')
                    && !rhs.contains('$')
                    && !rhs.contains('"');
                // Guard ARTEFAK minimizer (pelajaran seed77/fzH): lhs harus
                // ter-deklarasi dgn lebar ter-resolve (`[msb:0]` numerik) —
                // net implicit / parameter hilang (`child #(), CW tak
                // ter-deklarasi`) → lebar ambigu → varian bisa beda bukan
                // karena bug eval. Sama dgn guard property-oracle.
                let width_known = safe && crate::ast_mutate::declared_width(source, &lhs).is_some();
                if width_known {
                    out.push_str(&format!("assign {} = ({} {});", lhs, rhs, op));
                    out.push('\n');
                    replaced = true;
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if replaced {
        Some(out)
    } else {
        None
    }
}

/// Oracle metamorfik-identitas: original vs varian `(rhs op 0)` — sinyal
/// common harus identik di SEMUA waktu (fingerprint TRACE mid-simulation,
/// bukan hanya nilai final): bug transient (salah di delta lalu pulih) kini
/// terlihat. Identitas bitwise bersifat time-invariant → sound.
pub fn meta_identity_check(source: &str, cfg: &FuzzConfig) -> DiffVerdict {
    if oracle::has_nondeterministic_src(source) {
        return DiffVerdict::Skip;
    }
    let Some(variant) = meta_identity_variant(source) else {
        return DiffVerdict::Skip;
    };
    // Interval sampling diskalakan dgn max_time (deterministik per source).
    let iv = (cfg.max_time.max(8)) / 8;
    let f_orig = harness::trace_isolated(source, cfg.max_time, cfg.hang_ms, iv);
    let f_var = harness::trace_isolated(&variant, cfg.max_time, cfg.hang_ms, iv);
    match (f_orig, f_var) {
        (Some(a), Some(b)) => match compare_common(&a, &b) {
            None => DiffVerdict::Same,
            Some(detail) => DiffVerdict::Mismatch(format!("orig vs meta-identity: {}", detail)),
        },
        _ => DiffVerdict::Skip,
    }
}

/// Urai fingerprint `name=bits@width|name=...` → map nama → nilai.
fn fingerprint_map(fp: &str) -> std::collections::HashMap<String, String> {
    fp.split('|')
        .filter(|row| row.contains('='))
        .map(|row| {
            let (name, value) = row.split_once('=').unwrap_or((row, ""));
            (name.to_string(), value.to_string())
        })
        .collect()
}

/// Bandingkan hanya sinyal yang ada di kedua sisi.
/// None = identik; Some(detail) = daftar penyimpangan.
///
/// Sinyal INTERNAL artefak fuzzer diabaikan: prefix `fz_`, `_fz_`, `_fuzz_`
/// (semuanya ditanam oracle generator/grammar/assert-mirror/interface; `fz_q1`,
/// `fz_y`, `_fz_atA/B`, `_fz_viol`, `_fuzz_pa`, `fz_bif`, `fz_d`, dsb).
/// Dead-code menambah net yang utk source malformed (multi-driver / implicit
/// net) mengubah net-topology → `z` vs `x` beda, PADAHAL bukan bug engine.
/// EMI bug sejati = dead-code mengubah sinyal TOP nyata (`y`, `r`, `flag`,
/// sinyal user non-`fz_`). Nota: kalau user menulis sinyal sendiri berawalan
/// `fz_`, EMI tak bisa membedakannya — konvensi internal fuzzer.
fn is_internal_artifact(name: &str) -> bool {
    let base = name.rsplit('.').next().unwrap_or(name);
    base.starts_with("fz_") || base.starts_with("_fz_") || base.starts_with("_fuzz")
}

/// Bandingkan hanya sinyal yang ada di kedua sisi, mengabaikan artefak internal.
fn compare_common(a: &str, b: &str) -> Option<String> {
    let ma = fingerprint_map(a);
    let mb = fingerprint_map(b);
    let mut diffs: Vec<String> = Vec::new();
    for (name, va) in &ma {
        if is_internal_artifact(name) {
            continue;
        }
        if let Some(vb) = mb.get(name) {
            if va != vb {
                diffs.push(format!("{}: {} vs {}", name, va, vb));
            }
        }
    }
    if diffs.is_empty() {
        None
    } else {
        Some(diffs.join("; "))
    }
}

// ──────────────────────────────────────────────────────────────────────
// Differential vs tool referensi LRM (iverilog/verilator) — jembatan ke
// paritas VCS/Questa-class. Target maria "sejajar VCS" = semantik IEEE 1800
// yang benar (mis. multi-driver race → X, bukan last-writer deterministik;
// ekstensi lebar/tanda; region ordering). Oracle konsistensi-diri (determinism
// /EMI/metamorphic) BUTA terhadap "salah konsisten" — differential inilah yang
// menangkap deviasi vs LRM. DUT = core PASIF (dari gen::passive_core): tanpa
// stimulus/clock internal agar tb eksternal drive input tanpa konflik wire
// (maria longgar meng-drive input; iverilog/VCS menolak).
// ──────────────────────────────────────────────────────────────────────

/// Cari executable di PATH (tanpa dep eksternal).
fn find_on_path(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Port DUT yang di-parse dari header `module top ... (...)`: (dir, width,
/// name). Heuristik cukup utk core generated maria (format konsisten).
fn parse_ports(source: &str) -> Option<(String, Vec<(String, usize, String)>)> {
    let lines: Vec<&str> = source.lines().collect();
    let mut mi = None;
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim_start();
        if t.starts_with("module ") {
            let after = t[7..].trim();
            let name: String = after
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            mi = Some((i, name));
            break;
        }
    }
    let (start, mname) = mi?;
    // Kumpulkan teks header modul sampai `);`.
    let mut buf = String::new();
    let mut closed = false;
    for l in &lines[start..] {
        let t = l.trim();
        if let Some(pos) = t.find(");") {
            // Potong hanya bagian sebelum `);` (menghindari isi body).
            let cut = if t.starts_with("module") || t.contains('#') {
                t.split(");").next().unwrap_or(t)
            } else {
                t
            };
            buf.push_str(cut);
            closed = true;
            break;
        }
        buf.push_str(t);
        buf.push('\n');
    }
    if !closed {
        return None;
    }
    // Ambil segmen setelah buka-tanda kurung port (setelah `#(...)` atau `(`, )
    let open = buf.find('(')?;
    let inner = &buf[open + 1..];
    let mut ports = Vec::new();
    for chunk in inner.split(',') {
        let c = chunk.trim();
        if c.is_empty() {
            continue;
        }
        let dir = if c.contains("input") {
            "input"
        } else if c.contains("output") {
            "output"
        } else {
            continue; // non-port token
        };
        // Lebar dari `[msb-1:0]` / `[msb:0]` / `[w:0]`.
        let width = if let (Some(a), Some(b)) = (c.find('['), c.find(']')) {
            let range = &c[a + 1..b];
            let mut width = 1usize;
            if let Some((msb_s, _lsb)) = range.split_once(':') {
                let msb_s = msb_s.trim();
                if let Some((x, y)) = msb_s.split_once('-') {
                    if let (Ok(x), Ok(y)) = (x.trim().parse::<i64>(), y.trim().parse::<i64>()) {
                        width = (x - y + 1).max(1) as usize;
                    }
                } else if let Ok(x) = msb_s.parse::<i64>() {
                    width = (x + 1).max(1) as usize;
                }
            }
            width
        } else {
            1
        };
        // Nama = identifier terakhir (buang tipe/kata kunci).
        let name = c
            .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
            .filter(|t| !t.is_empty())
            .last()?
            .trim_end_matches(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
            .to_string();
        ports.push((dir.to_string(), width, name));
    }
    if ports.is_empty() {
        return None;
    }
    Some((mname, ports))
}

/// Nilai stimulus deterministik per (nama, waktu, iterasi-hash).
fn ivl_val(name: &str, t: u64) -> u64 {
    let mut h = 0x9E3779B97F4A7C15u64;
    for b in name.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x1000_0000_01b3);
    }
    h = (h ^ t).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}

/// Bangun testbench: drive input + sampel output di beberapa waktu →
/// tulis `dtrace.txt` (format `%0t <out>=%b` per baris) — SAMA utk maria
/// dan iverilog (perbandingan adil).
fn build_tb(module: &str, ports: &[(String, usize, String)], max_time: u64) -> String {
    let mut s = String::new();
    s.push_str("`timescale 1ns/1ps\n");
    s.push_str("module tb;\n");
    s.push_str("  logic clk;\n");
    s.push_str("  logic rst_n;\n");
    let mut conns = Vec::new();
    for (dir, w, name) in ports {
        if dir == "input" {
            if name == "clk" {
                conns.push(format!(".clk(clk)"));
            } else if name == "rst_n" {
                conns.push(format!(".rst_n(rst_n)"));
            } else if *w == 1 {
                s.push_str(&format!("  logic {};\n", name));
                conns.push(format!(".{}({})", name, name));
            } else {
                s.push_str(&format!("  logic [{}:0] {};\n", w - 1, name));
                conns.push(format!(".{}({})", name, name));
            }
        } else {
            if *w == 1 {
                s.push_str(&format!("  wire {};\n", name));
            } else {
                s.push_str(&format!("  wire [{}:0] {};\n", w - 1, name));
            }
            conns.push(format!(".{}({})", name, name));
        }
    }
    s.push_str(&format!(
        "  {} u ({});\n",
        module,
        conns.join(", ")
    ));
    // Clock generik.
    s.push_str("  initial begin clk = 0; forever #5 clk = ~clk; end\n");
    // Stimulus: nilai deterministik per port di t=3 dan t=41.
    s.push_str("  initial begin\n    rst_n = 0;\n");
    for (dir, w, name) in ports {
        if dir != "input" || name == "clk" || name == "rst_n" {
            continue;
        }
        let v = ivl_val(name, 3) & ((1u64 << (*w).min(16)) - 1);
        s.push_str(&format!("    {} = {};\n", name, v));
    }
    s.push_str("    #9 rst_n = 1;\n");
    for (dir, w, name) in ports {
        if dir != "input" || name == "clk" || name == "rst_n" {
            continue;
        }
        let v = ivl_val(name, 41) & ((1u64 << (*w).min(16)) - 1);
        s.push_str(&format!("    #13 {} = {};\n", name, v));
    }
    let horizon = max_time.min(100).max(30);
    s.push_str(&format!("    #{} $finish;\n  end\n", horizon));
    // Trace: sampel output di 4 waktu.
    s.push_str("  initial begin\n    integer f;\n    f = $fopen(\"dtrace.txt\", \"w\");\n");
    for t in [3u64, 25, 55, (horizon as u64 - 1).min(85)] {
        if t <= horizon {
            s.push_str(&format!("    #{};\n", t));
            for (dir, _w, name) in ports {
                if dir == "output" {
                    s.push_str(&format!("    $fdisplay(f, \"T{} {} = %0d\", {});\n", t, name, name));
                }
            }
        }
    }
    s.push_str("    $fclose(f);\n  end\n");
    s.push_str("endmodule\n");
    s
}

/// Ekstrak sampel (waktu, nilai) dari trace teks — tahan perbedaan newline
/// (maria `$fdisplay` kadang tanpa '\n' saat banyak tulis ke handle sama;
/// iverilog pakai newline). Format: `T<time> <name> = <value>`.
fn trace_samples(s: &str) -> Vec<(u64, String)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == b'T' {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 1 {
                let t: u64 = std::str::from_utf8(&b[i + 1..j])
                    .ok()
                    .and_then(|x| x.parse().ok())
                    .unwrap_or(0);
                let mut k = j;
                while k < b.len() && b[k] != b'=' {
                    k += 1;
                }
                if k < b.len() {
                    let mut v = k + 1;
                    while v < b.len() && b[v] == b' ' {
                        v += 1;
                    }
                    let vs = v;
                    while v < b.len()
                        && (b[v].is_ascii_digit()
                            || matches!(b[v], b'x' | b'X' | b'z' | b'Z'))
                    {
                        v += 1;
                    }
                    out.push((
                        t,
                        std::str::from_utf8(&b[vs..v]).unwrap_or("").to_string(),
                    ));
                    i = v;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

/// Jalankan tool eksternal di temp dir, tulis `dtrace.txt`; Ok = sukses.
fn run_tool_trace(bin: &str, args: &[&str], dir: &std::path::Path) -> bool {
    let st = std::process::Command::new(bin)
        .args(args)
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    st.map(|s| s.success()).unwrap_or(false)
}

/// Jalankan verilator (`--binary`) — proxy LRM kedua; lebih mahal dari
/// iverilog → dipakai KONFIRMASI, bukan screening.
fn run_verilator_trace(core: &str, tb: &str, dir: &std::path::Path) -> Option<String> {
    if find_on_path("verilator").is_none() {
        return None;
    }
    let _ = std::fs::write(dir.join("core.sv"), core);
    let _ = std::fs::write(dir.join("tb.sv"), tb);
    if !run_tool_trace(
        "verilator",
        &[
            "--binary",
            "-O0",
            "-Wno-fatal",
            "--top-module",
            "tb",
            "-o",
            "vprog",
            "core.sv",
            "tb.sv",
        ],
        dir,
    ) {
        return None;
    }
    if !run_tool_trace("./vprog", &[], dir) {
        return None;
    }
    std::fs::read_to_string(dir.join("dtrace.txt")).ok()
}

/// Oracle referensi (publik): bandingkan trace maria vs iverilog (+ konfirmasi
/// verilator). Temp dir dibersihkan setelah selesai — prinsip "0 file saat
/// normal"; matikan pembersihan dgn env `MARIA_FUZZ_KEEP_TMP` (debug trace).
pub fn reference_vs_ivl(core: &str, cfg: &FuzzConfig) -> DiffVerdict {
    let dir = std::env::temp_dir().join(format!(
        "maria_ref_{:x}",
        core.bytes().fold(0x6d61_7269u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3))
    ));
    let v = reference_vs_ivl_at(core, cfg, &dir);
    if std::env::var("MARIA_FUZZ_KEEP_TMP").is_err() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    v
}

/// Inti oracle referensi — `dir` disediakan pemanggil (wrapper pembersihan).
fn reference_vs_ivl_at(core: &str, cfg: &FuzzConfig, dir: &std::path::Path) -> DiffVerdict {
    let Some(ivl) = find_on_path("iverilog") else {
        return DiffVerdict::Skip;
    };
    let Some(vvp) = find_on_path("vvp") else {
        return DiffVerdict::Skip;
    };
    let maria_bin = std::env::var("MARIA_FUZZ_BIN").unwrap_or_else(|_| {
        let cwd = std::env::current_dir().unwrap_or_default();
        for cand in [
            "target/debug/maria",
            "../target/debug/maria",
            "../../target/debug/maria",
            "maria/target/debug/maria",
        ] {
            let p = cwd.join(cand);
            if p.is_file() {
                return p.to_string_lossy().to_string();
            }
        }
        cwd.join("target/debug/maria").to_string_lossy().to_string()
    });
    if !std::path::Path::new(&maria_bin).exists() {
        return DiffVerdict::Skip;
    }
    let Some((mname, ports)) = parse_ports(core) else {
        return DiffVerdict::Skip;
    };
    let tb = build_tb(&mname, &ports, cfg.max_time);

    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("[ref] gagal buat temp dir: {}", e);
        return DiffVerdict::Skip;
    }
    let _ = std::fs::write(dir.join("core.sv"), core);
    let _ = std::fs::write(dir.join("tb.sv"), &tb);

    // Jalankan maria (compile+sim) → dtrace.txt.
    let mut cmd = std::process::Command::new(&maria_bin);
    cmd.args(["-T", &cfg.max_time.to_string(), "tb.sv", "core.sv"])
        .current_dir(&dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if cmd.status().map(|s| !s.success()).unwrap_or(true) {
        return DiffVerdict::Skip; // maria gagal compile/sim seed → bukan deviasi
    }
    let trace_m = std::fs::read_to_string(dir.join("dtrace.txt")).unwrap_or_default();
    if trace_m.is_empty() {
        return DiffVerdict::Skip;
    }

    // Jalankan iverilog + vvp → dtrace.txt (overwrite; tb sama).
    let ivl_ok = run_tool_trace(&ivl.to_string_lossy(), &["-g2012", "-o", "prog", "core.sv", "tb.sv"], &dir)
        && run_tool_trace(&vvp.to_string_lossy(), &["prog"], &dir);
    if !ivl_ok {
        return DiffVerdict::Skip;
    }
    let trace_i = std::fs::read_to_string(dir.join("dtrace.txt")).unwrap_or_default();
    if trace_i.is_empty() {
        return DiffVerdict::Skip;
    }

    let ma = trace_samples(&trace_m);
    let mi = trace_samples(&trace_i);
    if ma == mi {
        return DiffVerdict::Same;
    }

    // ── Konfirmasi verilator (proxy LRM kedua) ──
    let vt = run_verilator_trace(core, &tb, &dir);
    if let Some(vtrace) = &vt {
        let mv = trace_samples(vtrace);
        if mv == ma {
            // maria setuju dgn verilator; iverilog outlier — tool-variance,
            // bukan deviasi maria (JANGAN diklaim).
            return DiffVerdict::Skip;
        }
    }

    let mut diffs = Vec::new();
    let n = ma.len().max(mi.len());
    for i in 0..n {
        let a = ma.get(i).map(|(t, v)| format!("t{}={}", t, v)).unwrap_or_else(|| "<missing>".to_string());
        let b = mi.get(i).map(|(t, v)| format!("t{}={}", t, v)).unwrap_or_else(|| "<missing>".to_string());
        if a != b {
            diffs.push(format!("{} | maria:{}  ivl:{}", i, a, b));
        }
        if diffs.len() >= 6 {
            break;
        }
    }
    DiffVerdict::Mismatch(format!("vs iverilog (LRM proxy): {}", diffs.join(" ;; ")))
}

/// Jalankan command dengan stdout/stderr dibuang; Ok(())=sukses.
fn io_discard_command(bin: &std::path::Path, dir: &std::path::Path, args: &[&str]) -> Result<(), std::io::Error> {
    let st = std::process::Command::new(bin)
        .args(args)
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if st.success() {
        Ok(())
    } else {
        Err(std::io::Error::new(std::io::ErrorKind::Other, "non-zero"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FuzzConfig;

    const COUNTER: &str = r#"
module top(input logic clk, input logic rst_n,
           input logic [3:0] a, input logic [3:0] b,
           output logic [3:0] y);
  logic [3:0] r;
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) r <= '0;
    else r <= a + b;
  end
  assign y = r;
  initial begin clk = 0; forever #5 clk = ~clk; end
  initial begin rst_n = 0; a = 1; b = 2; #7 rst_n = 1; #3 a = 3; b = 4; end
endmodule
"#;

    fn cfg() -> FuzzConfig {
        FuzzConfig {
            max_time: 40,
            hang_ms: 2000,
            ..FuzzConfig::default()
        }
    }

    #[test]
    fn emi_variant_valid_and_larger() {
        let v = emi_variant(COUNTER);
        assert!(v.len() > COUNTER.len());
        assert!(v.contains("_fuzz_dn"));
        assert!(v.trim_end().ends_with("endmodule"));
    }

    #[test]
    fn emi_style_deterministic_per_source() {
        // Minimizer memanggil emi_check berulang — variant harus pure function.
        for _ in 0..10 {
            let a = emi_style_for(COUNTER);
            let b = emi_style_for(COUNTER);
            assert_eq!(a, b, "style harus deterministik utk source sama");
        }
    }

    #[test]
    fn emi_expr_swap_reorders_first_commutative_op() {
        // variant harus menukar `a + b` → `b + a` dan hasil sim tetap sama.
        let ab = "module top(input logic [3:0] a, input logic [3:0] b, output logic [3:0] y);\n  assign y = a + b;\nendmodule\n";
        let v = emi_expr_swap(ab);
        assert!(v.contains("b + a"), "operan harus tertukar: {}", v);
        let cfg = cfg();
        // fingerprints identik (commutativity & width sama) — DiffVerdict::Same.
        let f_orig = harness::fingerprint_isolated(ab, cfg.max_time, cfg.hang_ms);
        let f_var = harness::fingerprint_isolated(&v, cfg.max_time, cfg.hang_ms);
        match (f_orig, f_var) {
            (Some(a), Some(b)) => assert_eq!(a, b, "a+b vs b+a harus identik"),
            _ => panic!("fingerprint gagal utk ekspresi sederhana"),
        }
    }

    #[test]
    fn emi_all_styles_preserve_counter_semantics() {
        let cfg = cfg();
        for style in [
            EmiStyle::DeadWire,
            EmiStyle::DeadAlways,
            EmiStyle::ExprSwap,
        ] {
            let v = emi_variant_ex(COUNTER, style);
            assert!(v.len() > COUNTER.len() || v != COUNTER, "variant {:?} harus beda", style);
            assert_eq!(emi_check(COUNTER, &cfg), DiffVerdict::Same, "style {:?} merusak counter", style);
        }
    }

    #[test]
    fn emi_dead_always_variant_contains_consult() {
        let v = emi_variant_ex(COUNTER, EmiStyle::DeadAlways);
        assert!(v.contains("_fuzz_dq"));
        assert!(v.contains("always_comb"));
    }

    #[test]
    fn determinism_same_for_counter() {
        assert_eq!(determinism_check(COUNTER, &cfg()), DiffVerdict::Same);
    }

    #[test]
    fn emi_same_for_counter() {
        // Dead-code insertion wajib menghasilkan fingerprint identik.
        assert_eq!(emi_check(COUNTER, &cfg()), DiffVerdict::Same);
    }

    #[test]
    fn nondeterministic_skipped() {
        let src = format!("{}\ninitial $display($urandom());\n", COUNTER);
        assert_eq!(determinism_check(&src, &cfg()), DiffVerdict::Skip);
    }

    #[test]
    fn internal_artifact_filter() {
        // Sinyal top nyata (user) → tidak di-filter.
        assert!(!is_internal_artifact("y"));
        assert!(!is_internal_artifact("r"));
        assert!(!is_internal_artifact("tl_h_i"));
        // Artefak internal fuzzer (oracle/gen/grammar) → di-filter.
        assert!(is_internal_artifact("fz_q1"));
        assert!(is_internal_artifact("_fz_atA_7"));
        assert!(is_internal_artifact("_fz_viol_7"));
        assert!(is_internal_artifact("_fuzz_pa_123"));
        assert!(is_internal_artifact("fz_u_5.fz_d_14800"));
        assert!(is_internal_artifact("fz_bif_3"));
        // Top-level output bersih terhadap internal.
        let ma = "y=00@2|r=00@2|fz_q1=1@1|_fuzz_pa_5=zzz@3";
        let mb = "y=00@2|r=00@2|fz_q1=x@1|_fuzz_pa_5=001@3";
        assert_eq!(compare_common(ma, mb), None, "hanya artefak beda → bukan bug EMI");
    }

    #[test]
    fn meta_identity_variant_changes_source() {
        let v = meta_identity_variant(COUNTER).expect("counter punya assign aman");
        assert_ne!(v, COUNTER);
        assert!(v.contains("0);"), "variant harus memuat `op 0`: {}", v);
    }

    #[test]
    fn meta_identity_same_for_counter() {
        // Soundness: `(r op 0) ≡ r` utk semua nilai — validasi relasi identitas
        // pada design nyata (harus Same, bukan false positive).
        let cfg = cfg();
        assert_eq!(meta_identity_check(COUNTER, &cfg), DiffVerdict::Same);
    }

    #[test]
    fn meta_identity_sensitive_to_semantic_change() {
        // Sensitivitas: `(a + 1)` BUKAN identitas → fingerprint harus beda
        // (bukti oracle benar-benar mendeteksi perubahan semantik).
        let cfg = cfg();
        let src = "module top(input logic [3:0] a, output logic [3:0] y);\n  assign y = a;\n  initial begin a = 4'd5; end\nendmodule\n";
        let bad = src.replace("assign y = a;", "assign y = (a + 1'b1);");
        let f1 = harness::fingerprint_isolated(src, cfg.max_time, cfg.hang_ms);
        let f2 = harness::fingerprint_isolated(&bad, cfg.max_time, cfg.hang_ms);
        match (f1, f2) {
            (Some(a), Some(b)) => assert_ne!(a, b, "a vs a+1 harus beda"),
            _ => panic!("fingerprint gagal utk program sederhana"),
        }
    }

    #[test]
    fn meta_identity_sound_with_unknown_carry() {
        // IEEE 1800: `(rhs + 0) ≠ rhs` bila rhs memuat X — carry X menjalar
        // ke semua bit (maria memberi semua-X = BENAR, bukan bug). Identitas
        // metamorfik hanya berlaku utk op bitwise tanpa carry (`|`, `^`);
        // oracle wajib membatasi diri agar tidak false positive (pelajaran
        // temuan kampanye seed 77: `(~x + 0)` → all-X).
        let cfg = cfg();
        // Sanity: perilaku aritmetika X nggak identitas (dokumentasi caveat).
        let plus = "module top;\n  logic [6:0] a;\n  wire [7:0] x;\n  assign x = a;\n  wire [7:0] q;\n  assign q = (~x + 0);\nendmodule\n";
        let fp = harness::fingerprint_isolated(plus, cfg.max_time, cfg.hang_ms);
        assert_eq!(
            fp.map(|f| f.contains("q=xxxxxxxx@8")),
            Some(true),
            "x+0 semua-X = carry X merambat (perilaku LRM)"
        );
        // Identitas bitwise dipertahankan meski x memuat X sebagian.
        let src = "module top;\n  logic [6:0] a;\n  wire [7:0] x;\n  assign x = a;\n  wire [7:0] q;\n  assign q = (~x | 0);\nendmodule\n";
        assert_eq!(
            meta_identity_check(src, &cfg),
            DiffVerdict::Same,
            "|0 identity harus dipertahankan pada nilai 4-state parsial"
        );
    }

    #[test]
    fn meta_style_for_restricted_to_bitwise() {
        // Oracle metamorfik HANYA memakai op bitwise tanpa carry.
        for _ in 0..200 {
            let s = meta_style_for("module top; endmodule");
            assert!(s == "| 0" || s == "^ 0", "op harus bitwise: {}", s);
        }
    }

    #[test]
    fn reference_vs_ivl_detects_multiwriter_race() {
        // Regression: multi-driver race — maria resolve deterministik,
        // iverilog (LRM) beri X → oracle referensi WAJIB Mismatch.
        if find_on_path("iverilog").is_none() || find_on_path("vvp").is_none() {
            eprintln!("skip: iverilog/vvp tidak ada di PATH");
            return;
        }
        let core = r#"module top(input logic clk, input logic rst_n, input logic [7:0] a, output logic [7:0] y);
  logic [7:0] r;
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) r <= '0;
    else r <= a;
  end
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) r <= 8'hA5;
    else r <= r + 8'd1;
  end
  assign y = r;
endmodule"#;
        match reference_vs_ivl(core, &cfg()) {
            DiffVerdict::Mismatch(d) => {
                assert!(!d.is_empty(), "detail mismatch harus terisi");
            }
            DiffVerdict::Same => {
                panic!("race multi-driver harus beda: maria deterministik vs iverilog X")
            }
            DiffVerdict::Skip => panic!("oracle harus bisa menjalankan kedua tool"),
        }
    }

    #[test]
    fn reference_vs_ivl_same_on_single_driver() {
        // Sanity: DUT single-driver deterministik → trace maria == iverilog
        // (bukan false-positive).
        if find_on_path("iverilog").is_none() || find_on_path("vvp").is_none() {
            eprintln!("skip: iverilog/vvp tidak ada di PATH");
            return;
        }
        let core = r#"module top(input logic clk, input logic rst_n, input logic [7:0] a, output logic [7:0] y);
  logic [15:0] acc;
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) acc <= 16'd0;
    else acc <= acc + {8'd0, a};
  end
  assign y = acc[7:0];
endmodule"#;
        assert_eq!(
            reference_vs_ivl(core, &cfg()),
            DiffVerdict::Same,
            "DUT single-driver harus identik dgn iverilog"
        );
    }

    #[test]
    fn reference_vs_ivl_repro_minimized_race_finding() {
        // Regression ter-minimize dari kampanye --ref-diff: 3 writer NBA
        // pada reg SAMA saat reset — maria resolve deterministik, iverilog
        // (LRM) beri X → oracle HARUS tetap Mismatch (stabil/re-reproducible).
        if find_on_path("iverilog").is_none() || find_on_path("vvp").is_none() {
            eprintln!("skip: iverilog/vvp tidak ada di PATH");
            return;
        }
        let core = r#"module top #(parameter W = 2) (
  input  logic clk,
  input  logic rst_n,
  input  logic [2-1:0] a,
  output logic [7:0] wv_o);
  logic [1:0] r2;
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) r2 <= '0;
  end
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) r2 <= 8'hA5;
  end
  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) r2 <= '0;
    else begin
    end
  end
endmodule"#;
        match reference_vs_ivl(core, &cfg()) {
            DiffVerdict::Mismatch(d) => assert!(!d.is_empty()),
            other => panic!("temuan race reset harus tetap Mismatch, dapat {:?}", other),
        }
    }

    // ── Deep differential: jalur eksekusi internal ──

/// Deep differential (jalur internal): jalankan source dgn flag jalur
/// evaluasi (packed/DAG/timing-wheel/MIR) vs jalur standard — fingerprint
/// WAJIB identik utk input sama; beda = bug internal engine (SIM-28 dll.).
const PATH_HANG_MS: u64 = 3000;

/// Bandingkan fingerprint jalur ber-flag vs jalur standard.
fn path_flag_diff(source: &str, max_time: u64, flags: maria_api::EngineFlags) -> DiffVerdict {
    let f1 = harness::fingerprint_isolated_flags(source, max_time, PATH_HANG_MS, flags);
    let f2 = harness::fingerprint_isolated(source, max_time, PATH_HANG_MS);
    match (f1, f2) {
        (Some(a), Some(b)) if a == b => DiffVerdict::Same,
        (Some(a), Some(b)) => DiffVerdict::Mismatch(format!("path-flag:\n  {}\n  {}", a, b)),
        _ => DiffVerdict::Skip,
    }
}

/// packed-eval vs evaluator standard.
fn path_packed_vs_standard(source: &str, max_time: u64) -> DiffVerdict {
    path_flag_diff(
        source,
        max_time,
        maria_api::EngineFlags {
            use_packed_eval: true,
            ..maria_api::EngineFlags::default()
        },
    )
}

/// timing-wheel vs vector queue.
fn path_timing_wheel_vs_vec(source: &str, max_time: u64) -> DiffVerdict {
    path_flag_diff(
        source,
        max_time,
        maria_api::EngineFlags {
            use_timing_wheel: true,
            ..maria_api::EngineFlags::default()
        },
    )
}

/// DAG-parallel vs serial evaluasi.
fn path_dag_vs_serial(source: &str, max_time: u64) -> DiffVerdict {
    path_flag_diff(
        source,
        max_time,
        maria_api::EngineFlags {
            use_dag_parallel: true,
            ..maria_api::EngineFlags::default()
        },
    )
}

/// MIR JIT vs interpreter.
fn path_mir_jit_vs_interpreted(source: &str, max_time: u64) -> DiffVerdict {
    path_flag_diff(
        source,
        max_time,
        maria_api::EngineFlags {
            use_mir_jit: true,
            ..maria_api::EngineFlags::default()
        },
    )
}

    // ── Deep differential: jalur eksekusi internal ──

    #[test]
    fn path_packed_vs_standard_same() {
        let src = "module p; logic [7:0] a,b,y; assign a=8'hF0; assign b=8'h0F; assign y = a & b; initial #5 $finish; endmodule";
        let v = path_packed_vs_standard(src, 10);
        assert!(matches!(v, DiffVerdict::Same), "packed==standard: {:?}", v);
    }

    #[test]
    fn path_timing_wheel_vs_vec_same() {
        let src = "module t; logic clk; logic [3:0] c; initial clk=0; always #5 clk=~clk; always_ff @(posedge clk) c <= c + 1; initial #50 $finish; endmodule";
        let v = path_timing_wheel_vs_vec(src, 60);
        assert!(matches!(v, DiffVerdict::Same), "wheel==vec: {:?}", v);
    }

    #[test]
    fn path_dag_vs_serial_same() {
        let src = "module d; logic [7:0] a,b,c; assign a = 8'd5; assign b = a + 8'd3; assign c = b * 8'd2 + a; initial #5 $finish; endmodule";
        let v = path_dag_vs_serial(src, 10);
        assert!(matches!(v, DiffVerdict::Same), "dag==serial: {:?}", v);
    }

    #[test]
    fn path_mir_jit_vs_interpreted_same() {
        let src = "module m; logic [7:0] a,b,y; assign a=8'd6; assign b=8'd7; assign y = a * b + a; initial #5 $finish; endmodule";
        let v = path_mir_jit_vs_interpreted(src, 10);
        assert!(matches!(v, DiffVerdict::Same), "mir==interp: {:?}", v);
    }

    #[test]
    fn cross_file_pair_ok() {
        // Dua file valid digabung → masih valid (tidak boleh error palsu).
        let a = "package pa; localparam int P = 4; endpackage\n";
        let b = "module mb; import pa::*; logic [P-1:0] x; assign x = '0; endmodule\n";
        let joined = format!("{}{}", a, b);
        assert!(maria_api::compile_diag_counts(&joined).0 == 0);
    }
}