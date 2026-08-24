//! Semantic highlighter SystemVerilog → `LayoutJob` egui.
//!
//! Melampaui syntax-highlighting biasa: identifier diklasifikasikan menurut
//! peran semantiknya (bukan sekadar keyword/type), persis filosofi desain
//! Maria:
//!
//! - `module` → **biru** (nama module setelah keyword)
//! - `interface` → **ungu**
//! - `package` → **cyan** (termasuk nama package setelah `import`)
//! - `parameter`/`localparam` → **oranye**
//! - signal (identifier biasa) → **putih**
//! - clock (`clk`, `*_clk`, `*clock*`) → **kuning**
//! - reset (`rst`, `*_rst*`, `*reset*`) → **merah**
//! - macro (`` `FOO ``) → **abu**
//! - `typedef` name → **hijau**
//! - `enum` → **teal**
//!
//! Dipakai editor kustom di `panels/editor.rs` via `TextEdit::layouter`.
//! (egui_code_editor tidak bisa disuntik layouter kustom, jadi lexer ini
//! menggantikannya sepenuhnya.)

use eframe::egui;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};

/// Ukuran font editor — harus sinkron dengan `TextEdit::font` di editor.
pub const FONT_SIZE: f32 = 13.0;

/// Kategori semantik token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemKind {
    Comment,
    String,
    Number,
    /// `` `FOO `` — macro preprocessing (abu).
    Macro,
    Keyword,
    Type,
    /// `$display`, `$clog2`, ...
    SysFunc,
    /// Nama module (biru).
    Module,
    /// Nama interface (ungu).
    Interface,
    /// Nama package (cyan).
    Package,
    /// Nama parameter/localparam (oranye).
    Parameter,
    /// Signal — identifier biasa (putih).
    Signal,
    /// Clock — nama mengandung clk/clock (kuning).
    Clock,
    /// Reset — nama mengandung rst/reset (merah).
    Reset,
    /// Nama typedef (hijau).
    Typedef,
    /// Keyword `enum` + anggotanya (teal).
    Enum,
    /// Punctuation/operator (abu muda).
    Punct,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// Warna per kategori (palet dark tenang, sesuai desain Maria).
pub fn color(kind: SemKind) -> Color32 {
    match kind {
        SemKind::Comment => rgb(0x6a, 0x73, 0x7d),
        SemKind::String => rgb(0xce, 0x91, 0x78),
        SemKind::Number => rgb(0xb5, 0xce, 0xa8),
        SemKind::Macro => rgb(0x9d, 0xa5, 0xb4), // abu
        SemKind::Keyword => rgb(0x56, 0x9c, 0xd6),
        SemKind::Type => rgb(0x4e, 0xc9, 0xb0),
        SemKind::SysFunc => rgb(0x56, 0xb6, 0xc2),
        SemKind::Module => rgb(0x61, 0xaf, 0xef),    // biru
        SemKind::Interface => rgb(0xc6, 0x78, 0xdd), // ungu
        SemKind::Package => rgb(0x56, 0xb6, 0xc2),   // cyan
        SemKind::Parameter => rgb(0xd1, 0x9a, 0x66), // oranye
        SemKind::Signal => rgb(0xe0, 0xe0, 0xe0),    // putih
        SemKind::Clock => rgb(0xe5, 0xc0, 0x7b),     // kuning
        SemKind::Reset => rgb(0xe0, 0x6c, 0x75),     // merah
        SemKind::Typedef => rgb(0x98, 0xc3, 0x79),   // hijau
        SemKind::Enum => rgb(0x0f, 0xb9, 0xb1),      // teal
        SemKind::Punct => rgb(0x80, 0x84, 0x8e),
    }
}

/// Keyword struktural + kontrol SystemVerilog.
const SV_KEYWORDS: &[&str] = &[
    "module",
    "endmodule",
    "interface",
    "endinterface",
    "package",
    "endpackage",
    "program",
    "endprogram",
    "class",
    "endclass",
    "function",
    "endfunction",
    "task",
    "endtask",
    "property",
    "endproperty",
    "sequence",
    "endsequence",
    "clocking",
    "endclocking",
    "checker",
    "endchecker",
    "primitive",
    "endprimitive",
    "config",
    "endconfig",
    "generate",
    "endgenerate",
    "specify",
    "endspecify",
    "input",
    "output",
    "inout",
    "ref",
    "import",
    "export",
    "bind",
    "modport",
    "always",
    "always_comb",
    "always_ff",
    "always_latch",
    "initial",
    "final",
    "assign",
    "deassign",
    "force",
    "release",
    "if",
    "else",
    "case",
    "casex",
    "casez",
    "endcase",
    "for",
    "while",
    "repeat",
    "forever",
    "do",
    "begin",
    "end",
    "fork",
    "join",
    "join_any",
    "join_none",
    "disable",
    "wait",
    "assert",
    "assume",
    "cover",
    "rand",
    "randc",
    "constraint",
    "solve",
    "before",
    "dist",
    "unique",
    "priority",
    "new",
    "this",
    "super",
    "extends",
    "implements",
    "default",
    "global",
    "defparam",
    "signed",
    "unsigned",
    "genvar",
    "automatic",
    "static",
    "virtual",
    "pure",
    "typedef",
    "enum",
    "struct",
    "union",
    "return",
    "break",
    "continue",
    "void",
    "local",
    "extern",
    "protected",
    "var",
    "parameter",
    "localparam",
    "posedge",
    "negedge",
    "edge",
    "iff",
    "inside",
    "with",
    "within",
    "first_match",
    "foreach",
    "intersect",
    "throughout",
    "matches",
];

