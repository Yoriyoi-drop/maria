//! Maria HDL (.mv) — bahasa baru milik Maria untuk menulis RTL yang lebih bersih,
//! di-transpile ke SystemVerilog `.sv`/`.svh` oleh tool `maria mgen`.
//!
//! Pipeline: `.mv` → lexer → parser → AST → check → codegen → `.sv` + `.svh`
//! (lihat MARIA-HDL.md untuk spesifikasi bahasa).
//!
//! 1 file = 1 tanggung jawab:
//! - `lexer.rs`  — tokenizer
//! - `parser.rs` — recursive descent → `ast::MvFile`
//! - `check.rs`  — type-check & semantic (E2001–E2007, MARIA-HDL.md §9)
//! - `codegen.rs`— emitter SystemVerilog (`.sv`/`.svh`)

pub mod ast;
pub mod check;
pub mod codegen;
pub mod lexer;
pub mod parser;

use crate::ast::MvFile;
use std::fmt;

/// Error parse/lex Maria HDL dengan posisi (line, col).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MvError {
    pub line: usize,
    pub col: usize,
    pub msg: String,
}

impl MvError {
    pub fn new(line: usize, col: usize, msg: String) -> Self {
        MvError { line, col, msg }
    }

    /// Format ringkas `line:col: pesan`. Sejak F11 semua error (lexer, parser,
    /// DAN type-check E2001–E2007) membawa posisi — format ini selalu
    /// menampilkan `line:col:`.
    pub fn format(&self) -> String {
        if self.line == 0 && self.col == 0 {
            self.msg.clone()
        } else {
            format!("{}:{}: {}", self.line, self.col, self.msg)
        }
    }
}

impl fmt::Display for MvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format())
    }
}

impl std::error::Error for MvError {}

/// Format error dengan snippet source (F11): ringkasan `path: line:col: msg`
/// + baris sumber + caret menunjuk posisi persis (gaya rustc). `line`/`col`
/// 1-based; 0 berarti tanpa posisi (tidak tampil snippet). Dipakai `mgen`
/// (src/tools/gen.rs) dan `run` (src/main.rs) untuk UX error yang konsisten.
///
/// Batasan (sama seperti `raw_slice` di parser): `col` lexer dihitung per
/// CHAR sedangkan padding caret memakai spasi — baris yang memuat TAB atau
/// karakter multi-byte (non-ASCII) SEBELUM titik error membuat caret
/// meleset. Praktis tidak terjadi di .mv (ASCII + spasi); dibiarkan agar
/// sederhana.
pub fn format_error(path: &str, src: &str, e: &MvError) -> String {
    let mut out = format!("{path}: {}", e.format());
    if e.line > 0 && e.col > 0 {
        if let Some(line) = src.lines().nth(e.line - 1) {
            out.push_str(&format!("\n  --> {path}:{}:{}", e.line, e.col));
            out.push_str("\n   |");
            out.push_str(&format!("\n{:>4} | {}", e.line, line));
            // caret: col 1-based → (col-1) spasi; tab tidak diganti (jarang
            // dipakai di .mv; offset tetap 1 kolom per karakter source).
            let pad = " ".repeat(e.col.saturating_sub(1));
            out.push_str(&format!("\n     | {pad}^"));
        }
    }
    out
}

/// Hasil transpile satu file `.mv`.
#[derive(Debug, Clone)]
pub struct TranspileResult {
    /// Konten `.sv` (module/program/class/function/task)
    pub sv: String,
    /// Konten `.svh` (package/typedef/interface + include guard)
    pub svh: String,
}

/// Transpile source `.mv` → `.sv` + `.svh`.
/// `base` = nama file tanpa ekstensi (mis. `counter` dari `counter.mv`).
/// Type-check dijalankan SEBELUM emisi — error E2001–E2007 muncul di level
/// `.mv`, bukan di SV hasil generate (MARIA-HDL.md §9, prinsip desain #4).
pub fn transpile(src: &str, base: &str) -> Result<TranspileResult, MvError> {
    let file = parser::parse(src)?;
    check::check(&file)?;
    generate_from(&file, base, &[])
}

