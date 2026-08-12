//! Maria HDL (.mv) — Parser.
//! Recursive descent: tokens → AST (src/mv/ast.rs).
//! 1 file = 1 tanggung jawab: hanya parsing, tanpa codegen.

use crate::ast::*;
use crate::lexer::{tokenize, Tok};
use crate::MvError;

/// Parse source `.mv` → `MvFile`.
pub fn parse(src: &str) -> Result<MvFile, MvError> {
    let toks = tokenize(src)?;
    // Precompute offset byte awal tiap baris — dipakai raw-slice
    // `assert property (...)` yang harus dipertahankan tekstual (operator
    // SVA seperti `|->`/`##` bukan token `.mv`).
    let mut line_starts = vec![0usize];
    for (i, ch) in src.char_indices() {
        if ch == '\n' {
            line_starts.push(i + 1);
        }
    }
    let mut p = Parser {
        toks,
        pos: 0,
        src: src.to_string(),
        line_starts,
    };
    p.parse_file()
}

struct Parser {
    toks: Vec<(Tok, usize, usize)>,
    pos: usize,
    /// Sumber asli (untuk raw-slice `assert property`).
    src: String,
    /// Offset byte awal tiap baris (indeks = line - 1).
    line_starts: Vec<usize>,
}

impl Parser {
    // ── Token helpers ──
    fn peek(&self) -> &Tok {
        &self.toks[self.pos.min(self.toks.len() - 1)].0
    }
    fn peek_at(&self, n: usize) -> &Tok {
        let idx = (self.pos + n).min(self.toks.len() - 1);
        &self.toks[idx].0
    }
    fn pos_line(&self) -> (usize, usize) {
        let (_, l, c) = self.toks[self.pos.min(self.toks.len() - 1)];
        (l, c)
    }
    fn advance(&mut self) {
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == t {
            self.advance();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, t: &Tok) -> Result<(), MvError> {
        if self.peek() == t {
            self.advance();
            Ok(())
        } else {
            let (l, c) = self.pos_line();
            Err(MvError::new(
                l,
                c,
                format!("diharapkan {:?}, ditemukan {:?}", t, self.peek()),
            ))
        }
    }
    fn expect_ident(&mut self) -> Result<String, MvError> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.advance();
                Ok(s)
            }
            _ => {
                let (l, c) = self.pos_line();
                Err(MvError::new(l, c, format!("diharapkan identifier, ditemukan {:?}", self.peek())))
            }
        }
    }
    fn is_ident(&self, s: &str) -> bool {
        matches!(self.peek(), Tok::Ident(x) if x == s)
    }
    fn expect_ident_kw(&mut self, s: &str) -> Result<(), MvError> {
        if self.is_ident(s) {
            self.advance();
            Ok(())
        } else {
            let (l, c) = self.pos_line();
            Err(MvError::new(l, c, format!("diharapkan '{}'", s)))
        }
    }
    fn is_type_start(&self) -> bool {
        // Tipe dasar (`bit`, `logic`, `int`, ...) dan tipe user-defined semuanya
        // di-lex sebagai `Tok::Ident` — dibedakan lewat string di parse_base_type.
        matches!(self.peek(), Tok::Ident(_))
    }

    // ── Raw slice (untuk `assert property`) ──
    fn byte_offset(&self, line: usize, col: usize) -> usize {
        let ls = *self.line_starts.get(line.wrapping_sub(1)).unwrap_or(&0);
        ls + col.saturating_sub(1)
    }

    /// Potong teks asli dari (sl, sc) ke (el, ec) — 1-based line/col, end eksklusif.
    ///
    /// Catatan (edge case): `line_starts` memakai offset BYTE (`char_indices`),
    /// sedangkan `col` lexer dihitung per CHAR — untuk baris yang memuat
    /// karakter multi-byte (non-ASCII) SEBELUM titik potong, offset bisa
    /// meleset. Body `assert property` praktis selalu ASCII (operator SVA),
    /// jadi ini diterima; jangan dipakai untuk slicing sumber bebas unicode.
    fn raw_slice(&self, sl: usize, sc: usize, el: usize, ec: usize) -> String {
        let s = self.byte_offset(sl, sc);
        let e = self.byte_offset(el, ec).min(self.src.len());
        if s >= e {
            String::new()
        } else {
            self.src[s..e].to_string()
        }
    }

    // ── File ──
    fn parse_file(&mut self) -> Result<MvFile, MvError> {
        let mut f = MvFile::default();
        loop {
            match self.peek().clone() {
                Tok::Eof => break,
                Tok::Type => {
                    f.typedefs.push(self.parse_typedef_alias()?);
                }
                Tok::Packed | Tok::Struct | Tok::Enum | Tok::Union => {
                    f.typedefs.push(self.parse_typedef()?);
                }
                Tok::Package => {
                    f.packages.push(self.parse_package()?);
                }
                Tok::Interface => {
                    f.interfaces.push(self.parse_interface()?);
                }
                Tok::Module => {
                    f.modules.push(self.parse_module()?);
                }
                Tok::Func => {
                    f.funcs.push(self.parse_func()?);
                }
                Tok::Task => {
                    f.tasks.push(self.parse_task()?);
                }
                Tok::Ident(s) if s == "program" => {
                    // `program` bukan keyword token — di-lex sebagai Ident.
                    f.programs.push(self.parse_program()?);
                }
                Tok::Ident(s) if s == "class" => {
                    // `class` bukan keyword token — di-lex sebagai Ident.
                    f.classes.push(self.parse_class()?);
                }
                Tok::Ident(_) => {
                    // `type NAME = ...` boleh juga tanpa keyword `type`? Tidak —
                    // alias wajib `type`. Ident di level file = error.
                    let (l, c) = self.pos_line();
                    return Err(MvError::new(l, c, format!("konstruk tidak dikenal di level file: {:?}", self.peek())));
                }
                _ => {
                    let (l, c) = self.pos_line();
                    return Err(MvError::new(l, c, format!("konstruk tidak dikenal: {:?}", self.peek())));
                }
            }
        }
        Ok(f)
    }

    // ── Typedef ──
    fn parse_typedef_alias(&mut self) -> Result<Typedef, MvError> {
        self.expect(&Tok::Type)?;
        // F11: posisi nama typedef (untuk error type-check ber-posisi)
        let (l, c) = self.pos_line();
        let name = self.expect_ident()?;
        self.expect(&Tok::BlockingAssign)?;
        let ty = self.parse_type()?;
        Ok(Typedef::Alias { name, ty, line: l, col: c })
    }

    fn parse_typedef(&mut self) -> Result<Typedef, MvError> {
        match self.peek().clone() {
            Tok::Packed | Tok::Struct => {
                let packed = self.eat(&Tok::Packed);
                self.expect(&Tok::Struct)?;
                let (l, c) = self.pos_line();
                let name = self.expect_ident()?;
                self.expect(&Tok::LBrace)?;
                let mut fields = Vec::new();
                while !self.eat(&Tok::RBrace) {
                    fields.push(self.parse_field()?);
                    self.eat(&Tok::Comma);
                }
                Ok(Typedef::Struct { name, packed, fields, line: l, col: c })
            }
            Tok::Enum => {
                self.expect(&Tok::Enum)?;
                let width = if self.eat(&Tok::LParen) {
                    let w = self.parse_expr()?;
                    self.expect(&Tok::RParen)?;
                    Some(w)
                } else {
                    None
                };
                let (l, c) = self.pos_line();
                let name = self.expect_ident()?;
                self.expect(&Tok::LBrace)?;
                let mut members = Vec::new();
                while !self.eat(&Tok::RBrace) {
                    let (ml, mc) = self.pos_line();
                    let mname = self.expect_ident()?;
                    let value = if self.eat(&Tok::BlockingAssign) {
                        Some(self.parse_expr()?)
                    } else {
                        None
                    };
                    members.push(EnumMember { name: mname, value, line: ml, col: mc });
                    self.eat(&Tok::Comma);
                }
                Ok(Typedef::Enum { name, width, members, line: l, col: c })
            }
            Tok::Union => {
                self.expect(&Tok::Union)?;
                let (l, c) = self.pos_line();
                let name = self.expect_ident()?;
                self.expect(&Tok::LBrace)?;
                let mut fields = Vec::new();
                while !self.eat(&Tok::RBrace) {
                    fields.push(self.parse_field()?);
                    self.eat(&Tok::Comma);
                }
                // Union diperlakukan sebagai struct packed lebar-max di codegen.
                Ok(Typedef::Struct { name, packed: true, fields, line: l, col: c })
            }
            _ => {
                let (l, c) = self.pos_line();
                Err(MvError::new(l, c, "typedef tidak dikenal".to_string()))
            }
        }
    }

    fn parse_field(&mut self) -> Result<Field, MvError> {
        let (l, c) = self.pos_line();
        let mut names = vec![self.expect_ident()?];
        while self.eat(&Tok::Comma) {
            names.push(self.expect_ident()?);
        }
        self.expect(&Tok::Colon)?;
        let ty = self.parse_type()?;
        Ok(Field { names, ty, line: l, col: c })
    }

    // ── Package ──
    fn parse_package(&mut self) -> Result<Package, MvError> {
        self.expect(&Tok::Package)?;
        let (l, c) = self.pos_line();
        let name = self.expect_ident()?;
        self.expect(&Tok::LBrace)?;
        let mut typedefs = Vec::new();
        let mut consts = Vec::new();
        loop {
            match self.peek().clone() {
                Tok::RBrace => {
                    self.advance();
                    break;
                }
                Tok::Eof => {
                    let (l, c) = self.pos_line();
                    return Err(MvError::new(l, c, "package tidak ditutup dengan '}'".to_string()));
                }
                Tok::Type => typedefs.push(self.parse_typedef_alias()?),
                Tok::Packed | Tok::Struct | Tok::Enum | Tok::Union => typedefs.push(self.parse_typedef()?),
                Tok::Const => {
                    self.expect(&Tok::Const)?;
                    let cname = self.expect_ident()?;
                    let ty = if self.eat(&Tok::Colon) { Some(self.parse_type()?) } else { None };
                    self.expect(&Tok::BlockingAssign)?;
                    let value = self.parse_expr()?;
                    consts.push((cname, ty, value));
                }
                _ => {
                    let (l, c) = self.pos_line();
                    return Err(MvError::new(l, c, format!("item package tidak dikenal: {:?}", self.peek())));
                }
            }
        }
        Ok(Package { name, typedefs, consts, line: l, col: c })
    }

    // ── Interface (MARIA-HDL.md §6.10) ──
    /// `interface name { in/out ports, sig, modport }` — definisi bersama
    /// yang di-emit ke `.svh`. Body: port (`in clk : bit`) & `sig x : T`
    /// sama-sama jadi deklarasi signal interface; `modport` menamai subset
    /// signal dengan arah untuk view koneksi.
    fn parse_interface(&mut self) -> Result<Interface, MvError> {
        self.expect(&Tok::Interface)?;
        let (l, c) = self.pos_line();
        let name = self.expect_ident()?;
        self.expect(&Tok::LBrace)?;
        let mut ports = Vec::new();
        let mut sigs = Vec::new();
        let mut modports = Vec::new();
        loop {
            match self.peek().clone() {
                Tok::RBrace => {
                    self.advance();
                    break;
                }
                Tok::Eof => {
                    let (l, c) = self.pos_line();
                    return Err(MvError::new(l, c, "interface tidak ditutup dengan '}'".to_string()));
                }
                Tok::In | Tok::Out | Tok::Inout => {
                    let (pl, pc) = self.pos_line();
                    let dir = match self.peek().clone() {
                        Tok::In => Dir::In,
                        Tok::Out => Dir::Out,
                        _ => Dir::Inout,
                    };
                    self.advance();
                    let mut names = vec![self.expect_ident()?];
                    while self.eat(&Tok::Comma) {
                        names.push(self.expect_ident()?);
                    }
                    self.expect(&Tok::Colon)?;
                    let ty = self.parse_type()?;
                    ports.push(Port { dir, names, ty, line: pl, col: pc });
                }
                Tok::Sig => {
                    let (sl, sc) = self.pos_line();
                    self.advance();
                    let mut names = vec![self.expect_ident()?];
                    while self.eat(&Tok::Comma) {
                        names.push(self.expect_ident()?);
                    }
                    self.expect(&Tok::Colon)?;
                    let ty = self.parse_type()?;
                    sigs.push((names, ty, sl, sc));
                }
                Tok::Modport => modports.push(self.parse_modport()?),
                _ => {
                    let (l, c) = self.pos_line();
                    return Err(MvError::new(
                        l,
                        c,
                        format!("item interface tidak dikenal: {:?}", self.peek()),
                    ));
                }
            }
        }
        Ok(Interface {
            name,
            ports,
            sigs,
            modports,
            line: l,
            col: c,
        })
    }

    /// `modport slave { in a, b; out c }` — deklarasi arah baru dimulai oleh
    /// keyword `in`/`out`/`inout` (nama signal tidak mungkin keyword),
    /// sehingga baris baru tanpa `;` juga ter-parse dengan benar.
    fn parse_modport(&mut self) -> Result<Modport, MvError> {
        self.expect(&Tok::Modport)?;
        let (l, c) = self.pos_line();
        let name = self.expect_ident()?;
        self.expect(&Tok::LBrace)?;
        let mut dirs = Vec::new();
        loop {
            let dir = match self.peek().clone() {
                Tok::In => {
                    self.advance();
                    Dir::In
                }
                Tok::Out => {
                    self.advance();
                    Dir::Out
                }
                Tok::Inout => {
                    self.advance();
                    Dir::Inout
                }
                _ => break,
            };
            let mut names = vec![self.expect_ident()?];
            while self.eat(&Tok::Comma) {
                names.push(self.expect_ident()?);
            }
            dirs.push((dir, names));
        }
        self.expect(&Tok::RBrace)?;
        Ok(Modport {
            name,
            dirs,
            line: l,
            col: c,
        })
    }

    // ── Module ──
    /// `program name { ... }` (MARIA-HDL.md §7.3) — testbench program.
    /// Body memakai item module (port, initial, ...) — struktur reuse `Module`.
    fn parse_program(&mut self) -> Result<Module, MvError> {
        self.advance(); // `program` (Ident)
        let (l, c) = self.pos_line();
        let name = self.expect_ident()?;
        self.expect(&Tok::LBrace)?;
        let mut items = Vec::new();
        loop {
            match self.peek().clone() {
                Tok::RBrace => {
                    self.advance();
                    break;
                }
                Tok::Eof => {
                    let (l, c) = self.pos_line();
                    return Err(MvError::new(l, c, "program tidak ditutup dengan '}'".to_string()));
                }
                _ => items.push(self.parse_module_item()?),
            }
        }
        Ok(Module {
            name,
            params: Vec::new(),
            items,
            line: l,
            col: c,
        })
    }

    /// `class Name [extends Base] { field/constraint/func/task }` (MARIA-HDL.md §8).
    /// Keyword `class`/`extends`/`field`/`rand`/`constraint` di-lex sebagai
    /// `Tok::Ident` — dibedakan lewat string.
    fn parse_class(&mut self) -> Result<MClass, MvError> {
        self.advance(); // `class` (Ident)
        let name = self.expect_ident()?;
        let extends = if self.is_ident("extends") {
            self.advance();
            Some(self.expect_ident()?)
        } else {
            None
        };
        self.expect(&Tok::LBrace)?;
        let (l, c) = self.pos_line();
        let mut cls = MClass {
            name,
            extends,
            line: l,
            col: c,
            ..Default::default()
        };
        loop {
            match self.peek().clone() {
                Tok::RBrace => {
                    self.advance();
                    break;
                }
                Tok::Eof => {
                    let (l, c) = self.pos_line();
                    return Err(MvError::new(l, c, "class tidak ditutup dengan '}'".to_string()));
                }
                Tok::Func => cls.funcs.push(self.parse_func()?),
                Tok::Task => cls.tasks.push(self.parse_task()?),
                Tok::Ident(s) if s == "field" || s == "rand" => {
                    // `field x : T` / `rand field x : T`
                    let rand = if self.is_ident("rand") {
                        self.advance();
                        self.expect_ident_kw("field")?;
                        true
                    } else {
                        self.advance(); // field
                        false
                    };
                    let mut names = vec![self.expect_ident()?];
                    while self.eat(&Tok::Comma) {
                        names.push(self.expect_ident()?);
                    }
                    self.expect(&Tok::Colon)?;
                    let ty = self.parse_type()?;
                    for n in names {
                        cls.fields.push((n, ty.clone(), rand));
                    }
                }
                Tok::Ident(s) if s == "constraint" => {
                    self.advance();
                    let cname = self.expect_ident()?;
                    // parse_constraint_block sendiri yang memakan `{` (F12).
                    let items = self.parse_constraint_block()?;
                    cls.constraints.push((cname, items));
                }
                _ => {
                    let (l, c) = self.pos_line();
                    return Err(MvError::new(l, c, format!("item class tidak dikenal: {:?}", self.peek())));
                }
            }
        }
        Ok(cls)
    }

    fn parse_module(&mut self) -> Result<Module, MvError> {
        self.expect(&Tok::Module)?;
        let (l, c) = self.pos_line();
        let name = self.expect_ident()?;
        let params = if self.eat(&Tok::Hash) {
            self.expect(&Tok::LParen)?;
            let mut ps = Vec::new();
            while !self.eat(&Tok::RParen) {
                ps.push(self.parse_param()?);
                self.eat(&Tok::Comma);
            }
            ps
        } else {
            Vec::new()
        };
        self.expect(&Tok::LBrace)?;
        let mut items = Vec::new();
        loop {
            match self.peek().clone() {
                Tok::RBrace => {
                    self.advance();
                    break;
                }
                Tok::Eof => {
                    let (l, c) = self.pos_line();
                    return Err(MvError::new(l, c, "module tidak ditutup dengan '}'".to_string()));
                }
                _ => items.push(self.parse_module_item()?),
            }
        }
        Ok(Module { name, params, items, line: l, col: c })
    }

    fn parse_param(&mut self) -> Result<Param, MvError> {
        let (l, c) = self.pos_line();
        // F32: bentuk (B) `type T = logic[7:0]` — kata kunci `type` di awal
        // (paralel dgn bentuk (A) `T : type = logic[7:0]`).
        let is_type_kw = self.peek() == &Tok::Type;
        if is_type_kw {
            self.advance();
        }
        let name = self.expect_ident()?;
        let ty = if self.eat(&Tok::Colon) {
            Some(self.parse_type()?)
        } else if is_type_kw {
            // F32 fix review: bentuk (B) `type T = ...` — set marker agar
            // check/codegen/collect_ctx konsisten (tanpa ini ty=None →
            // is_tp false bila tak ada default → `sig x : T` E2005).
            Some(MvType::Named("type".into(), l, c))
        } else {
            None
        };
        let is_type_marker = matches!(&ty, Some(MvType::Named(s, ..)) if s == "type");
        let (default, type_default) = if self.eat(&Tok::BlockingAssign) {
            if is_type_kw || is_type_marker {
                // type param: default adalah TIPE, bukan ekspresi nilai
                (None, Some(self.parse_type()?))
            } else {
                (Some(self.parse_expr()?), None)
            }
        } else {
            (None, None)
        };
        Ok(Param { name, ty, default, type_default, line: l, col: c })
    }

    fn parse_module_item(&mut self) -> Result<MItem, MvError> {
        match self.peek().clone() {
            Tok::In | Tok::Out | Tok::Inout => {
                let (l, c) = self.pos_line();
                let dir = match self.peek().clone() {
                    Tok::In => Dir::In,
                    Tok::Out => Dir::Out,
                    _ => Dir::Inout,
                };
                self.advance();
                let mut names = vec![self.expect_ident()?];
                while self.eat(&Tok::Comma) {
                    names.push(self.expect_ident()?);
                }
                self.expect(&Tok::Colon)?;
                let ty = self.parse_type()?;
                Ok(MItem::Port(Port { dir, names, ty, line: l, col: c }))
            }
            Tok::Sig | Tok::Reg => {
                let (l, c) = self.pos_line();
                let is_reg = matches!(self.peek(), Tok::Reg);
                self.advance();
                let mut names = vec![self.expect_ident()?];
                while self.eat(&Tok::Comma) {
                    names.push(self.expect_ident()?);
                }
                self.expect(&Tok::Colon)?;
                let ty = self.parse_type()?;
                let init = if self.eat(&Tok::BlockingAssign) {
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                if is_reg {
                    Ok(MItem::Reg { names, ty, init, line: l, col: c })
                } else {
                    Ok(MItem::Sig { names, ty, init, line: l, col: c })
                }
            }
            Tok::Const => {
                let (l, c) = self.pos_line();
                self.advance();
                let name = self.expect_ident()?;
                let ty = if self.eat(&Tok::Colon) { Some(self.parse_type()?) } else { None };
                self.expect(&Tok::BlockingAssign)?;
                let value = self.parse_expr()?;
                Ok(MItem::Const { name, ty, value, line: l, col: c })
            }
            Tok::Use => {
                self.advance();
                let pkg = self.expect_ident()?;
                self.expect(&Tok::Scope)?;
                let item = if self.eat(&Tok::Star) {
                    "*".to_string()
                } else {
                    self.expect_ident()?
                };
                Ok(MItem::Use { pkg, item })
            }
            Tok::Seq => Ok(MItem::Seq(self.parse_seq_spec()?, self.parse_stmt()?)),
            Tok::Comb => {
                self.advance();
                Ok(MItem::Comb(self.parse_stmt()?))
            }
            Tok::Always => {
                self.advance();
                Ok(MItem::Always(self.parse_stmt()?))
            }
            Tok::Latch => {
                self.advance();
                Ok(MItem::Latch(self.parse_stmt()?))
            }
            Tok::Initial => {
                self.advance();
                Ok(MItem::Initial(self.parse_stmt()?))
            }
            Tok::Final => {
                self.advance();
                Ok(MItem::Final(self.parse_stmt()?))
            }
            Tok::Inst => Ok(self.parse_inst()?),
            Tok::For => {
                // generate for
                self.advance();
                let var = self.expect_ident()?;
                self.expect(&Tok::In)?;
                let from = self.parse_expr()?;
                self.expect(&Tok::DotDot)?;
                let to = self.parse_expr()?;
                let body = self.parse_module_item_block()?;
                Ok(MItem::GenFor { var, from, to, body })
            }
            Tok::If => {
                // generate if
                self.advance();
                self.expect(&Tok::LParen)?;
                let cond = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                let then = self.parse_module_item_block()?;
                let els = if self.eat(&Tok::Else) {
                    self.parse_module_item_block()?
                } else {
                    Vec::new()
                };
                Ok(MItem::GenIf { cond, then, els })
            }
            Tok::Func => Ok(MItem::Func(self.parse_func()?)),
            Tok::Task => Ok(MItem::Task(self.parse_task()?)),
            _ => {
                let (l, c) = self.pos_line();
                Err(MvError::new(l, c, format!("item module tidak dikenal: {:?}", self.peek())))
            }
        }
    }

    fn parse_module_item_block(&mut self) -> Result<Vec<MItem>, MvError> {
        self.expect(&Tok::LBrace)?;
        let mut items = Vec::new();
        while !self.eat(&Tok::RBrace) {
            items.push(self.parse_module_item()?);
        }
        Ok(items)
    }

    /// `seq(clk)` / `seq(clk, rst)` / `seq(clk, rst, sync)` / `seq(negedge clk, ...)`
    fn parse_seq_spec(&mut self) -> Result<SeqSpec, MvError> {
        self.expect(&Tok::Seq)?;
        let (l, c) = self.pos_line();
        self.expect(&Tok::LParen)?;
        let mut neg_edge = false;
        if matches!(self.peek(), Tok::NegEdge) {
            neg_edge = true;
            self.advance();
        } else if matches!(self.peek(), Tok::PosEdge) {
            self.advance();
        }
        // F26: clock bisa `clk` (signal) atau `iface.clk` (field port
        // interface) — parse postfix, ambil teks persis via raw_slice.
        let (sl, sc) = self.pos_line();
        let cexpr = self.parse_postfix_expr()?;
        let (el, ec) = self.pos_line();
        let clk = self.raw_slice(sl, sc, el, ec).trim().to_string();
        match cexpr {
            Expr::Ident(..) | Expr::Member(..) | Expr::Index(..) | Expr::Range(..) => {}
            _ => {
                return Err(MvError::new(
                    l,
                    c,
                    format!("clock seq harus berupa signal (mis. `clk` atau `iface.clk`), ditemukan: {clk}"),
                ));
            }
        }
        let mut reset = None;
        if self.eat(&Tok::Comma) {
            let rname = self.expect_ident()?;
            let active_low = rname.ends_with("_n") || rname.ends_with("_N");
            let mut sync = false;
            if self.eat(&Tok::Comma) && matches!(self.peek(), Tok::Ident(s) if s == "sync") {
                sync = true;
                self.advance();
            }
            reset = Some((rname, active_low, sync));
        }
        self.expect(&Tok::RParen)?;
        Ok(SeqSpec { clk, neg_edge, reset, line: l, col: c })
    }

    fn parse_inst(&mut self) -> Result<MItem, MvError> {
        self.expect(&Tok::Inst)?;
        // Catat posisi nama module utk error validasi koneksi port (F29).
        let (line, col) = self.pos_line();
        let module = self.expect_ident()?;
        let name = self.expect_ident()?;
        let dims = if self.eat(&Tok::LBrack) {
            let e = self.parse_expr()?;
            self.expect(&Tok::RBrack)?;
            Some(e)
        } else {
            None
        };
        let mut params = Vec::new();
        if self.eat(&Tok::Hash) {
            self.expect(&Tok::LParen)?;
            while !self.eat(&Tok::RParen) {
                self.expect(&Tok::Dot)?;
                let pname = self.expect_ident()?;
                self.expect(&Tok::LParen)?;
                let pval = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                params.push((pname, pval));
                self.eat(&Tok::Comma);
            }
        }
        let mut conns = Vec::new();
        if self.eat(&Tok::LParen) {
            while !self.eat(&Tok::RParen) {
                if self.eat(&Tok::Dot) {
                    let port = self.expect_ident()?;
                    let expr = if self.eat(&Tok::LParen) {
                        let e = self.parse_expr()?;
                        self.expect(&Tok::RParen)?;
                        Some(e)
                    } else {
                        None
                    };
                    conns.push(Conn::Named { port, expr });
                } else {
                    let e = self.parse_expr()?;
                    conns.push(Conn::Positional(e));
                }
                self.eat(&Tok::Comma);
            }
        }
        Ok(MItem::Inst { module, name, dims, params, conns, line, col })
    }

    // ── Function / Task ──
    fn parse_func(&mut self) -> Result<MFunc, MvError> {
        self.expect(&Tok::Func)?;
        let (l, c) = self.pos_line();
        let name = self.expect_ident()?;
        let args = self.parse_arg_list()?;
        let ret = if self.eat(&Tok::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };
        let body = self.parse_stmt_block()?;
        Ok(MFunc { name, args, ret, body, line: l, col: c })
    }

    fn parse_task(&mut self) -> Result<MTask, MvError> {
        self.expect(&Tok::Task)?;
        let (l, c) = self.pos_line();
        let name = self.expect_ident()?;
        let args = self.parse_arg_list()?;
        let body = self.parse_stmt_block()?;
        Ok(MTask { name, args, body, line: l, col: c })
    }

    fn parse_arg_list(&mut self) -> Result<Vec<(String, MvType, Option<Dir>)>, MvError> {
        self.expect(&Tok::LParen)?;
        let mut args = Vec::new();
        while !self.eat(&Tok::RParen) {
            let dir = match self.peek().clone() {
                Tok::In => {
                    self.advance();
                    Some(Dir::In)
                }
                Tok::Out => {
                    self.advance();
                    Some(Dir::Out)
                }
                Tok::Inout => {
                    self.advance();
                    Some(Dir::Inout)
                }
                _ => None,
            };
            let name = self.expect_ident()?;
            self.expect(&Tok::Colon)?;
            let ty = self.parse_type()?;
            args.push((name, ty, dir));
            self.eat(&Tok::Comma);
        }
        Ok(args)
    }

    /// F26: parse body `case (...)` setelah keyword — items `val: stmt`,
    /// `a, b: stmt`, dan `default: stmt`. `qual` = priority/unique/unique0
    /// (None utk biasa), `kind` = "case"/"casez"/"casex".
    fn parse_case_body(
        &mut self,
        qual: Option<String>,
        kind: String,
    ) -> Result<Stmt, MvError> {
        self.expect(&Tok::LParen)?;
        let expr = self.parse_expr()?;
        self.expect(&Tok::RParen)?;
        self.expect(&Tok::LBrace)?;
        let mut items = Vec::new();
        let mut default: Option<Box<Stmt>> = None;
        while !self.eat(&Tok::RBrace) {
            if self.eat(&Tok::Default) {
                self.expect(&Tok::Colon)?;
                default = Some(Box::new(self.parse_stmt()?));
            } else {
                let mut vals = vec![self.parse_expr()?];
                while self.eat(&Tok::Comma) {
                    vals.push(self.parse_expr()?);
                }
                self.expect(&Tok::Colon)?;
                let body = self.parse_stmt()?;
                items.push((vals, body));
            }
        }
        Ok(Stmt::Case {
            expr,
            items,
            default,
            qual,
            kind,
        })
    }

    fn parse_stmt_block(&mut self) -> Result<Vec<Stmt>, MvError> {
        self.expect(&Tok::LBrace)?;
        let mut stmts = Vec::new();
        while !self.eat(&Tok::RBrace) {
            stmts.push(self.parse_stmt()?);
        }
        Ok(stmts)
    }

    // ── Statements ──
    fn parse_stmt(&mut self) -> Result<Stmt, MvError> {
        match self.peek().clone() {
            Tok::LBrace => {
                let body = self.parse_stmt_block()?;
                Ok(Stmt::Block(body))
            }
            Tok::If => {
                self.advance();
                self.expect(&Tok::LParen)?;
                let cond = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                let then = self.parse_stmt()?;
                let els = if self.eat(&Tok::Else) {
                    Some(Box::new(self.parse_stmt()?))
                } else {
                    None
                };
                Ok(Stmt::If { cond, then: Box::new(then), els })
            }
            // F26: case qualifier + casez/casex — `priority case (...)`,
            // `unique casez (...)`, `casex (...)`. Qualifier/kind dibaca dulu,
            // lalu body case di-parse oleh parse_case_body.
            Tok::Priority | Tok::Unique | Tok::Unique0 => {
                let qual = match self.peek() {
                    Tok::Priority => "priority",
                    Tok::Unique => "unique",
                    _ => "unique0",
                }
                .to_string();
                self.advance();
                let kind = match self.peek() {
                    Tok::Case => "case",
                    Tok::Casez => "casez",
                    Tok::Casex => "casex",
                    _ => {
                        let (l, c) = self.pos_line();
                        return Err(MvError::new(
                            l,
                            c,
                            "diharapkan 'case'/'casez'/'casex' setelah qualifier".to_string(),
                        ));
                    }
                }
                .to_string();
                self.advance();
                self.parse_case_body(Some(qual), kind)
            }
            Tok::Casez | Tok::Casex => {
                let kind = match self.peek() {
                    Tok::Casez => "casez",
                    _ => "casex",
                }
                .to_string();
                self.advance();
                self.parse_case_body(None, kind)
            }
            Tok::Case => {
                self.advance();
                self.parse_case_body(None, "case".to_string())
            }
            Tok::For => {
                self.advance();
                let var = self.expect_ident()?;
                self.expect(&Tok::In)?;
                let from = self.parse_expr()?;
                self.expect(&Tok::DotDot)?;
                let to = self.parse_expr()?;
                let body = self.parse_stmt()?;
                Ok(Stmt::For { var, from, to, body: Box::new(body) })
            }
            Tok::While => {
                self.advance();
                self.expect(&Tok::LParen)?;
                let cond = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                let body = self.parse_stmt()?;
                Ok(Stmt::While { cond, body: Box::new(body) })
            }
            // F38: `do { body } while (cond)` — loop post-test.
            Tok::Do => {
                self.advance();
                let body = self.parse_stmt()?;
                self.expect(&Tok::While)?;
                self.expect(&Tok::LParen)?;
                let cond = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                self.eat(&Tok::Semi);
                Ok(Stmt::DoWhile { cond, body: Box::new(body) })
            }
            // F38: event trigger `->ev` — memicu event named (emisi `-> ev;`).
            // Target HANYA ident: parser SV `Stmt::EventTrigger` menerima nama
            // ident saja (`expect_ident`), jadi `-> obj.sig`/`-> q[0]` ditolak
            // di level .mv dengan error jelas — bukan SV invalid hasil generate.
            Tok::Arrow => {
                self.advance();
                let (l, c) = self.pos_line();
                let name = self.expect_ident()?;
                self.eat(&Tok::Semi);
                Ok(Stmt::EventTrigger(Expr::Ident(name, l, c)))
            }
            // F39: `fork { stmt* } { stmt* } ... join / join_any / join_none`
            // — branch konkurren, masing-masing blok `{ ... }`. Diakhiri salah
            // satu keyword join (bukan `}` lagi).
            Tok::Fork => {
                self.advance();
                let mut branches = Vec::new();
                loop {
                    match self.peek() {
                        Tok::Join | Tok::JoinAny | Tok::JoinNone => break,
                        Tok::LBrace => {
                            let b = self.parse_stmt_block()?;
                            branches.push(Stmt::Block(b));
                        }
                        _ => {
                            let (l, c) = self.pos_line();
                            return Err(MvError::new(
                                l,
                                c,
                                "tiap branch fork harus blok '{ ... }' — diakhiri 'join'/'join_any'/'join_none'".to_string(),
                            ));
                        }
                    }
                }
                let join = match self.peek() {
                    Tok::Join => {
                        self.advance();
                        ForkJoin::Join
                    }
                    Tok::JoinAny => {
                        self.advance();
                        ForkJoin::JoinAny
                    }
                    Tok::JoinNone => {
                        self.advance();
                        ForkJoin::JoinNone
                    }
                    _ => {
                        let (l, c) = self.pos_line();
                        return Err(MvError::new(
                            l,
                            c,
                            "diharapkan 'join' / 'join_any' / 'join_none' setelah blok fork".to_string(),
                        ));
                    }
                };
                Ok(Stmt::Fork { branches, join })
            }
            Tok::Repeat => {
                self.advance();
                self.expect(&Tok::LParen)?;
                let count = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                let body = self.parse_stmt()?;
                Ok(Stmt::Repeat { count, body: Box::new(body) })
            }
            Tok::Forever => {
                self.advance();
                let body = self.parse_stmt()?;
                Ok(Stmt::Forever(Box::new(body)))
            }
            Tok::Wait => {
                self.advance();
                self.expect(&Tok::LParen)?;
                let cond = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                let body = self.parse_stmt()?;
                Ok(Stmt::Wait { cond, body: Box::new(body) })
            }
            Tok::At => {
                self.advance();
                self.expect(&Tok::LParen)?;
                let expr = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                let body = self.parse_stmt()?;
                Ok(Stmt::Event { expr, body: Box::new(body) })
            }
            Tok::Hash => {
                self.advance();
                let amt = self.parse_expr()?;
                // Delay tanpa body (`#100` di akhir method/block) = delay-only
                let body = if matches!(self.peek(), Tok::RBrace) {
                    Stmt::Block(Vec::new())
                } else {
                    self.parse_stmt()?
                };
                Ok(Stmt::Delay { amt, body: Box::new(body) })
            }
            Tok::Return => {
                self.advance();
                let v = if matches!(self.peek(), Tok::RBrace) || matches!(self.peek(), Tok::Semi) {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                self.eat(&Tok::Semi);
                Ok(Stmt::Return(v))
            }
            Tok::Break => {
                self.advance();
                self.eat(&Tok::Semi);
                Ok(Stmt::Break)
            }
            Tok::Continue => {
                self.advance();
                self.eat(&Tok::Semi);
                Ok(Stmt::Continue)
            }
            Tok::Var => {
                self.advance();
                let mut names = vec![self.expect_ident()?];
                while self.eat(&Tok::Comma) {
                    names.push(self.expect_ident()?);
                }
                self.expect(&Tok::Colon)?;
                let ty = self.parse_type()?;
                let init = if self.eat(&Tok::BlockingAssign) {
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                Ok(Stmt::VarDecl { names, ty, init })
            }
            Tok::Assert => {
                self.advance();
                // `assert property (...)` — concurrent assertion. Body diambil
                // RAW (teks persis `(...)`) karena berisi operator SVA (`|->`,
                // `##`, `[*]`) yang bukan token .mv. Emisi 1:1 (MARIA-HDL.md §7.2).
                if self.is_ident("property") {
                    self.advance();
                    let (sl, sc) = self.pos_line(); // posisi `(`
                    self.expect(&Tok::LParen)?;
                    let mut depth = 1usize;
                    loop {
                        match self.peek() {
                            Tok::Eof => {
                                let (l, c) = self.pos_line();
                                return Err(MvError::new(l, c, "assert property tidak ditutup".to_string()));
                            }
                            Tok::LParen => {
                                depth += 1;
                                self.advance();
                            }
                            Tok::RParen => {
                                let (l, c) = self.pos_line();
                                depth -= 1;
                                self.advance();
                                if depth == 0 {
                                    let raw = self.raw_slice(sl, sc, l, c + 1);
                                    return Ok(Stmt::AssertProperty(raw));
                                }
                            }
                            _ => {
                                self.advance();
                            }
                        }
                    }
                }
                self.expect(&Tok::LParen)?;
                let cond = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                let pass = if matches!(self.peek(), Tok::Else) || matches!(self.peek(), Tok::RBrace) {
                    None
                } else {
                    Some(Box::new(self.parse_stmt()?))
                };
                let fail = if self.eat(&Tok::Else) {
                    Some(Box::new(self.parse_stmt()?))
                } else {
                    None
                };
                Ok(Stmt::Assert { cond, pass, fail })
            }
            // F37: prefix `++lhs` / `--lhs` di level statement. Hasil sama
            // dengan postfix (`lhs++`); di-emit sesuai aslinya.
            Tok::PlusPlus | Tok::MinusMinus => {
                let inc = matches!(self.peek(), Tok::PlusPlus);
                let (l, c) = self.pos_line();
                self.advance();
                let lhs = self.parse_postfix_expr()?;
                Ok(Stmt::IncDec { lhs, inc, pre: true, line: l, col: c })
            }
            _ => self.parse_assign_or_expr(),
        }
    }

    /// Statement di level statement: assignment (`=`/`<=`) atau ekspresi.
    fn parse_assign_or_expr(&mut self) -> Result<Stmt, MvError> {
        // Cek pola lvalue: Ident (postfix)* diikuti `=` / `<=`
        if matches!(self.peek(), Tok::Ident(_)) {
            // F11: posisi statement untuk error assignment (E2002/E2003/E2004)
            let (l, c) = self.pos_line();
            // Simpan posisi untuk backtrack
            let save = self.pos;
            // F37: varian stmt — lhs boleh postfix `i++` (arm F36 di bawah),
            // ekspresi RHS (`j = i++`) tetap ditolak.
            let lhs = match self.parse_postfix_expr_stmt() {
                Ok(e) => e,
                Err(_) => {
                    self.pos = save;
                    let e = self.parse_expr()?;
                    return Ok(Stmt::ExprStmt(e));
                }
            };
            match self.peek().clone() {
                Tok::BlockingAssign => {
                    self.advance();
                    let rhs = self.parse_expr()?;
                    return Ok(Stmt::Assign { lhs, rhs, nba: false, line: l, col: c });
                }
                Tok::NonBlockingAssign => {
                    self.advance();
                    let rhs = self.parse_expr()?;
                    return Ok(Stmt::Assign { lhs, rhs, nba: true, line: l, col: c });
                }
                // F36: postfix `lhs++` / `lhs--`
                Tok::PlusPlus | Tok::MinusMinus => {
                    // F37 fix: guard baris — `++`/`--` di baris BERBEDA dari
                    // akhir lhs = statement prefix baru (`--i` setelah
                    // `$display(...)`), bukan postfix dari statement ini.
                    let end_line = self.toks[(self.pos.saturating_sub(1)).min(self.toks.len() - 1)].1;
                    if self.toks[self.pos].1 != end_line {
                        self.pos = save;
                        let e = self.parse_expr()?;
                        return Ok(Stmt::ExprStmt(e));
                    }
                    let inc = matches!(self.peek(), Tok::PlusPlus);
                    self.advance();
                    return Ok(Stmt::IncDec { lhs, inc, pre: false, line: l, col: c });
                }
                // F36: compound `lhs += rhs` dst.
                Tok::PlusEq
                | Tok::MinusEq
                | Tok::StarEq
                | Tok::SlashEq
                | Tok::PercentEq
                | Tok::ShlEq
                | Tok::SshrEq
                | Tok::AndEq
                | Tok::OrEq
                | Tok::XorEq => {
                    let op = match self.peek().clone() {
                        Tok::PlusEq => "+=".to_string(),
                        Tok::MinusEq => "-=".to_string(),
                        Tok::StarEq => "*=".to_string(),
                        Tok::SlashEq => "/=".to_string(),
                        Tok::PercentEq => "%=".to_string(),
                        Tok::ShlEq => "<<=".to_string(),
                        Tok::SshrEq => ">>=".to_string(),
                        Tok::AndEq => "&=".to_string(),
                        Tok::OrEq => "|=".to_string(),
                        Tok::XorEq => "^=".to_string(),
                        _ => unreachable!(),
                    };
                    self.advance();
                    let rhs = self.parse_expr()?;
                    return Ok(Stmt::CompoundAssign { lhs, rhs, op, line: l, col: c });
                }
                _ => {
                    self.pos = save;
                    let e = self.parse_expr()?;
                    return Ok(Stmt::ExprStmt(e));
                }
            }
        }
        let e = self.parse_expr()?;
        Ok(Stmt::ExprStmt(e))
    }

    // ── Types ──
    fn parse_type(&mut self) -> Result<MvType, MvError> {
        let mut t = self.parse_base_type()?;
        // unpacked dims: `Type[N][M]`
        while self.eat(&Tok::LBrack) {
            let d = self.parse_expr()?;
            self.expect(&Tok::RBrack)?;
            t = MvType::Array(Box::new(t), vec![d]);
        }
        Ok(t)
    }

    fn parse_base_type(&mut self) -> Result<MvType, MvError> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                let (l, c) = self.pos_line();
                let s = s.clone();
                self.advance();
                // `logic[...]`
                if s == "logic" {
                    let range = if self.eat(&Tok::LBrack) {
                        let a = self.parse_expr()?;
                        if self.eat(&Tok::Colon) {
                            let b = self.parse_expr()?;
                            self.expect(&Tok::RBrack)?;
                            Some((a, b))
                        } else {
                            self.expect(&Tok::RBrack)?;
                            // `logic[N]` → `[N-1:0]`
                            Some((sub_one(a), Expr::Int(0)))
                        }
                    } else {
                        None
                    };
                    Ok(MvType::Logic(range))
                } else if s == "bit" {
                    Ok(MvType::Bit)
                } else if s == "int" {
                    Ok(MvType::Int)
                } else if s == "uint" {
                    Ok(MvType::Uint)
                } else if s == "longint" {
                    Ok(MvType::LongInt)
                } else if s == "shortint" {
                    Ok(MvType::ShortInt)
                } else if s == "byte" {
                    Ok(MvType::Byte)
                } else if s == "real" {
                    Ok(MvType::Real)
                } else if s == "time" {
                    Ok(MvType::Time)
                } else if s == "string" {
                    Ok(MvType::Str)
                } else if s == "signed" {
                    let inner = self.parse_base_type()?;
                    Ok(MvType::Signed(Box::new(inner)))
                } else {
                    // User-defined type `State`, `Packet`, `Addr`
                    // scoped: `pkg::Type`
                    if self.eat(&Tok::Scope) {
                        let item = self.expect_ident()?;
                        Ok(MvType::Named(format!("{}::{}", s, item), l, c))
                    } else {
                        Ok(MvType::Named(s, l, c))
                    }
                }
            }
            Tok::Type => {
                // F32: kata kunci `type` sbg tipe — marker type parameter
                // (`T : type = logic[7:0]`). Bukan tipe nyata: validasi &
                // emisi ditangani khusus di check.rs / codegen.rs.
                let (l, c) = self.pos_line();
                self.advance();
                Ok(MvType::Named("type".into(), l, c))
            }
            _ => {
                let (l, c) = self.pos_line();
                Err(MvError::new(l, c, format!("tipe tidak dikenal: {:?}", self.peek())))
            }
        }
    }

    // ── Expressions (Pratt) ──
    fn parse_expr(&mut self) -> Result<Expr, MvError> {
        self.parse_binary(1)
    }

    /// Tabel precedence: 1 = loosest, 12 = tightest (binary).
    fn parse_binary(&mut self, min_prec: u8) -> Result<Expr, MvError> {
        let mut lhs = self.parse_unary()?;
        loop {
            let (op, prec) = match self.peek().clone() {
                Tok::PipePipe => ("||", 1),
                Tok::AmpAmp => ("&&", 2),
                Tok::Pipe => ("|", 3),
                Tok::Caret => ("^", 4),
                Tok::Amp => ("&", 5),
                Tok::Eq => ("==", 6),
                Tok::Neq => ("!=", 6),
                Tok::CaseEq => ("===", 6),
                Tok::CaseNeq => ("!==", 6),
                // `<=` di dalam ekspresi = relasional; di statement level
                // parse_assign_or_expr menangkapnya sebagai non-blocking.
                Tok::Lt => ("<", 7),
                Tok::NonBlockingAssign => ("<=", 7),
                Tok::Gt => (">", 7),
                Tok::Ge => (">=", 7),
                Tok::Shl => ("<<", 8),
                Tok::Shr => (">>", 8),
                Tok::Sshl => ("<<<", 8),
                Tok::Sshr => (">>>", 8),
                Tok::Plus => ("+", 9),
                Tok::Minus => ("-", 9),
                Tok::Star => ("*", 10),
                Tok::Slash => ("/", 10),
                Tok::Percent => ("%", 10),
                Tok::Power => ("**", 11),
                _ => break,
            };
            if prec < min_prec {
                break;
            }
            self.advance();
            let rhs = self.parse_binary(prec + 1)?;
            lhs = Expr::Binary(op.into(), Box::new(lhs), Box::new(rhs));
        }
        // `lhs inside { set }` / `lhs dist { items }` (F12) — hanya di level
        // ekspresi paling luar (min_prec 1). Set/dist bisa berisi range
        // `[lo:hi]` yang bukan ekspresi biasa.
        if min_prec <= 1 && self.eat(&Tok::Inside) {
            self.expect(&Tok::LBrace)?;
            let mut items = Vec::new();
            while !self.eat(&Tok::RBrace) {
                if matches!(self.peek(), Tok::LBrack) {
                    self.advance();
                    let lo = self.parse_expr()?;
                    self.expect(&Tok::Colon)?;
                    let hi = self.parse_expr()?;
                    self.expect(&Tok::RBrack)?;
                    items.push(InsideItem::Range(lo, hi));
                } else {
                    items.push(InsideItem::Value(self.parse_expr()?));
                }
                self.eat(&Tok::Comma);
            }
            lhs = Expr::Inside {
                expr: Box::new(lhs),
                items,
            };
        }
        if min_prec <= 1 && self.eat(&Tok::Dist) {
            self.expect(&Tok::LBrace)?;
            let mut items = Vec::new();
            while !self.eat(&Tok::RBrace) {
                items.push(self.parse_dist_item()?);
                self.eat(&Tok::Comma);
            }
            lhs = Expr::Dist {
                expr: Box::new(lhs),
                items,
            };
        }
        // Ternary
        if min_prec <= 1 && self.eat(&Tok::Question) {
            let then = self.parse_expr()?;
            self.expect(&Tok::Colon)?;
            let els = self.parse_expr()?;
            lhs = Expr::Ternary(Box::new(lhs), Box::new(then), Box::new(els));
        }
        Ok(lhs)
    }

    // ── Constraint lanjutan (F12) ──

    /// Blok `{ item, item, ... }` di dalam `constraint c { ... }`.
    fn parse_constraint_block(&mut self) -> Result<Vec<ConstraintItem>, MvError> {
        self.expect(&Tok::LBrace)?;
        let mut items = Vec::new();
        while !self.eat(&Tok::RBrace) {
            items.push(self.parse_constraint_item()?);
            self.eat(&Tok::Comma);
        }
        Ok(items)
    }

    /// Satu item constraint: `if/else`, `solve var before a, b`, atau ekspresi.
    fn parse_constraint_item(&mut self) -> Result<ConstraintItem, MvError> {
        match self.peek().clone() {
            Tok::If => {
                self.advance();
                self.expect(&Tok::LParen)?;
                let cond = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                let then = self.parse_constraint_block()?;
                let els = if self.eat(&Tok::Else) {
                    self.parse_constraint_block()?
                } else {
                    Vec::new()
                };
                Ok(ConstraintItem::If { cond, then, els })
            }
            Tok::Solve => {
                let (l, c) = self.pos_line();
                self.advance();
                let var = self.expect_ident()?;
                self.expect(&Tok::Before)?;
                let mut before = vec![self.expect_ident()?];
                while self.eat(&Tok::Comma) {
                    before.push(self.expect_ident()?);
                }
                Ok(ConstraintItem::Solve {
                    var,
                    before,
                    line: l,
                    col: c,
                })
            }
            _ => Ok(ConstraintItem::Expr(self.parse_expr()?)),
        }
    }

    /// Item dist: `[lo:hi] := w` / `[lo:hi] :/ w` / `value := w` / `value :/ w`.
    fn parse_dist_item(&mut self) -> Result<DistItem, MvError> {
        if matches!(self.peek(), Tok::LBrack) {
            self.advance();
            let lo = self.parse_expr()?;
            self.expect(&Tok::Colon)?;
            let hi = self.parse_expr()?;
            self.expect(&Tok::RBrack)?;
            let (exact, weight) = self.parse_dist_weight()?;
            Ok(DistItem {
                value: lo.clone(),
                range: Some((lo, hi)),
                weight,
                exact,
            })
        } else {
            let value = self.parse_expr()?;
            let (exact, weight) = self.parse_dist_weight()?;
            Ok(DistItem {
                value,
                range: None,
                weight,
                exact,
            })
        }
    }

    /// Bobot dist: `:= expr` (exact) atau `:/ expr` (dibagi).
    fn parse_dist_weight(&mut self) -> Result<(bool, Expr), MvError> {
        if self.eat(&Tok::Equiv) {
            Ok((true, self.parse_expr()?))
        } else if self.eat(&Tok::ColonSlash) {
            Ok((false, self.parse_expr()?))
        } else {
            let (l, c) = self.pos_line();
            Err(MvError::new(
                l,
                c,
                "diharapkan ':= ' atau ':/ ' setelah item dist".to_string(),
            ))
        }
    }

    fn parse_unary(&mut self) -> Result<Expr, MvError> {
        match self.peek().clone() {
            Tok::Not => {
                self.advance();
                // Operand unary di-parse dengan min_prec 11 agar `**` (prec 11)
                // terikat lebih erat dari unary: `-a ** b` = `-(a ** b)` (SV).
                let e = self.parse_binary(11)?;
                Ok(Expr::Unary("!".into(), Box::new(e)))
            }
            Tok::Tilde => {
                self.advance();
                // Operand unary di-parse dengan min_prec 11 agar `**` (prec 11)
                // terikat lebih erat dari unary: `-a ** b` = `-(a ** b)` (SV).
                let e = self.parse_binary(11)?;
                Ok(Expr::Unary("~".into(), Box::new(e)))
            }
            Tok::Minus => {
                self.advance();
                // Operand unary di-parse dengan min_prec 11 agar `**` (prec 11)
                // terikat lebih erat dari unary: `-a ** b` = `-(a ** b)` (SV).
                let e = self.parse_binary(11)?;
                Ok(Expr::Unary("-".into(), Box::new(e)))
            }
            Tok::Plus => {
                self.advance();
                // Operand unary di-parse dengan min_prec 11 agar `**` (prec 11)
                // terikat lebih erat dari unary: `-a ** b` = `-(a ** b)` (SV).
                let e = self.parse_binary(11)?;
                Ok(Expr::Unary("+".into(), Box::new(e)))
            }
            Tok::Amp => {
                self.advance();
                // Operand unary di-parse dengan min_prec 11 agar `**` (prec 11)
                // terikat lebih erat dari unary: `-a ** b` = `-(a ** b)` (SV).
                let e = self.parse_binary(11)?;
                Ok(Expr::Unary("&".into(), Box::new(e)))
            }
            Tok::Pipe => {
                self.advance();
                // Operand unary di-parse dengan min_prec 11 agar `**` (prec 11)
                // terikat lebih erat dari unary: `-a ** b` = `-(a ** b)` (SV).
                let e = self.parse_binary(11)?;
                Ok(Expr::Unary("|".into(), Box::new(e)))
            }
            Tok::Caret => {
                self.advance();
                // Operand unary di-parse dengan min_prec 11 agar `**` (prec 11)
                // terikat lebih erat dari unary: `-a ** b` = `-(a ** b)` (SV).
                let e = self.parse_binary(11)?;
                Ok(Expr::Unary("^".into(), Box::new(e)))
            }
            // F37: prefix `++x` / `--x` di level EKSPRESI (RHS): `j = ++i`.
            // Operand di-parse postfix (memakan `[i]`/`.f`/`(args)` bila ada).
            Tok::PlusPlus | Tok::MinusMinus => {
                let inc = matches!(self.peek(), Tok::PlusPlus);
                self.advance();
                let e = self.parse_postfix_expr()?;
                Ok(Expr::IncDec { inc, pre: true, expr: Box::new(e) })
            }
            _ => self.parse_postfix_expr(),
        }
    }

    fn parse_postfix_expr(&mut self) -> Result<Expr, MvError> {
        self.parse_postfix_expr_inner(false)
    }

    /// F37: varian utk lhs statement — postfix `++`/`--` DIPERBOLEHKAN
    /// (`i++` baris sendiri); ekspresi RHS (`j = i++`) tetap ditolak.
    /// Postfix di-`break`, pemanggil (parse_assign_or_expr arm F36) yang
    /// mengubahnya jadi `Stmt::IncDec`.
    fn parse_postfix_expr_stmt(&mut self) -> Result<Expr, MvError> {
        self.parse_postfix_expr_inner(true)
    }

    fn parse_postfix_expr_inner(&mut self, allow_postfix: bool) -> Result<Expr, MvError> {
        let mut e = self.parse_primary()?;
        loop {
            match self.peek().clone() {
                Tok::LBrack => {
                    self.advance();
                    let a = self.parse_expr()?;
                    if self.eat(&Tok::Colon) {
                        let b = self.parse_expr()?;
                        self.expect(&Tok::RBrack)?;
                        e = Expr::Range(Box::new(e), Box::new(a), Box::new(b));
                    } else {
                        self.expect(&Tok::RBrack)?;
                        e = Expr::Index(Box::new(e), Box::new(a));
                    }
                }
                Tok::Dot => {
                    let (l, c) = self.pos_line();
                    self.advance();
                    let f = self.expect_ident()?;
                    e = Expr::Member(Box::new(e), f, l, c);
                }
                Tok::LParen => {
                    self.advance();
                    let mut args = Vec::new();
                    while !self.eat(&Tok::RParen) {
                        args.push(self.parse_expr()?);
                        self.eat(&Tok::Comma);
                    }
                    match e {
                        Expr::Ident(name, ..) => e = Expr::Call(name, args),
                        Expr::Scoped(p, i, ..) => e = Expr::Call(format!("{}::{}", p, i), args),
                        // method call `obj.method(args)` — termasuk `this`/`super`
                        Expr::Member(obj, method, ..) => {
                            e = Expr::MethodCall {
                                obj,
                                method,
                                args,
                            }
                        }
                        Expr::MethodCall { obj, method, .. } => {
                            e = Expr::MethodCall { obj, method, args }
                        }
                        _ => {
                            let (l, c) = self.pos_line();
                            return Err(MvError::new(l, c, "hanya fungsi yang bisa dipanggil".to_string()));
                        }
                    }
                }
                // F37: postfix `x++`/`x--` di akhir ekspresi.
                Tok::PlusPlus | Tok::MinusMinus => {
                    // Guard baris: `++`/`--` di baris BERBEDA dari akhir
                    // ekspresi = statement prefix baru (`++i` setelah
                    // `$display(...)`), bukan postfix — break biar statement
                    // berikutnya yang menanganinya.
                    let end_line = self.toks[(self.pos.saturating_sub(1)).min(self.toks.len() - 1)].1;
                    if self.toks[self.pos].1 != end_line {
                        break;
                    }
                    if allow_postfix {
                        // lhs statement: serahkan ke parse_assign_or_expr
                        // (arm F36) yang mengubahnya jadi Stmt::IncDec.
                        break;
                    }
                    // RHS ekspresi: postfix TIDAK didukung di .mv (side-effect
                    // dalam ekspresi tidak bisa diwakili SV) — error jelas di
                    // level .mv, bukan menghasilkan SV invalid.
                    let (l, c) = self.pos_line();
                    let op = if matches!(self.peek(), Tok::PlusPlus) { "++" } else { "--" };
                    return Err(MvError::new(
                        l,
                        c,
                        format!("postfix {op} hanya didukung sebagai statement (baris sendiri), bukan di dalam ekspresi"),
                    ));
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn parse_primary(&mut self) -> Result<Expr, MvError> {
        match self.peek().clone() {
            Tok::Int(v) => {
                self.advance();
                Ok(Expr::Int(v))
            }
            Tok::Sized(w, b, d) => {
                let (l, c) = self.pos_line();
                self.advance();
                Ok(Expr::Sized(w, b, d, l, c))
            }
            Tok::Real(v) => {
                self.advance();
                Ok(Expr::Real(v))
            }
            Tok::Fill(c) => {
                self.advance();
                Ok(Expr::Fill(c))
            }
            Tok::Str(s) => {
                self.advance();
                Ok(Expr::Str(s))
            }
            Tok::Ident(s) => {
                let (l, c) = self.pos_line();
                // F33: type cast `T'(expr)` / `logic'(x)` / `Word16'(x)` /
                // `pkg::T'(x)` — ident (tipe) diikuti Quote. parse_base_type
                // menangani tipe dasar, user-defined, dan scoped `pkg::T`.
                if self.peek_at(1) == &Tok::Quote {
                    let ty = self.parse_base_type()?;
                    self.expect(&Tok::Quote)?;
                    self.expect(&Tok::LParen)?;
                    let e = self.parse_expr()?;
                    self.expect(&Tok::RParen)?;
                    return Ok(Expr::Cast {
                        ty: Box::new(ty),
                        expr: Box::new(e),
                        line: l,
                        col: c,
                    });
                }
                let s = s.clone();
                self.advance();
                // scoped `pkg::item`
                if self.eat(&Tok::Scope) {
                    let item = self.expect_ident()?;
                    Ok(Expr::Scoped(s, item, l, c))
                } else {
                    Ok(Expr::Ident(s, l, c))
                }
            }
            // `@(posedge clk)` — edge diwakili unary "posedge"/"negedge"
            Tok::PosEdge | Tok::NegEdge => {
                let neg = matches!(self.peek(), Tok::NegEdge);
                self.advance();
                let inner = self.parse_primary()?;
                let op = if neg { "negedge" } else { "posedge" };
                Ok(Expr::Unary(op.into(), Box::new(inner)))
            }
            Tok::LParen => {
                self.advance();
                let e = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                Ok(Expr::Paren(Box::new(e)))
            }
            Tok::LBrace => {
                // concat `{a, b}` atau replication `{n{a}}`
                self.advance();
                let first = self.parse_expr()?;
                if self.eat(&Tok::LBrace) {
                    // replication `{n{expr}}`
                    let inner = self.parse_expr()?;
                    self.expect(&Tok::RBrace)?;
                    self.expect(&Tok::RBrace)?;
                    Ok(Expr::Replicate(Box::new(first), Box::new(inner)))
                } else {
                    let mut parts = vec![first];
                    while self.eat(&Tok::Comma) {
                        parts.push(self.parse_expr()?);
                    }
                    self.expect(&Tok::RBrace)?;
                    Ok(Expr::Concat(parts))
                }
            }
            _ => {
                let (l, c) = self.pos_line();
                Err(MvError::new(l, c, format!("ekspresi tidak valid: {:?}", self.peek())))
            }
        }
    }
}

/// `expr - 1` (untuk `logic[N]` → `[N-1:0]`). Konstanta langsung di-fold.
fn sub_one(e: Expr) -> Expr {
    match e {
        Expr::Int(v) => Expr::Int(v - 1),
        other => Expr::Binary("-".into(), Box::new(other), Box::new(Expr::Int(1))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_counter() {
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
        let f = parse(src).expect("parse counter");
        assert_eq!(f.modules.len(), 1);
        let m = &f.modules[0];
        assert_eq!(m.name, "counter");
        assert_eq!(m.params.len(), 1);
        assert_eq!(m.items.len(), 4); // 3 port + 1 blok seq
        match &m.items[0] {
            MItem::Port(p) => {
                assert_eq!(p.dir, Dir::In);
                assert_eq!(p.names, vec!["clk", "rst_n"]);
            }
            _ => panic!("harus port"),
        }
    }

    #[test]
    fn parse_precedence_power_vs_unary() {
        // `**` terikat lebih erat dari unary minus: `-a ** b` = `-(a ** b)` (SV).
        let src = "module m { in a, b : bit\n out y : bit\n comb { y = -a ** b } }";
        let f = parse(src).unwrap();
        let m = &f.modules[0];
        let mut found = false;
        for item in &m.items {
            if let MItem::Comb(body) = item {
                let stmts: &[Stmt] = match body {
                    Stmt::Block(s) => s.as_slice(),
                    other => std::slice::from_ref(other),
                };
                for s in stmts {
                    if let Stmt::Assign { rhs, .. } = s {
                        // rhs harus Unary("-", Binary("**", a, b))
                        if let Expr::Unary(op, inner) = rhs {
                            if op == "-" {
                                if let Expr::Binary(bop, l, r) = inner.as_ref() {
                                    assert_eq!(bop, "**");
                                    assert!(matches!(l.as_ref(), Expr::Ident(..)));
                                    assert!(matches!(r.as_ref(), Expr::Ident(..)));
                                    found = true;
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(found, "`-a ** b` harus di-parse sebagai `-(a ** b)`");
    }

    #[test]
    fn parse_package_and_types() {
        let src = r#"
package counter_pkg {
    type Addr = logic[15:0]
    packed struct Packet {
        valid : bit,
        addr  : Addr
    }
    enum(3) Color { RED = 0, GREEN = 2, BLUE = 4 }
}
"#;
        let f = parse(src).unwrap();
        assert_eq!(f.packages.len(), 1);
        let pkg = &f.packages[0];
        assert_eq!(pkg.typedefs.len(), 3);
        assert!(matches!(pkg.typedefs[0], Typedef::Alias { name: ref n, .. } if n == "Addr"));
        assert!(matches!(pkg.typedefs[1], Typedef::Struct { packed: true, .. }));
        assert!(matches!(pkg.typedefs[2], Typedef::Enum { name: ref n, .. } if n == "Color"));
    }

    #[test]
    fn parse_traffic() {
        let src = r#"
module traffic #(GREEN_T = 30) {
    use traffic_pkg::*
    in  clk, rst_n : bit
    out state      : State
    out red, green, yellow : bit

    reg state : State
    reg timer : uint[8]

    seq(clk, rst_n) {
        if (!rst_n) {
            state <= RED
            timer <= 0
        } else {
            case (state) {
                RED: {
                    if (timer == GREEN_T) {
                        state <= GREEN
                        timer <= 0
                    } else {
                        timer <= timer + 1
                    }
                }
                default: {
                    state <= RED
                }
            }
        }
    }

    comb {
        red    = (state == RED)
        green  = (state == GREEN)
        yellow = (state == YELLOW)
    }
}
"#;
        let f = parse(src).unwrap();
        let m = &f.modules[0];
        assert_eq!(m.name, "traffic");
        let seq_count = m
            .items
            .iter()
            .filter(|i| matches!(i, MItem::Seq(..)))
            .count();
        assert_eq!(seq_count, 1);
        let comb_count = m.items.iter().filter(|i| matches!(i, MItem::Comb(..))).count();
        assert_eq!(comb_count, 1);
    }

    #[test]
    fn parse_instance() {
        let src = r#"
module top {
    in clk : bit
    inst counter_small u_small (.clk, .rst_n, .count(count[3:0]))
    inst alu u_alu (a, b, op, y)
    inst flipflop u_ff[8] (.clk, .d(d_in), .q(q_out))
}
"#;
        let f = parse(src).unwrap();
        let m = &f.modules[0];
        let insts: Vec<_> = m
            .items
            .iter()
            .filter_map(|i| match i {
                MItem::Inst { name, dims, conns, .. } => Some((name.as_str(), dims.is_some(), conns.len())),
                _ => None,
            })
            .collect();
        assert_eq!(insts.len(), 3);
        assert_eq!(insts[0], ("u_small", false, 3));
        assert_eq!(insts[1], ("u_alu", false, 4));
        assert_eq!(insts[2], ("u_ff", true, 3));
    }

    #[test]
    fn parse_func_task() {
        let src = r#"
func clog2(x : int) -> int {
    var r : int = 0
    var n : int = x - 1
    while (n > 0) {
        r = r + 1
        n = n >> 1
    }
    return r
}

task send(data : logic[7:0], out ok : bit) {
    #10
    ok = 1
}
"#;
        let f = parse(src).unwrap();
        assert_eq!(f.funcs.len(), 1);
        assert_eq!(f.tasks.len(), 1);
        let func = &f.funcs[0];
        assert_eq!(func.name, "clog2");
        assert!(func.ret.is_some());
        assert_eq!(func.args.len(), 1);
        assert_eq!(func.body.len(), 4); // var, var, while, return
    }

    #[test]
    fn parse_expr_precedence() {
        let src = r#"
module m {
    out y : logic[7:0]
    comb {
        y = a + b * c - d / e
        y = (a || b) && (c & d) | (e ^ f)
        y = a ? b : c
        y = {a, b, c}
        y = {4{a}}
        y = x[3:0]
        y = pkt.valid
        y = $clog2(DEPTH)
        y = 8'hFF
    }
}
"#;
        let f = parse(src).unwrap();
        let m = &f.modules[0];
        let comb = m.items.iter().find_map(|i| match i {
            MItem::Comb(s) => Some(s),
            _ => None,
        });
        let comb = comb.expect("comb");
        let stmts = match comb {
            Stmt::Block(v) => v,
            _ => panic!("comb body harus block"),
        };
        assert_eq!(stmts.len(), 9);
    }

    #[test]
    fn parse_assert_property_raw() {
        // Body `assert property (...)` dipertahankan RAW (operator SVA `|->`
        // dan `##` bukan token .mv) — emisi 1:1.
        let src = r#"
module m {
    in clk, enable : bit
    in count       : logic[7:0]
    initial {
        assert property (@(posedge clk) enable |-> count == $past(count) + 1)
    }
}
"#;
        let f = parse(src).unwrap();
        let m = &f.modules[0];
        // module punya 2 port + 1 initial = 3 item; cari initial via iter
        let init = m
            .items
            .iter()
            .find_map(|i| match i {
                MItem::Initial(body) => Some(body),
                _ => None,
            })
            .expect("harus ada initial");
        let stmts: &[Stmt] = match init {
            Stmt::Block(s) => s.as_slice(),
            other => std::slice::from_ref(other),
        };
        let mut found = false;
        for s in stmts {
            if let Stmt::AssertProperty(raw) = s {
                assert_eq!(
                    raw,
                    "(@(posedge clk) enable |-> count == $past(count) + 1)",
                    "raw harus persis (termasuk parens): {raw}"
                );
                found = true;
            }
        }
        assert!(found, "harus ada Stmt::AssertProperty");
    }

    #[test]
    fn parse_program_block() {
        // Program block (MARIA-HDL.md §7.3)
        let src = r#"
program test_runner {
    in clk : bit
    initial {
        run_test("my_test")
    }
}
"#;
        let f = parse(src).unwrap();
        assert_eq!(f.programs.len(), 1);
        let p = &f.programs[0];
        assert_eq!(p.name, "test_runner");
        assert_eq!(p.items.len(), 2); // port + initial
        assert!(matches!(p.items[0], MItem::Port(..)));
        assert!(matches!(p.items[1], MItem::Initial(..)));
    }

    #[test]
    fn parse_class_uvm() {
        // Class + UVM subset (MARIA-HDL.md §8)
        let src = r#"
class my_test extends uvm_test {
    field count     : uint
    rand field seed : uint
    constraint c { seed > 10, seed < 200 }
    func new(name : string) {
        super.new(name)
    }
    func build_phase() {
        uvm_config_db::set(this, "*.agent", "count", count)
    }
    task run_phase() {
        var seqr : uvm_sequencer
        seqr.start_item(item)
        seqr.finish_item(item)
        #100
    }
}
"#;
        let f = parse(src).unwrap();
        assert_eq!(f.classes.len(), 1);
        let c = &f.classes[0];
        assert_eq!(c.name, "my_test");
        assert_eq!(c.extends.as_deref(), Some("uvm_test"));
        assert_eq!(c.fields.len(), 2);
        assert_eq!(c.fields[0], ("count".into(), MvType::Uint, false));
        assert_eq!(c.fields[1], ("seed".into(), MvType::Uint, true));
        assert_eq!(c.constraints.len(), 1);
        assert_eq!(c.funcs.len(), 2);
        assert_eq!(c.tasks.len(), 1);
        // method call + delay-only di body task
        let t = &c.tasks[0];
        assert!(t.body.iter().any(|s| matches!(
            s,
            Stmt::ExprStmt(Expr::MethodCall {
                method,
                obj: _,
                args: _
            }) if method == "start_item"
        )));
        assert!(t.body.iter().any(|s| matches!(
            s,
            Stmt::Delay {
                amt: Expr::Int(100),
                body
            } if matches!(body.as_ref(), Stmt::Block(v) if v.is_empty())
        )));
    }

    #[test]
    fn parse_class_field_edges() {
        // multi-nama field, constraint kosong
        let src = r#"
class c {
    field a, b : uint
    rand field r1, r2 : bit
    constraint empty { }
}
"#;
        let f = parse(src).unwrap();
        let c = &f.classes[0];
        assert_eq!(c.fields.len(), 4);
        assert_eq!(c.fields[0], ("a".into(), MvType::Uint, false));
        assert_eq!(c.fields[1], ("b".into(), MvType::Uint, false));
        assert_eq!(c.fields[2], ("r1".into(), MvType::Bit, true));
        assert_eq!(c.fields[3], ("r2".into(), MvType::Bit, true));
        assert_eq!(c.constraints.len(), 1);
        assert!(c.constraints[0].1.is_empty());
    }

    #[test]
    fn parse_delay_only_in_initial() {
        // delay-only `#100` di akhir blok module (bukan hanya class task)
        let src = "module tb {\n in clk : bit\n initial {\n clk = 0\n #100\n } }\n";
        let f = parse(src).unwrap();
        let m = &f.modules[0];
        if let MItem::Initial(body) = &m.items[1] {
            let stmts: &[Stmt] = match body {
                Stmt::Block(s) => s.as_slice(),
                other => std::slice::from_ref(other),
            };
            assert!(stmts.iter().any(|s| matches!(
                s,
                Stmt::Delay {
                    amt: Expr::Int(100),
                    body
                } if matches!(body.as_ref(), Stmt::Block(v) if v.is_empty())
            )));
        } else {
            panic!("harus initial");
        }
    }

    #[test]
    fn parse_class_plain() {
        // Class tanpa extends — reuse parse_class
        let src = r#"
class counter_model {
    field value : uint
    func new() {
        value = 0
    }
    task tick() {
        value = value + 1
    }
}
"#;
        let f = parse(src).unwrap();
        assert_eq!(f.classes.len(), 1);
        let c = &f.classes[0];
        assert_eq!(c.name, "counter_model");
        assert!(c.extends.is_none());
        assert_eq!(c.fields.len(), 1);
        assert_eq!(c.funcs.len(), 1);
        assert_eq!(c.tasks.len(), 1);
    }

    #[test]
    fn parse_constraint_advanced() {
        // F12: constraint lanjutan — inside/dist/if-else/solve dalam satu blok
        let src = r#"
class item extends uvm_sequence_item {
    rand field mode : uint[2]
    rand field addr : uint[8]
    rand field data : uint[8]
    field limit : uint[8]
    constraint c_adv {
        addr inside {[1:10], 20, 30},
        data dist { 0 := 1, [1:5] :/ 9 },
        if (mode == 1) { addr > 5 } else { addr < 100 },
        solve addr before data
    }
}
"#;
        let f = parse(src).unwrap();
        assert_eq!(f.classes.len(), 1);
        let c = &f.classes[0];
        assert_eq!(c.constraints.len(), 1);
        let (name, items) = &c.constraints[0];
        assert_eq!(name, "c_adv");
        assert_eq!(items.len(), 4);
        // 1) inside (dengan range + nilai tunggal, urutan dijaga 1:1)
        assert!(matches!(items[0], ConstraintItem::Expr(Expr::Inside { .. })));
        if let ConstraintItem::Expr(Expr::Inside { items: ins, .. }) = &items[0] {
            assert_eq!(ins.len(), 3);
            assert!(matches!(ins[0], InsideItem::Range(_, _))); // [1:10]
            assert!(matches!(ins[1], InsideItem::Value(_))); // 20
            assert!(matches!(ins[2], InsideItem::Value(_))); // 30
        } else {
            panic!("item 0 harus inside");
        }
        // 2) dist (nilai := dan range :/)
        assert!(matches!(items[1], ConstraintItem::Expr(Expr::Dist { .. })));
        if let ConstraintItem::Expr(Expr::Dist { items: d, .. }) = &items[1] {
            assert_eq!(d.len(), 2);
            assert!(d[0].exact && d[0].range.is_none()); // 0 := 1
            assert!(!d[1].exact && d[1].range.is_some()); // [1:5] :/ 9
        } else {
            panic!("item 1 harus dist");
        }
        // 3) if/else
        assert!(matches!(items[2], ConstraintItem::If { .. }));
        if let ConstraintItem::If { then, els, .. } = &items[2] {
            assert_eq!(then.len(), 1);
            assert_eq!(els.len(), 1);
        } else {
            panic!("item 2 harus if");
        }
        // 4) solve
        assert!(matches!(&items[3], ConstraintItem::Solve { var, .. } if var == "addr"));
    }

    #[test]
    fn parse_generate() {
        let src = r#"
module shiftreg #(N : int = 8) {
    in clk : bit
    in d   : bit
    out q  : logic[N-1:0]
    seq(clk) {
        q[0] <= d
    }
    for i in 1..N {
        seq(clk) {
            q[i] <= q[i-1]
        }
    }
}
"#;
        let f = parse(src).unwrap();
        let m = &f.modules[0];
        assert!(m.items.iter().any(|i| matches!(i, MItem::GenFor { .. })));
    }
}
