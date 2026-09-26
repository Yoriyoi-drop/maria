//! Atribut `(* ... *)` pada instance — EMULATOR.md §10 (anotasi Mivon
//! `mivon_region` / `mivon_irq`).
//!
//! Tanggung jawab: parser menempatkan atribut ke `ModuleInstance.attrs`
//! (nilai string/angka/ident/bare), tetap melewati atribut sebelum item
//! non-instance tanpa merusak parsing, dan line/col instance TIDAK bergeser
//! (100% akurat — aturan repo).

use mivon_ast::*;
use mivon_parser::lexer::{Lexer, Token};
use mivon_parser::preprocessor::Preprocessor;
use mivon_parser::Parser;

/// Parse source SV mentah (tanpa makro) → Design, pipeline sama dengan
/// `mivon-api::compile_str` tapi berhenti di AST.
fn parse(src: &str) -> Design {
    let mut pp = Preprocessor::new();
    let preprocessed = pp.preprocess(src, None).expect("preprocess");
    let mut lexer = Lexer::new(&preprocessed);
    let mut tokens = Vec::new();
    loop {
        let (tok, line, col) = lexer.next_token();
        if tok == Token::Eof {
            break;
        }
        tokens.push((tok, line, col));
    }
    let file_line_map = lexer.file_line_map.clone();
    let first_source = if file_line_map.is_empty() {
        "<test>".to_string()
    } else {
        file_line_map[0].2.clone()
    };
    let mut parser = Parser::new(tokens, &first_source)
        .with_source_lines(&preprocessed)
        .with_file_line_map(file_line_map);
    parser.parse_design().expect("parse")
}

/// Cari instance bernama `name` di seluruh module (urut sumber).
fn find_instance<'a>(d: &'a Design, name: &str) -> &'a ModuleInstance {
    d.modules
        .iter()
        .flat_map(|m| m.items.iter())
        .find_map(|it| match it {
            ModuleItem::Instance(i) if i.instance_name.as_str() == name => Some(i),
            _ => None,
        })
        .unwrap_or_else(|| panic!("instance '{}' tidak ditemukan", name))
}

/// Nilai atribut `key` pada instance.
fn attr<'a>(i: &'a ModuleInstance, key: &str) -> Option<&'a str> {
    i.attrs
        .iter()
        .find(|a| a.key.as_str() == key)
        .and_then(|a| a.value.as_deref())
}

const SOC: &str = r#"
module uart (input logic clk, input logic rst_n, output logic tx);
  assign tx = 1'b0;
endmodule

module soc (input logic clk, input logic rst_n, output logic tx);
  (* mivon_region = "mmio", base = "0x10000000", size = "0x1000" *)
  (* mivon_irq = "5" *)
  uart u_uart (.clk(clk), .rst_n(rst_n), .tx(tx));
  uart u_plain (.clk(clk), .rst_n(rst_n), .tx());
endmodule
"#;

#[test]
fn test_instance_attr_region_and_irq() {
    let d = parse(SOC);
    let u = find_instance(&d, "u_uart");
    // Dua blok atribut berurutan digabung (urutan tak peduli — lookup by key).
    assert_eq!(u.attrs.len(), 4, "attrs={:?}", u.attrs);
    assert_eq!(attr(u, "mivon_region"), Some("mmio"));
    assert_eq!(attr(u, "base"), Some("0x10000000"));
    assert_eq!(attr(u, "size"), Some("0x1000"));
    assert_eq!(attr(u, "mivon_irq"), Some("5"));
}

#[test]
fn test_instance_without_attr_is_empty_and_pos_accurate() {
    let d = parse(SOC);
    let plain = find_instance(&d, "u_plain");
    assert!(plain.attrs.is_empty(), "attrs={:?}", plain.attrs);
    // Line instance TANPA atribut = baris source-nya sendiri (1-based).
    let line = SOC.lines().position(|l| l.contains("u_plain")).unwrap() + 1;
    assert_eq!(plain.line, line, "line u_plain akurat");
    // Kolom = posisi token module name (`uart`) di baris tsb (1-based).
    let text = SOC.lines().nth(line - 1).unwrap();
    assert_eq!(plain.col, text.find("uart").unwrap() + 1, "col akurat");

    // Instance beratribut: line/col menunjuk token module name SESUDAH
    // atribut (bukan `(*`).
    let u = find_instance(&d, "u_uart");
    let uline = SOC.lines().position(|l| l.contains("u_uart ")).unwrap() + 1;
    assert_eq!(u.line, uline, "line u_uart akurat (setelah atribut)");
    let utext = SOC.lines().nth(uline - 1).unwrap();
    assert_eq!(u.col, utext.find("uart").unwrap() + 1, "col u_uart akurat");
}

#[test]
fn test_attr_before_non_instance_item_still_parses() {
    // Perilaku lama dipertahankan: atribut sebelum item non-instance
    // (declaration / always) dilewati tanpa error; instance sesudahnya
    // tetap ter-parse dan atribut ikut menempel.
    let src = r#"
module top (input logic clk, output logic q);
  (* syn_preserve *) logic tmp;
  (* some.attr = 1, foo = "bar" *)
  always_ff @(posedge clk) q <= tmp;
  (* mivon_irq = "3" *)
  sub u_sub (.clk(clk), .q(q));
endmodule

module sub (input logic clk, output logic q);
  always_ff @(posedge clk) q <= 1'b1;
endmodule
"#;
    let d = parse(src);
    let sub = find_instance(&d, "u_sub");
    assert_eq!(attr(sub, "mivon_irq"), Some("3"));
    // Atribut non-instance TIDAK bocor ke instance lain.
    assert_eq!(sub.attrs.len(), 1, "attrs={:?}", sub.attrs);
}

#[test]
fn test_attr_bare_value_and_expr_noise() {
    // Bentuk nilai lain: bare flag (tanpa `=`), angka polos, dan ekspresi
    // yang tidak dikenal (dilewati seperti skip_attribute lama) — parsing
    // tetap utuh dan atribut Mivon tetap terbaca.
    let src = r#"
module top (input logic clk, output logic q);
  (* syn_preserve *)
  (* mivon_irq = 7, width = 8, vendor = acme *)
  (* dont_touch = (a && b) *)
  sub u_sub (.clk(clk), .q(q));
endmodule

module sub (input logic clk, output logic q);
  assign q = clk;
endmodule
"#;
    let d = parse(src);
    let sub = find_instance(&d, "u_sub");
    assert_eq!(attr(sub, "syn_preserve"), None, "bare flag = tanpa nilai");
    assert!(sub.attrs.iter().any(|a| a.key.as_str() == "syn_preserve"));
    assert_eq!(attr(sub, "mivon_irq"), Some("7"), "angka polos");
    assert_eq!(attr(sub, "width"), Some("8"));
    assert_eq!(attr(sub, "vendor"), Some("acme"));
    // `dont_touch = (a && b)` — nilai ekspresi tidak dikenal → key tetap ada
    // (tanpa nilai), parser tidak error.
    assert!(sub.attrs.iter().any(|a| a.key.as_str() == "dont_touch"));
}