/// Transpile TANPA type-check (escape hatch `mgen --no-check` — untuk kode
/// yang memakai konstruk eksternal yang belum dipahami checker).
pub fn transpile_no_check(src: &str, base: &str) -> Result<TranspileResult, MvError> {
    let file = parser::parse(src)?;
    generate_from(&file, base, &[])
}

/// Transpile BEBERAPA file `.mv` sekaligus dengan KONTEKS GABUNGAN (F9):
/// tipe/package/konstanta dari semua file terlihat oleh semua file, sehingga
/// `use pkg::*` antar-file (`types.mv` → `counter.mv`) lolos type-check.
///
/// `items` = pasangan (sumber, base). Hasil sejajar dengan `items`. Error
/// pertama di-return bersama indeks item asalnya — pemanggil menyertakan
/// path-nya dalam pesan error.
pub fn transpile_many(items: &[(String, String)]) -> Result<Vec<TranspileResult>, (usize, MvError)> {
    let files = parse_all(items)?;
    let refs: Vec<&ast::MvFile> = files.iter().collect();
    check::check_many(&refs).map_err(|(i, e)| (i, e))?;
    generate_all(items, &files)
}

/// `transpile_many` tanpa type-check (padanan `--no-check` untuk batch).
pub fn transpile_many_no_check(
    items: &[(String, String)],
) -> Result<Vec<TranspileResult>, (usize, MvError)> {
    let files = parse_all(items)?;
    generate_all(items, &files)
}

fn parse_all(items: &[(String, String)]) -> Result<Vec<ast::MvFile>, (usize, MvError)> {
    let mut files = Vec::with_capacity(items.len());
    for (i, (src, _)) in items.iter().enumerate() {
        let f = parser::parse(src).map_err(|e| (i, e))?;
        files.push(f);
    }
    Ok(files)
}

fn generate_all(
    items: &[(String, String)],
    files: &[ast::MvFile],
) -> Result<Vec<TranspileResult>, (usize, MvError)> {
    // F26 fix review: nama interface dari SEMUA file (konteks gabungan) —
    // port module bertipe interface yang didefinisikan di file lain tetap
    // di-emit tanpa arah (konsisten dgn check_many). Definisi interface
    // sendiri tetap keluar hanya di .svh file asalnya.
    let all_ifaces: Vec<&str> = files
        .iter()
        .flat_map(|f| f.interfaces.iter().map(|i| i.name.as_str()))
        .collect();
    let mut out = Vec::with_capacity(items.len());
    for (i, (_, base)) in items.iter().enumerate() {
        let r = generate_from(&files[i], base, &all_ifaces).map_err(|e| (i, e))?;
        out.push(r);
    }
    Ok(out)
}

