//! SIMD-accelerated byte-level lexer untuk SystemVerilog.
//!
//! Beroperasi pada `&[u8]` (mmap'd content) alih-alih `Vec<char>`.
//! Menghasilkan token yang sama persis dengan legacy lexer.
//! Menggunakan SIMD untuk karakter classification di hot path.

use crate::parser::lexer::{Lexer, Token};
use super::simd::{self, SimdLevel};

/// SIMD lexer — byte-level, SIMD-accelerated.
/// Produces same `(Token, usize, usize)` as legacy lexer.
pub struct SimdLexer<'a> {
    data: &'a [u8],
    pos: usize,
    line: usize,
    col: usize,
    simd_level: SimdLevel,
}

impl<'a> SimdLexer<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        SimdLexer {
            data,
            pos: 0,
            line: 1,
            col: 1,
            simd_level: simd::detect_simd_level(),
        }
    }

    fn advance(&mut self, n: usize) {
        for &b in &self.data[self.pos..self.pos + n] {
            if b == b'\n' { self.line += 1; self.col = 1; }
            else { self.col += 1; }
        }
        self.pos += n;
    }

    /// Skip whitespace using SIMD where possible.
    fn skip_ws(&mut self) {
        let skipped = simd::skip_whitespace(self.data, self.pos, self.simd_level);
        if skipped > 0 {
            self.advance(skipped);
        }
    }

    /// Get next token from input.
    pub fn next_token(&mut self) -> (Token, usize, usize) {
        loop {
            self.skip_ws();
            if self.pos >= self.data.len() {
                return (Token::Eof, self.line, self.col);
            }

            let start = self.pos;
            let (line, col) = (self.line, self.col);
            let b = self.data[start];

            // Line comment
            if b == b'/' && start + 1 < self.data.len() && self.data[start + 1] == b'/' {
                let n = simd::skip_line_comment(self.data, start, self.simd_level);
                self.advance(n);
                continue;
            }

            // Block comment
            if b == b'/' && start + 1 < self.data.len() && self.data[start + 1] == b'*' {
                let n = simd::skip_block_comment(self.data, start, self.simd_level);
                self.advance(n);
                continue;
            }

            // String literal
            if b == b'"' {
                return self.lex_string_literal(line, col);
            }

            // Fill literal '0, '1, 'x, 'z
            if b == b'\'' {
                return self.lex_fill_literal(line, col);
            }

            // Number literal
            if b.is_ascii_digit() {
                return self.lex_number(line, col);
            }

            // Identifier or keyword
            if b.is_ascii_alphabetic() || b == b'_' {
                return self.lex_ident(line, col);
            }

            // System function ($) — lex as Dollar + next call gets Ident
            if b == b'$' {
                self.advance(1);
                return (Token::Dollar, line, col);
            }

            // Operators (single-char)
            return match b {
                b'+' => self.lex_plus_ops(line, col),
                b'-' => self.op2(line, col, Token::Minus, b'-', Token::Decrement),
                b'*' => self.op2(line, col, Token::Star, b'*', Token::StarStar),
                b'/' => self.op1(line, col, Token::Slash),
                b'%' => self.op1(line, col, Token::Percent),
                b'=' => self.lex_assign_or_eq(line, col),
                b'!' => self.lex_not_or_neq(line, col),
                b'<' => self.lex_lt_or_shift(line, col),
                b'>' => self.lex_gt_or_shift(line, col),
                b'&' => self.op2(line, col, Token::Amp, b'&', Token::AmpAmp),
                b'|' => self.op2(line, col, Token::Pipe, b'|', Token::PipePipe),
                b'~' => self.lex_tilde_ops(line, col),
                b'^' => self.lex_caret_ops(line, col),
                b'(' => self.op1(line, col, Token::LParen),
                b')' => self.op1(line, col, Token::RParen),
                b'{' => self.op1(line, col, Token::LBrace),
                b'}' => self.op1(line, col, Token::RBrace),
                b'[' => self.op1(line, col, Token::LBrack),
                b']' => self.op1(line, col, Token::RBrack),
                b';' => self.op1(line, col, Token::Semi),
                b',' => self.op1(line, col, Token::Comma),
                b'.' => self.op1(line, col, Token::Dot),
                b':' => self.lex_colon_or_scope(line, col),
                b'#' => self.op1(line, col, Token::Hash),
                b'@' => self.op1(line, col, Token::At),
                b'?' => self.op1(line, col, Token::Question),
                b'`' => {
                    self.advance(1);
                    (Token::Error("unexpected `".into()), line, col)
                }
                _ => {
                    self.advance(1);
                    (Token::Error(format!("unexpected char '{}'", b as char)), line, col)
                }
            };
        }
    }

    // ─── Operator helpers ───

    fn lex_plus_ops(&mut self, line: usize, col: usize) -> (Token, usize, usize) {
        self.advance(1);
        if self.pos < self.data.len() {
            match self.data[self.pos] {
                b'+' => { self.advance(1); return (Token::Increment, line, col); }
                b'=' => { self.advance(1); return (Token::PlusAssign, line, col); }
                b':' => { self.advance(1); return (Token::PlusColon, line, col); }
                _ => {}
            }
        }
        (Token::Plus, line, col)
    }

    fn op1(&mut self, line: usize, col: usize, tok: Token) -> (Token, usize, usize) {
        self.advance(1);
        (tok, line, col)
    }

    fn op2(&mut self, line: usize, col: usize, single: Token, next: u8, compound: Token) -> (Token, usize, usize) {
        self.advance(1);
        if self.pos < self.data.len() && self.data[self.pos] == next {
            self.advance(1);
            (compound, line, col)
        } else {
            (single, line, col)
        }
    }

    fn lex_assign_or_eq(&mut self, line: usize, col: usize) -> (Token, usize, usize) {
        self.advance(1);
        if self.pos < self.data.len() {
            match self.data[self.pos] {
                b'=' => {
                    self.advance(1);
                    if self.pos < self.data.len() && self.data[self.pos] == b'=' {
                        self.advance(1);
                        return (Token::Equiv, line, col);
                    }
                    return (Token::Eq, line, col);
                }
                b'?' => { self.advance(1); return (Token::CaseEq, line, col); }
                b'*' => { self.advance(1); return (Token::WildcardEq, line, col); }
                _ => { /* nothing */ }
            }
        }
        (Token::BlockingAssign, line, col)
    }

    fn lex_not_or_neq(&mut self, line: usize, col: usize) -> (Token, usize, usize) {
        self.advance(1);
        if self.pos < self.data.len() && self.data[self.pos] == b'=' {
            self.advance(1);
            if self.pos < self.data.len() && self.data[self.pos] == b'=' {
                self.advance(1);
                return (Token::NotEquiv, line, col); // !==
            }
            if self.pos < self.data.len() && self.data[self.pos] == b'?' {
                self.advance(1);
                return (Token::CaseNeq, line, col); // !=?
            }
            if self.pos < self.data.len() && self.data[self.pos] == b'*' {
                self.advance(1);
                return (Token::WildcardNeq, line, col); // !=*
            }
            return (Token::Neq, line, col); // !=
        }
        (Token::Not, line, col) // !
    }

    fn lex_lt_or_shift(&mut self, line: usize, col: usize) -> (Token, usize, usize) {
        self.advance(1);
        if self.pos < self.data.len() {
            let c = self.data[self.pos];
            match c {
                b'<' => { self.advance(1); return (Token::Shl, line, col) }
                b'=' => {
                    self.advance(1);
                    // Legacy produces NonBlockingAssign for <=
                    (Token::NonBlockingAssign, line, col)
                }
                _ => (Token::Lt, line, col)
            }
        } else {
            (Token::Lt, line, col)
        }
    }

    fn lex_gt_or_shift(&mut self, line: usize, col: usize) -> (Token, usize, usize) {
        self.advance(1);
        if self.pos < self.data.len() {
            match self.data[self.pos] {
                b'>' => { self.advance(1); return (Token::Shr, line, col) }
                b'=' => { self.advance(1); return (Token::Ge, line, col) }
                _ => {}
            }
        }
        (Token::Gt, line, col)
    }

    fn lex_colon_or_scope(&mut self, line: usize, col: usize) -> (Token, usize, usize) {
        self.advance(1);
        if self.pos < self.data.len() && self.data[self.pos] == b':' {
            self.advance(1);
            (Token::Scope, line, col)
        } else {
            (Token::Colon, line, col)
        }
    }

    fn lex_tilde_ops(&mut self, line: usize, col: usize) -> (Token, usize, usize) {
        self.advance(1);
        if self.pos < self.data.len() {
            match self.data[self.pos] {
                b'&' => { self.advance(1); return (Token::TildeAmp, line, col); }
                b'|' => { self.advance(1); return (Token::TildePipe, line, col); }
                b'^' => { self.advance(1); return (Token::CaretTilde, line, col); }
                _ => {}
            }
        }
        (Token::Tilde, line, col)
    }

    fn lex_caret_ops(&mut self, line: usize, col: usize) -> (Token, usize, usize) {
        self.advance(1);
        if self.pos < self.data.len() && self.data[self.pos] == b'~' {
            self.advance(1);
            return (Token::CaretTilde, line, col);
        }
        if self.pos < self.data.len() && self.data[self.pos] == b'=' {
            self.advance(1);
            return (Token::XorAssign, line, col);
        }
        (Token::Caret, line, col)
    }

    fn lex_fill_literal(&mut self, line: usize, col: usize) -> (Token, usize, usize) {
        self.advance(1);
        if self.pos < self.data.len() {
            let val = match self.data[self.pos] {
                b'0' => crate::ir::LogicVal::Zero,
                b'1' => crate::ir::LogicVal::One,
                b'x' | b'X' => crate::ir::LogicVal::X,
                b'z' | b'Z' => crate::ir::LogicVal::Z,
                _ => { self.advance(1); return (Token::Quote, line, col); }
            };
            self.advance(1);
            (Token::FillLit(val), line, col)
        } else {
            (Token::Quote, line, col)
        }
    }

    fn lex_string_literal(&mut self, line: usize, col: usize) -> (Token, usize, usize) {
        self.advance(1); // skip opening "
        let start = self.pos;
        while self.pos < self.data.len() {
            if self.data[self.pos] == b'"' {
                let raw = &self.data[start..self.pos];
                let s = unsafe { std::str::from_utf8_unchecked(raw) };
                self.advance(1); // skip closing "
                return (Token::StringLit(Symbol::intern(s)), line, col);
            }
            if self.data[self.pos] == b'\\' && self.pos + 1 < self.data.len() {
                self.advance(2);
            } else {
                self.advance(1);
            }
        }
        (Token::Error("unterminated string".into()), line, col)
    }

    fn lex_number(&mut self, line: usize, col: usize) -> (Token, usize, usize) {
        let end = self.pos + simd::scan_number_scalar(self.data, self.pos);
        let substr = unsafe { std::str::from_utf8_unchecked(&self.data[self.pos..end]) };
        self.advance(end - self.pos);

        let s = substr;
        // Check for real number
        if s.contains('.') && !s.contains('\'') {
            // Simple real number — skip scientific notation for speed
            let clean = s.replace('_', "");
            return (Token::RealNum(clean), line, col);
        }

        let (value, base, width, is_signed) = if let Some(apos) = s.find('\'') {
            let w: usize = s[..apos].parse().unwrap_or(0);
            let rest = s[apos+1..].to_lowercase();
            let rest2 = rest.trim_start_matches(|c: char| c == 's' || c == 'S');
            let base_char = rest2.chars().next().unwrap_or('d');
            // Legacy stores original digit string (e.g., "FF" for hex), not decimal
            let digits_str = &rest2[1..].replace('_', "");
            let base_val: u8 = match base_char { 'h' => 16, 'o' => 8, 'b' => 2, _ => 10 };
            // Store the original digit representation as the legacy does
            let value_str = digits_str.to_uppercase();
            // Determine signedness
            let signed = rest2 != rest;
            (value_str, Some(base_val), Some(w), signed)
        } else {
            let clean = s.replace('_', "");
            (clean.clone(), None, None, false)
        };

        (Token::Number { value, base, width, is_signed }, line, col)
    }

    fn lex_ident(&mut self, line: usize, col: usize) -> (Token, usize, usize) {
        let start = self.pos;
        let n = simd::scan_identifier(self.data, start, self.simd_level);
        self.advance(n);
        let s = unsafe { std::str::from_utf8_unchecked(&self.data[start..start + n]) };
        (keyword_or_ident(s), line, col)
    }
}