/// Tipe data SystemVerilog.
const SV_TYPES: &[&str] = &[
    "bit", "logic", "reg", "wire", "byte", "int", "integer", "longint", "shortint", "time", "real",
    "realtime", "string", "event",
];

fn is_keyword(w: &str) -> bool {
    SV_KEYWORDS.contains(&w)
}

fn is_type(w: &str) -> bool {
    SV_TYPES.contains(&w)
}

/// Satu token hasil lexing.
#[derive(Debug, Clone, Copy)]
struct Tok {
    start: usize,
    end: usize,
    kind: SemKind,
}

/// Lex text menjadi token. Identifier yang belum jelas perannya diberi
/// `SemKind::Signal` sementara — diklasifikasi ulang oleh `classify`.
fn lex(text: &str) -> Vec<Tok> {
    let b = text.as_bytes();
    let n = b.len();
    let mut toks: Vec<Tok> = Vec::new();
    let mut i = 0usize;

    while i < n {
        let c = b[i] as char;

        // Byte non-ASCII (BOM UTF-8 di awal file, karakter unicode di luar
        // string/komentar): lompati seluruh karakter beserta continuation
        // bytes-nya, agar slicing `&text[a..b]` di bawah tidak pernah pecah di
        // tengah char UTF-8 (kalau sampai pecah → panic di Rust).
        if !b[i].is_ascii() {
            i += 1;
            while i < n && (b[i] & 0xC0) == 0x80 {
                i += 1;
            }
            continue;
        }

        // ── Whitespace ──
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // ── Line comment ──
        if c == '/' && i + 1 < n && b[i + 1] == b'/' {
            let s = i;
            while i < n && b[i] != b'\n' {
                i += 1;
            }
            toks.push(Tok {
                start: s,
                end: i,
                kind: SemKind::Comment,
            });
            continue;
        }

        // ── Block comment ──
        if c == '/' && i + 1 < n && b[i + 1] == b'*' {
            let s = i;
            i += 2;
            while i + 1 < n && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(n);
            toks.push(Tok {
                start: s,
                end: i,
                kind: SemKind::Comment,
            });
            continue;
        }

        // ── String ──
        if c == '"' {
            let s = i;
            i += 1;
            while i < n && b[i] != b'"' {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(n);
            toks.push(Tok {
                start: s,
                end: i,
                kind: SemKind::String,
            });
            continue;
        }

        // ── Macro `FOO ──
        if c == '`' {
            let s = i;
            i += 1;
            while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            toks.push(Tok {
                start: s,
                end: i,
                kind: SemKind::Macro,
            });
            continue;
        }

        // ── System function $display ──
        if c == '$' {
            let s = i;
            i += 1;
            while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            toks.push(Tok {
                start: s,
                end: i,
                kind: SemKind::SysFunc,
            });
            continue;
        }

        // ── Number (termasuk sized literal: 32'hFF, 4'b1010, 1.5, 1e9) ──
        if c.is_ascii_digit() {
            let s = i;
            while i < n
                && (b[i].is_ascii_alphanumeric() || b[i] == b'\'' || b[i] == b'_' || b[i] == b'.')
            {
                i += 1;
            }
            toks.push(Tok {
                start: s,
                end: i,
                kind: SemKind::Number,
            });
            continue;
        }

        // ── Identifier / keyword ──
        if c.is_ascii_alphabetic() || c == '_' {
            let s = i;
            while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'$') {
                i += 1;
            }
            let word = &text[s..i];
            let kind = if is_keyword(word) {
                SemKind::Keyword
            } else if is_type(word) {
                SemKind::Type
            } else {
                SemKind::Signal // sementara; klasifikasi di `classify`
            };
            toks.push(Tok {
                start: s,
                end: i,
                kind,
            });
            continue;
        }

        // ── Punctuation (1 byte) ──
        let s = i;
        i += 1;
        toks.push(Tok {
            start: s,
            end: i,
            kind: SemKind::Punct,
        });
    }

    toks
}