fn generate_from(file: &MvFile, base: &str, iface_names: &[&str]) -> Result<TranspileResult, MvError> {
    let out = codegen::generate_with_ifaces(file, base, iface_names);
    Ok(TranspileResult { sv: out.sv, svh: out.svh })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transpile_counter_roundtrip() {
        let src = r#"
module counter #(WIDTH = 8) {
    in  clk, rst_n : bit
    in  enable     : bit
    out count      : logic[WIDTH-1:0]

    seq(clk, rst_n) {
        if (!rst_n) {
            count <= '0
        } else if (enable) {
            count <= count + 1
        }
    }
}
"#;
        let r = transpile(src, "counter").expect("transpile");
        assert!(r.sv.contains("module counter"));
        // Tanpa package/typedef → `.svh` kosong & `.sv` berdiri sendiri
        // (tidak ada baris `include) — fix: svh hanya digenerate jika perlu.
        assert!(r.svh.is_empty(), "svh harus kosong: {}", r.svh);
        assert!(!r.sv.contains("`include"), "sv tidak boleh include svh");
    }

    #[test]
    fn transpile_error_position() {
        let err = transpile("module {", "m").unwrap_err();
        assert_eq!(err.line, 1);
        assert!(err.msg.contains("identifier"));
    }

    #[test]
    fn f11_format_error_shows_snippet_and_caret() {
        // F11: error type-check kini berposisi; format_error menampilkan
        // baris sumber + caret menunjuk kolom persis (gaya rustc).
        let src = "module m {\n    in clk : bit\n    out y : bit\n    comb { y = foo }\n}\n";
        let err = transpile(src, "m").unwrap_err();
        assert_eq!(err.line, 4);
        let rendered = format_error("counter.mv", src, &err);
        assert!(rendered.contains("counter.mv: 4:16: [E2001]"), "got: {rendered}");
        assert!(rendered.contains("comb { y = foo }"), "got: {rendered}");
        assert!(rendered.contains("^"), "caret harus ada: {rendered}");
        // Caret menunjuk kolom `foo` (16): baris caret = `     | ` (7 char)
        // + (col-1)=15 spasi + '^' → index 7+15=22.
        let caret_line = rendered.lines().find(|l| l.contains('^')).unwrap();
        assert_eq!(caret_line.find('^').unwrap(), 22, "caret col: {rendered}");
    }

    #[test]
    fn transpile_error_unclosed() {
        let err = transpile("module m {\n    in clk : bit\n", "m").unwrap_err();
        assert!(err.msg.contains("tidak ditutup"));
    }

    #[test]
    fn transpile_many_cross_file() {
        // F9: `types.mv` + `counter.mv` di-transpile bersama — package dari
        // file pertama terlihat oleh file kedua (konteks gabungan).
        let items = vec![
            (
                "package types_pkg {\n type Addr = logic[15:0]\n enum State { IDLE, RUN }\n}\nmodule types_dummy {\n in clk : bit\n}\n".to_string(),
                "types".to_string(),
            ),
            (
                "module counter {\n use types_pkg::*\n in clk, rst_n : bit\n out addr : Addr\n out st : State\n seq(clk, rst_n) {\n if (!rst_n) {\n addr <= '0\n st <= IDLE\n } else {\n addr <= addr + 1\n st <= RUN\n }\n }\n}\n".to_string(),
                "counter".to_string(),
            ),
        ];
        let results = transpile_many(&items).expect("transpile batch lintas-file");
        // counter.sv memakai tipe dari package (bukan typedef lokal)
        assert!(results[1].sv.contains("module counter"));
        assert!(results[1].sv.contains("import types_pkg::*;"));
        // types.svh berisi package types_pkg
        assert!(results[0].svh.contains("package types_pkg;"));
        assert!(results[0].svh.contains("typedef logic [15:0] Addr;"));
    }

    #[test]
    fn transpile_many_solo_still_errors() {
        // Satu file yang memakai tipe dari file lain (tanpa file definisinya)
        // tetap error E2005 — konsisten dengan perilaku per-file.
        let items = vec![(
            "module counter {\n use types_pkg::*\n in clk : bit\n out addr : Addr\n comb { addr = 1 }\n}\n".to_string(),
            "counter".to_string(),
        )];
        let (idx, e) = transpile_many(&items).unwrap_err();
        assert_eq!(idx, 0);
        assert!(e.msg.contains("E2005"), "msg: {}", e.msg);
    }

    #[test]
    fn transpile_package_svh() {
        let src = r#"
package pkt {
    type Addr = logic[15:0]
    enum State { IDLE, RUN }
}
module top {
    use pkt::*
    in clk : bit
    out a  : Addr
    out s  : State
}
"#;
        let r = transpile(src, "top").unwrap();
        assert!(r.svh.contains("package pkt;"));
        assert!(r.svh.contains("typedef logic [15:0] Addr;"));
        assert!(r.sv.contains("`include \"top.svh\""));
        assert!(r.sv.contains("import pkt::*;"));
    }
}