/// Map identifier string to Token (keyword or Ident).
fn keyword_or_ident(s: &str) -> Token {
    match s {
        "module" => Token::Module,
        "endmodule" => Token::Endmodule,
        "input" => Token::Input,
        "output" => Token::Output,
        "inout" => Token::Inout,
        "ref" => Token::Ref,
        "wire" => Token::Wire,
        "reg" => Token::Reg,
        "logic" => Token::Logic,
        "int" => Token::Int,
        "integer" => Token::Integer,
        "signed" => Token::Signed,
        "unsigned" => Token::Unsigned,
        "wand" => Token::Wand,
        "wor" => Token::Wor,
        "tri" => Token::Tri,
        "tri0" => Token::Tri0,
        "tri1" => Token::Tri1,
        "triand" => Token::TriAnd,
        "trior" => Token::TriOr,
        "supply0" => Token::Supply0,
        "supply1" => Token::Supply1,
        "always" => Token::Always,
        "always_comb" => Token::AlwaysComb,
        "always_ff" => Token::AlwaysFF,
        "always_latch" => Token::AlwaysLatch,
        "initial" => Token::Initial,
        "final" => Token::Final,
        "assign" => Token::Assign,
        "begin" => Token::Begin,
        "end" => Token::End,
        "if" => Token::If,
        "else" => Token::Else,
        "case" => Token::Case,
        "casex" => Token::CaseX,
        "casez" => Token::CaseZ,
        "endcase" => Token::Endcase,
        "for" => Token::For,
        "while" => Token::While,
        "do" => Token::Do,
        "repeat" => Token::Repeat,
        "forever" => Token::Forever,
        "fork" => Token::Fork,
        "join" => Token::Join,
        "join_any" => Token::JoinAny,
        "join_none" => Token::JoinNone,
        "posedge" => Token::PosEdge,
        "negedge" => Token::NegEdge,
        "or" => Token::Or,
        "param" => Token::Param,
        "parameter" => Token::Parameter,
        "localparam" => Token::LocalParam,
        "genvar" => Token::GenVar,
        "generate" => Token::Generate,
        "endgenerate" => Token::EndGenerate,
        "function" => Token::Function,
        "endfunction" => Token::EndFunction,
        "task" => Token::Task,
        "endtask" => Token::EndTask,
        "foreach" => Token::Foreach,
        "auto" => Token::Auto,
        "static" => Token::Static,
        "real" => Token::Real,
        "time" => Token::Time,
        "realtime" => Token::RealTime,
        "string" => Token::String,
        "class" => Token::Class,
        "endclass" => Token::EndClass,
        "virtual" => Token::Virtual,
        "extends" => Token::Extends,
        "this" => Token::This,
        "new" => Token::New,
        "void" => Token::Void,
        "break" => Token::Break,
        "continue" => Token::Continue,
        "default" => Token::Default,
        "disable" => Token::Disable,
        "force" => Token::Force,
        "release" => Token::Release,
        "deassign" => Token::Deassign,
        "return" => Token::Return,
        "wait" => Token::Wait,
        "wait_order" => Token::WaitOrder,
        "inside" => Token::Inside,
        "rand" => Token::Rand,
        "randc" => Token::RandC,
        "constraint" => Token::Constraint,
        "const" => Token::Const,
        "var" => Token::Var,
        "solve" => Token::Solve,
        "unique" => Token::Unique,
        "priority" => Token::Priority,
        "unique0" => Token::Unique0,
        "property" => Token::Property,
        "sequence" => Token::Sequence,
        "endsequence" => Token::EndSequence,
        "package" => Token::Package,
        "endpackage" => Token::EndPackage,
        "import" => Token::Import,
        "export" => Token::Export,
        "bind" => Token::Bind,
        "specify" => Token::Specify,
        "endspecify" => Token::EndSpecify,
        "specparam" => Token::SpecParam,
        "clocking" => Token::Clocking,
        "endclocking" => Token::EndClocking,
        "config" => Token::Config,
        "endconfig" => Token::EndConfig,
        "design" => Token::Design,
        "liblist" => Token::Liblist,
        "cell" => Token::Cell,
        "use" => Token::Use,
        "instance" => Token::Instance,
        "primitive" => Token::Primitive,
        "endprimitive" => Token::EndPrimitive,
        "table" => Token::Table,
        "endtable" => Token::EndTable,
        "covergroup" => Token::Covergroup,
        "endgroup" => Token::EndGroup,
        "coverpoint" => Token::Coverpoint,
        "cross" => Token::Cross,
        "bins" => Token::Bins,
        "illegal_bins" => Token::IllegalBins,
        "ignore_bins" => Token::IgnoreBins,
        "option" => Token::Option_,
        "type" => Token::Type,
        "program" => Token::Program,
        "endprogram" => Token::EndProgram,
        "assert" => Token::Assert,
        "assume" => Token::Assume,
        "cover" => Token::Cover,
        "expect" => Token::Expect,
        "bit" => Token::Bit,
        "enum" => Token::Enum,
        "struct" => Token::Struct,
        "union" => Token::Union,
        "module_" => Token::Module_,
        "interface" => Token::Interface,
        "endinterface" => Token::EndInterface,
        "modport" => Token::ModPort,
        "always_comb" => Token::AlwaysComb,
        "null" => Token::Null,
        "none" => Token::None,
        "some" => Token::Some_,
        "and" => Token::And,
        "xor" => Token::Xor,
        "nand" => Token::Nand,
        "nor" => Token::Nor,
        "xnor" => Token::Xnor,
        "buf" => Token::Buf,
        "not" => Token::NotGate,
        _ => Token::Ident(Symbol::intern(s)),
    }
}

