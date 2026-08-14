//! ──────────────────────────────────────────────────────────────────────────────
//! CATATAN: File ini adalah bagian dari pemisahan parser.rs (SRP Refactoring).
//! Tanggung jawab: Parsing class declaration (class ... endclass).
//!
//! Fungsi:
//!   - parse_class() — parsing deklarasi class dengan type params, extends, members
//!
//! ──────────────────────────────────────────────────────────────────────────────

use super::Parser;
use maria_ast::*;
use maria_core::error::SimError;
use crate::lexer::*;

impl Parser {
    /// Fast skip for first pass: collect class name + fast-skip body to endclass.
    /// Does NOT parse members — dramatically faster for class discovery pass.
    /// Parse isi blok `constraint name { ... }` (berhenti di `}` tanpa
    /// memakannya). Item: `solve a before b`, `if (c) {..} else {..}` (F12,
    /// constraint kondisional), atau ekspresi (termasuk `inside`/`dist`).
    /// Ekspresi yang gagal di-skip ke `;` (recovery — perilaku lama).
    fn parse_constraint_items(&mut self) -> Result<Vec<ConstraintItem>, SimError> {
        let mut body = Vec::new();
        while self.peek() != &Token::RBrace && self.peek() != &Token::Eof {
            // `solve var before var, var` (di-lex: Ident("solve"), Ident("before"))
            if let Token::Ident(ref s) = self.peek() {
                if s == "solve" {
                    self.advance();
                    let mut vars = Vec::new();
                    let first_var = self.expect_ident()?;
                    vars.push(first_var);
                    if let Token::Ident(ref s2) = self.peek() {
                        if s2 == "before" {
                            self.advance();
                            loop {
                                let v = self.expect_ident()?;
                                vars.push(v);
                                if self.peek() == &Token::Comma {
                                    self.advance();
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                    self.skip_semi();
                    body.push(ConstraintItem::SolveBefore { vars });
                    continue;
                }
            }
            // `if (cond) { items } else { items }` (F12) ATAU
            // `if (cond) expr;` / `if (cond) expr; else expr;` — constraint
            // kondisional TANPA braces (legal SV; contoh spid_common line
            // 1254 `if (wrap_test == 1'b 0) size >= 1 && size <= 256;`).
            if self.peek() == &Token::If {
                self.advance();
                self.expect(Token::LParen)?;
                let cond = self.parse_expr(0)?;
                self.expect(Token::RParen)?;
                let mut then = Vec::new();
                let mut els = Vec::new();
                if self.peek() == &Token::LBrace {
                    self.advance();
                    then = self.parse_constraint_items()?;
                    self.expect(Token::RBrace)?;
                } else {
                    // Tanpa braces: parse satu ekspresi constraint sebagai
                    // branch-then.
                    match self.parse_expr(0) {
                        Ok(e) => {
                            self.skip_semi();
                            then.push(ConstraintItem::Expr(e));
                        }
                        Err(_) => {
                            loop {
                                match self.peek() {
                                    Token::Semi => { self.advance(); break; }
                                    Token::RBrace | Token::Eof => break,
                                    _ => { self.advance(); }
                                }
                            }
                        }
                    }
                }
                if self.peek() == &Token::Else {
                    self.advance();
                    if self.peek() == &Token::LBrace {
                        self.advance();
                        els = self.parse_constraint_items()?;
                        self.expect(Token::RBrace)?;
                    } else {
                        match self.parse_expr(0) {
                            Ok(e) => {
                                self.skip_semi();
                                els.push(ConstraintItem::Expr(e));
                            }
                            Err(_) => {
                                loop {
                                    match self.peek() {
                                        Token::Semi => { self.advance(); break; }
                                        Token::RBrace | Token::Eof => break,
                                        _ => { self.advance(); }
                                    }
                                }
                            }
                        }
                    }
                }
                body.push(ConstraintItem::If { cond, then, els });
                continue;
            }
            // `soft expr;` (LANG-31) — constraint soft (best-effort): boleh
            // dilanggar bila bertentangan dengan hard constraint.
            if self.peek() == &Token::Soft {
                self.advance();
                match self.parse_expr(0) {
                    Ok(expr) => {
                        self.skip_semi();
                        body.push(ConstraintItem::Soft(expr));
                    }
                    Err(_) => {
                        // Recovery: skip ke ';' atau '}'
                        loop {
                            match self.peek() {
                                Token::Semi => { self.advance(); break; }
                                Token::RBrace | Token::Eof => break,
                                _ => { self.advance(); }
                            }
                        }
                    }
                }
                continue;
            }
            // Ekspresi constraint (relasional/equality/inside/dist)
            // parse_expr might fail on complex constraint expressions;
            // if so, skip to ';' to recover
            match self.parse_expr(0) {
                Ok(expr) => {
                    self.skip_semi();
                    body.push(ConstraintItem::Expr(expr));
                }
                Err(_) => {
                    // Error in constraint expression — skip to ';' or '}'
                    loop {
                        match self.peek() {
                            Token::Semi => { self.advance(); break; }
                            Token::RBrace | Token::Eof => break,
                            _ => { self.advance(); }
                        }
                    }
                }
            }
        }
        Ok(body)
    }

    pub(crate) fn parse_class_fast(&mut self) -> Result<(), SimError> {
        self.advance(); // consume 'class'
        // Skip optional #(type T = ...) parameter list
        if self.peek() == &Token::Hash {
            self.advance();
            if self.peek() == &Token::LParen {
                let _ = self.skip_balanced_paren_light();
            }
        }
        // Collect class name
        if let Token::Ident(name) = self.peek() {
            self.class_names.insert(*name);
            self.advance();
        }
        // Skip extends clause
        if self.peek() == &Token::Extends {
            self.advance();
            if matches!(self.peek(), Token::Ident(_)) {
                self.advance(); // base class name
            }
            // Skip optional #(.PARAM(...)) after base class
            if self.peek() == &Token::Hash {
                self.advance();
                if self.peek() == &Token::LParen {
                    let _ = self.skip_balanced_paren_light();
                }
            }
        }
        // Skip implements clause (SV-2005)
        if matches!(self.peek(), Token::Ident(s) if s.as_str() == "implements") {
            self.advance();
            loop {
                if matches!(self.peek(), Token::Ident(_)) { self.advance(); }
                if self.peek() == &Token::Comma { self.advance(); } else { break; }
            }
        }
        self.skip_semi();
        // Fast-skip class body until endclass
        loop {
            match self.peek() {
                Token::EndClass | Token::Eof => {
                    if self.peek() == &Token::EndClass { self.advance(); }
                    // Skip optional ': name' after endclass
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) {
                            self.advance();
                        }
                    }
                    break;
                }
                Token::Class => {
                    // Nested class — skip body
                    self.skip_class_body();
                }
                _ => {
                    if self.peek() == &Token::LParen && self.peek_ahead(1) == &Token::Star {
                        self.skip_attribute();
                    } else {
                        self.advance();
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn parse_class(&mut self) -> Result<ClassDecl, SimError> {
        self.advance(); // consume 'class'
        let mut type_params = Vec::new();
        if self.peek() == &Token::Hash {
            self.advance();
            self.expect(Token::LParen)?;
            loop {
                if self.peek() == &Token::RParen {
                    break;
                }
                self.expect(Token::Type)?;
                let tp_name = self.expect_ident()?;
                let default_type = if self.peek() == &Token::BlockingAssign {
                    self.advance();
                    Some(self.parse_type_expr()?)
                } else {
                    None
                };
                type_params.push(TypeParam {
                    name: tp_name,
                    default_type,
                });
                if self.peek() == &Token::Comma {
                    self.advance();
                } else {
                    break;
                }
            }
            self.expect(Token::RParen)?;
        }
        let name = self.expect_ident()?;
        let extends = if self.peek() == &Token::Extends {
            self.advance();
            let base_name = self.expect_ident()?;
            // Handle parameterized base class: extends Base #(.PARAM(value), ...)
            if self.peek() == &Token::Hash {
                self.advance();
                if self.peek() == &Token::LParen {
                    self.skip_balanced_paren()?;
                }
            }
            Some(base_name)
        } else {
            None
        };
        self.expect(Token::Semi)?;
        self.type_param_names = type_params.iter().map(|tp| tp.name).collect();
        let mut members = Vec::new();
        loop {
            match self.peek() {
                Token::EndClass => {
                    self.advance();
                    // Handle optional 'endclass : name'
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) {
                            self.advance();
                        }
                    }
                    break;
                }
                Token::Function => {
                    match self.parse_function(false) {
                        Ok(f) => members.push(ClassMember::Function(f)),
                        Err(_) => { let _ = self.skip_until_semi_or_end(); }
                    }
                }
                Token::Ident(s) if s == "extern" => {
                    // Extern prototype — consume until semicolon
                    self.advance(); // extern
                    // Handle 'extern local', 'extern protected', 'extern virtual', etc.
                    loop {
                        match self.peek() {
                            Token::Ident(n) if n == "local" || n == "protected" || n == "virtual" => {
                                self.advance();
                            }
                            _ => break,
                        }
                    }
                    // Handle extern constraint: `extern constraint name;`
                    if self.peek() == &Token::Constraint {
                        self.advance(); // consume constraint
                        let _ = self.expect_ident(); // consume name
                        self.skip_semi();
                        continue;
                    }
                    if !matches!(self.peek(), Token::Function | Token::Task) {
                        continue;
                    }
                    self.advance(); // consume function/task
                    let mut depth = 0i32;
                    loop {
                        match self.peek() {
                            Token::Semi if depth <= 0 => { self.advance(); break; }
                            Token::LParen => { depth += 1; self.advance(); }
                            Token::RParen => { depth -= 1; self.advance(); }
                            Token::EndClass | Token::Eof => break,
                            _ => { self.advance(); }
                        }
                    }
                }
                Token::Virtual => {
                    self.advance();
                    match self.peek() {
                        Token::Function => {
                            match self.parse_function(true) {
                                Ok(f) => members.push(ClassMember::Function(f)),
                                Err(_) => { let _ = self.skip_until_semi_or_end(); }
                            }
                        }
                        Token::Task => {
                            match self.parse_task(true) {
                                Ok(t) => members.push(ClassMember::Task(t)),
                                Err(_) => { let _ = self.skip_until_semi_or_end(); }
                            }
                        }
                        _ => {
                            match self.parse_decl() {
                                Ok(mut decl) => {
                                    for n in &mut decl.names { n.is_rand = false; }
                                    members.push(ClassMember::Decl(decl));
                                }
                                Err(_) => { let _ = self.skip_until_semi_or_end(); }
                            }
                        }
                    }
                }
                Token::Task => {
                    match self.parse_task(false) {
                        Ok(t) => members.push(ClassMember::Task(t)),
                        Err(_) => { let _ = self.skip_until_semi_or_end(); }
                    }
                }
                Token::Input
                | Token::Output
                | Token::Inout
                | Token::Reg
                | Token::Logic
                | Token::Wire
                | Token::Int
                | Token::Integer
                | Token::Signed
                | Token::Bit
                | Token::Byte
                | Token::Shortint
                | Token::Longint
                | Token::Time
                | Token::String
                | Token::Mailbox
                | Token::Semaphore
                | Token::Real
                | Token::RealTime
                | Token::Enum
                | Token::Struct
                | Token::Union
                | Token::Wand
                | Token::Wor
                | Token::Tri
                | Token::Tri0
                | Token::Tri1
                | Token::TriAnd
                | Token::TriOr
                | Token::Supply0
                | Token::Supply1 => {
                    match self.parse_decl() {
                        Ok(mut decl) => {
                            for n in &mut decl.names {
                                n.is_rand = false;
                            }
                            members.push(ClassMember::Decl(decl));
                        }
                        Err(_) => {
                            let _ = self.skip_until_semi_or_end();
                        }
                    }
                }
                Token::Rand | Token::RandC => {
                    self.advance();
                    match self.parse_decl() {
                        Ok(mut decl) => {
                            for n in &mut decl.names { n.is_rand = true; }
                            members.push(ClassMember::Decl(decl));
                        }
                        Err(_) => { let _ = self.skip_until_semi_or_end(); }
                    }
                }
                Token::Ident(name) if self.type_param_names.contains(name) => {
                    let tp_name = *name;
                    self.advance();
                    let decl_expr_range = if self.peek() == &Token::LBrack {
                        self.parse_range()?
                    } else {
                        None
                    };
                    let mut extra_packed: Vec<(ExprRange, Option<Range>)> = Vec::new();
                    while self.peek() == &Token::LBrack && self.peek_ahead(1) == &Token::Colon {
                        if let Some(er) = self.parse_range()? {
                            extra_packed.push((er, None));
                        }
                    }
                    let names = self.parse_decl_names(decl_expr_range, extra_packed)?;
                    self.skip_semi();
                    members.push(ClassMember::Decl(maria_ast::types::Decl {
                        dtype: DataType::UserDefined(tp_name),
                        kind: maria_ast::types::DeclKind::Logic,
                        names,
                    }));
                }
                Token::Ident(s) if s == "pure" && self.peek_ahead(1) == &Token::Virtual => {
                    self.advance(); // pure
                    loop {
                        match self.peek() {
                            Token::Semi => { self.advance(); break; }
                            Token::EndClass | Token::Eof => break,
                            _ => { self.advance(); }
                        }
                    }
                }
                Token::Ident(_) => {
                    // F18: field class bertipe user-defined (`my_env env;`).
                    // Sebelumnya hanya type-param Ident yang diparse sebagai
                    // field — tipe user-defined lain di-skip diam-diam oleh
                    // fallback `_ => advance()`, sehingga class fields kosong
                    // dan `env = new("env", this)` di build_phase tidak bisa
                    // resolve class (lihat resolve_new_class_hint).
                    match self.parse_decl() {
                        Ok(mut decl) => {
                            for n in &mut decl.names {
                                n.is_rand = false;
                            }
                            members.push(ClassMember::Decl(decl));
                        }
                        Err(_) => {
                            let _ = self.skip_until_semi_or_end();
                        }
                    }
                }
                Token::Constraint => {
                    self.advance();
                    let cname = self.expect_ident()?;
                    self.expect(Token::LBrace)?;
                    let body = self.parse_constraint_items()?;
                    self.expect(Token::RBrace)?;
                    members.push(ClassMember::Constraint { name: cname, body, is_static: false });
                }
                Token::Let => {
                    // LANG-40: `let` di dalam class.
                    match self.parse_let_decl() {
                        Ok(ld) => members.push(ClassMember::Let(ld)),
                        Err(_) => { let _ = self.skip_until_semi_or_end(); }
                    }
                }
                Token::Static => {
                    // LANG-32: `static constraint name { ... }` — block constraint
                    // dibagi antar semua instance class (IEEE 1800-2017 §18.5.10).
                    self.advance();
                    if self.peek() == &Token::Constraint {
                        self.advance();
                        let cname = self.expect_ident()?;
                        self.expect(Token::LBrace)?;
                        let body = self.parse_constraint_items()?;
                        self.expect(Token::RBrace)?;
                        members.push(ClassMember::Constraint { name: cname, body, is_static: true });
                    }
                    // Bukan static constraint (static var/function/task) — token
                    // Static sudah dikonsumsi; member diparse di iterasi berikutnya.
                }
                Token::Class => {
                    // Nested class — skip entire body to matching endclass
                    self.skip_class_body();
                }
                _ => {
                    if self.peek() == &Token::Eof {
                        // Hit EOF without finding endclass — abort to prevent infinite loop
                        break;
                    }
                    self.advance();
                }
            }
        }
        self.type_param_names.clear();
        Ok(ClassDecl {
            name,
            extends,
            type_params,
            members,
        })
    }
}
