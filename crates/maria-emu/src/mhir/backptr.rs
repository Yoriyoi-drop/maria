//! Resolver back-pointer MHIR — EMULATOR.md §4.2.
//!
//! `SignalInfo`/`Process` di `IrDesign` TIDAK membawa posisi source per
//! signal, jadi posisi (baris, kolom) di-resolve dengan mencari token nama
//! di `IrDesign.source_lines` (output preprocessed). Pola yang sama dipakai
//! elaborator (`find_name_in_source` + cache `source_name_loc`); di sini
//! diimplementasikan ringkas + cache per `Symbol` agar satu desain hanya
//! di-scan sekali per nama.

use std::cell::RefCell;
use std::collections::HashMap;

use maria_core::intern::Symbol;

use super::types::BackPointer;

/// Karakter yang membatasi token identifier di SystemVerilog.
fn is_delim(c: char) -> bool {
    !(c.is_alphanumeric() || c == '_' || c == '$')
}

/// Cari token `name` di satu baris. Kembalikan kolom 1-based (0 = tidak ada).
fn find_in_line(line: &str, name: &str) -> usize {
    let bytes = line.as_bytes();
    let n = name.as_bytes();
    if n.is_empty() || bytes.len() < n.len() {
        return 0;
    }
    let mut start = 0usize;
    while start + n.len() <= bytes.len() {
        // Temukan kandidat: byte pertama cocok.
        let Some(rel) = bytes[start..].iter().position(|&b| b == n[0]) else {
            break;
        };
        start += rel;
        if bytes[start..].len() < n.len() {
            break;
        }
        let end = start + n.len();
        let before_ok = start == 0 || is_delim(bytes[start - 1] as char);
        let after_ok = end == bytes.len() || is_delim(bytes[end] as char);
        if before_ok && after_ok && &bytes[start..end] == n {
            // Kolom 1-based (hitung karakter, bukan byte — source ASCII dominan).
            return line[..start].chars().count() + 1;
        }
        start += 1;
    }
    0
}

/// Resolver posisi nama di `source_lines`, dengan cache per `Symbol`.
#[derive(Default)]
pub struct SourceLocator {
    cache: RefCell<HashMap<Symbol, BackPointer>>,
}

impl SourceLocator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cari posisi token `name` di source. Kembalikan `(line, col)` 1-based
    /// (0,0 bila tidak ditemukan). Baris dicari dari akhir agar mendapat
    /// **deklarasi/modul terakhir** (definisi yang menang setelah dedup).
    pub fn locate(&self, lines: &[String], name: Symbol) -> BackPointer {
        if let Some(bp) = self.cache.borrow().get(&name) {
            return bp.clone();
        }
        let bp = self.locate_uncached(lines, name.as_str());
        self.cache.borrow_mut().insert(name, bp.clone());
        bp
    }

    fn locate_uncached(&self, lines: &[String], name: &str) -> BackPointer {
        for (idx, line) in lines.iter().enumerate().rev() {
            let col = find_in_line(line, name);
            if col > 0 {
                return BackPointer::known(None, idx + 1, col);
            }
        }
        BackPointer::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_of(src: &str) -> Vec<String> {
        src.lines().map(|l| l.to_string()).collect()
    }

    #[test]
    fn test_find_in_line_exact_token() {
        assert_eq!(find_in_line("module uart (", "uart"), 8);
        assert_eq!(find_in_line("  input logic clk,", "clk"), 15);
        // Bukan token penuh: "uart" tidak cocok dengan "uart_x".
        assert_eq!(find_in_line("uart_x u;", "uart"), 0);
        // Akhir baris tanpa delimiter.
        assert_eq!(find_in_line("output logic tx", "tx"), 14);
    }

    #[test]
    fn test_locate_last_definition_wins() {
        let src = "module uart;\nendmodule\nmodule uart;\nendmodule\n";
        let loc = SourceLocator::new();
        let bp = loc.locate(&lines_of(src), Symbol::intern("uart"));
        assert_eq!(bp.line, 3, "definisi terakhir yang menang (dedup)");
        assert_eq!(bp.col, 8);
    }

    #[test]
    fn test_locate_unknown_returns_default() {
        let loc = SourceLocator::new();
        let bp = loc.locate(&lines_of("module top; endmodule"), Symbol::intern("nope"));
        assert_eq!(bp.line, 0);
        assert_eq!(bp.col, 0);
    }

    #[test]
    fn test_locator_cache() {
        let lines = lines_of("reg [7:0] data;\n");
        let loc = SourceLocator::new();
        let s = Symbol::intern("data");
        let a = loc.locate(&lines, s);
        let b = loc.locate(&lines, s);
        assert_eq!(a, b);
    }
}