// ─── Comparison bench (legacy vs SIMD) ───

/// Tokenize with legacy lexer (Vec<char> based).
pub fn tokenize_legacy(source: &str) -> Vec<Token> {
    let mut lex = Lexer::new(source);
    let mut tokens = Vec::new();
    loop {
        let (tok, _, _) = lex.next_token();
        if tok == Token::Eof { break; }
        tokens.push(tok);
    }
    tokens
}

/// Tokenize with SIMD lexer (byte-based).
pub fn tokenize_simd(source: &str) -> Vec<Token> {
    let mut lex = SimdLexer::new(source.as_bytes());
    let mut tokens = Vec::new();
    loop {
        let (tok, _, _) = lex.next_token();
        if tok == Token::Eof { break; }
        tokens.push(tok);
    }
    tokens
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_tokens(src: &str) -> Vec<Token> { tokenize_legacy(src) }
    fn simd_tokens(src: &str) -> Vec<Token> { tokenize_simd(src) }

    #[test]
    fn test_simple_module() {
        let src = "module counter; endmodule";
        assert_eq!(legacy_tokens(src), simd_tokens(src));
    }

    #[test]
    fn test_ports() {
        let src = "module test(input clk, output reg [3:0] q); endmodule";
        assert_eq!(legacy_tokens(src), simd_tokens(src));
    }

    #[test]
    fn test_operators() {
        let src = "a + b - c * d / e % f";
        assert_eq!(legacy_tokens(src), simd_tokens(src));
    }

    #[test]
    fn test_comparison() {
        let src = "a == b & c != d && e === f !== g";
        assert_eq!(legacy_tokens(src), simd_tokens(src));
    }

    #[test]
    fn test_shift() {
        let src = "a << b >> c < d > e <= f >= g";
        assert_eq!(legacy_tokens(src), simd_tokens(src));
    }

    #[test]
    fn test_always_block() {
        let src = "always_ff @(posedge clk or negedge rst_n) begin q <= d; end";
        assert_eq!(legacy_tokens(src), simd_tokens(src));
    }

    #[test]
    fn test_number_formats() {
        let src = "42 8'hFF 16'd100 4'b1010 32'o77";
        assert_eq!(legacy_tokens(src), simd_tokens(src));
    }

    #[test]
    fn test_string_literal() {
        let src = r#"module m; initial $display("hello world"); endmodule"#;
        assert_eq!(legacy_tokens(src), simd_tokens(src));
    }

    #[test]
    fn test_comments() {
        let src = "// line comment\nmodule /* block */ test;";
        assert_eq!(legacy_tokens(src), simd_tokens(src));
    }

    #[test]
    fn test_full_module() {
        let src = include_str!("../../../test/counter.sv");
        assert_eq!(legacy_tokens(src), simd_tokens(src));
    }

    #[test]
    fn test_fill_literal() {
        let src = "assign a = '0; assign b = '1; assign c = 'x; assign d = 'z;";
        assert_eq!(legacy_tokens(src), simd_tokens(src));
    }

    #[test]
    fn test_increment_decrement() {
        let src = "a++ b-- c += 1 d -= 2";
        assert_eq!(legacy_tokens(src), simd_tokens(src));
    }

    #[test]
    fn test_scope() {
        let src = "pkg::my_type";
        assert_eq!(legacy_tokens(src), simd_tokens(src));
    }

    #[test]
    fn test_system_function() {
        let src = "$display $clog2 $bits $urandom";
        assert_eq!(legacy_tokens(src), simd_tokens(src));
    }
}