/// Klasifikasi semantik identifier:
/// - nama module/interface/package/parameter berdasar keyword sebelumnya
/// - nama typedef: identifier terakhir pada brace-depth 0 sebelum `;`
///   (menang atas in_enum, jadi `typedef enum {...} name_t;` memberi name_t
///   warna hijau, bukan teal)
/// - anggota enum + nama tipe enum → teal sampai `;`
/// - clock/reset berdasar pola nama (clk/clock, rst/reset)
fn classify(toks: &mut [Tok], text: &str) {
    let n = toks.len();
    let mut pending: Option<SemKind> = None; // module/interface/package/parameter
    let mut in_typedef = false;
    let mut typedef_candidate: Option<usize> = None;
    let mut brace = 0i32;
    let mut in_enum = false;

    for i in 0..n {
        let word = &text[toks[i].start..toks[i].end];
        match toks[i].kind {
            SemKind::Keyword => match word {
                "module" => pending = Some(SemKind::Module),
                "interface" => pending = Some(SemKind::Interface),
                "package" => pending = Some(SemKind::Package),
                "parameter" | "localparam" => pending = Some(SemKind::Parameter),
                "enum" => {
                    // Anggota enum (di dalam `{...}`) dan nama tipe enum
                    // diwarnai teal sampai `;`.
                    in_enum = true;
                    pending = None;
                }
                "typedef" => {
                    in_typedef = true;
                    typedef_candidate = None;
                    pending = None;
                }
                _ => {
                    // Keyword lain (always, if, ...) tidak menandai deklarasi.
                    pending = None;
                }
            },
            SemKind::Type => {
                // `parameter int WIDTH` — `int` tidak mengonsumsi pending.
            }
            SemKind::Signal => match pending {
                Some(SemKind::Parameter) => {
                    // Parameter bisa banyak (`parameter A = 1, B = 2;`) —
                    // pending dipertahankan sampai `;`.
                    toks[i].kind = SemKind::Parameter;
                }
                Some(p) => {
                    toks[i].kind = p;
                    pending = None;
                }
                None => {
                    if in_typedef && brace == 0 {
                        // Kandidat nama typedef: identifier terakhir pada
                        // depth 0 sebelum `;`.
                        typedef_candidate = Some(i);
                        toks[i].kind = SemKind::Signal;
                    } else if in_enum {
                        toks[i].kind = SemKind::Enum;
                    } else {
                        // Heuristik clock/reset dari nama signal.
                        let lower = word.to_ascii_lowercase();
                        if lower.contains("clk") || lower.contains("clock") {
                            toks[i].kind = SemKind::Clock;
                        } else if lower.contains("rst") || lower.contains("reset") {
                            toks[i].kind = SemKind::Reset;
                        } else {
                            toks[i].kind = SemKind::Signal;
                        }
                    }
                }
            },
            SemKind::Punct => match word {
                "{" => brace += 1,
                "}" => brace -= 1,
                ";" => {
                    if in_typedef && brace == 0 {
                        if let Some(idx) = typedef_candidate {
                            toks[idx].kind = SemKind::Typedef;
                        }
                        in_typedef = false;
                    }
                    pending = None;
                    in_enum = false;
                }
                _ => {}
            },
            _ => {}
        }
    }
}

/// Bangun `LayoutJob` berwarna dari text sumber.
///
/// Gap (whitespace) di antara token diisi warna signal (putih) agar baris
/// tetap terbaca; token ditambahkan per-section dengan warna kategorinya.
pub fn highlight(text: &str) -> LayoutJob {
    let mut toks = lex(text);
    classify(&mut toks, text);

    let mut job = LayoutJob::default();
    let font = FontId::monospace(FONT_SIZE);
    let mut pos = 0usize;

    for t in &toks {
        if t.start > pos {
            job.append(
                &text[pos..t.start],
                0.0,
                TextFormat::simple(font.clone(), color(SemKind::Punct)),
            );
        }
        job.append(
            &text[t.start..t.end],
            0.0,
            TextFormat::simple(font.clone(), color(t.kind)),
        );
        pos = t.end;
    }
    if pos < text.len() {
        job.append(
            &text[pos..],
            0.0,
            TextFormat::simple(font, color(SemKind::Punct)),
        );
    }
    job
}

/// Identifier + kategori semantik pada posisi byte tertentu (dipakai Hover
/// tooltip editor). Mengembalikan (nama, kategori) bila posisi jatuh di dalam
/// token identifier yang relevan; `None` untuk whitespace/punctuation/keyword.
pub fn identifier_at(text: &str, byte_idx: usize) -> Option<(String, SemKind)> {
    if byte_idx >= text.len() {
        return None;
    }
    let mut toks = lex(text);
    classify(&mut toks, text);
    let t = toks
        .iter()
        .find(|t| byte_idx >= t.start && byte_idx < t.end)?;
    match t.kind {
        SemKind::Signal
        | SemKind::Clock
        | SemKind::Reset
        | SemKind::Parameter
        | SemKind::Module
        | SemKind::Interface
        | SemKind::Package
        | SemKind::Typedef
        | SemKind::Enum
        | SemKind::Type => Some((text[t.start..t.end].to_string(), t.kind)),
        _ => None,
    }
}
