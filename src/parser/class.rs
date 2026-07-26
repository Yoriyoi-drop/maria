//! ──────────────────────────────────────────────────────────────────────────────
//! CATATAN: File ini adalah bagian dari pemisahan parser.rs (SRP Refactoring).
//! Tanggung jawab: Parsing class declaration (class ... endclass).
//!
//! Fungsi:
//!   - parse_class() — parsing deklarasi class dengan type params, extends, members
//! ──────────────────────────────────────────────────────────────────────────────

use super::Parser;
use crate::ast::*;
use crate::error::SimError;
use crate::intern::Symbol;
use crate::parser::lexer::*;

impl Parser {
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
        self.type_param_names = type_params.iter().map(|tp| tp.name.clone()).collect();
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
                    members.push(ClassMember::Function(self.parse_function(false)?));
                }
                Token::Ident(s) if s == "extern" => {
                    // Extern prototype — consume until semicolon
                    self.advance(); // extern
                    if matches!(self.peek(), Token::Ident(n) if n == "local" || n == "protected") {
                        self.advance();
                    }
                    if self.peek() == &Token::Virtual { self.advance(); }
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
                            members.push(ClassMember::Function(self.parse_function(true)?));
                        }
                        Token::Task => {
                            members.push(ClassMember::Task(self.parse_task(true)?));
                        }
                        _ => {
                            let mut decl = self.parse_decl()?;
                            for n in &mut decl.names {
                                n.is_rand = false;
                            }
                            members.push(ClassMember::Decl(decl));
                        }
                    }
                }
                Token::Task => {
                    members.push(ClassMember::Task(self.parse_task(false)?));
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
                    let mut decl = self.parse_decl()?;
                    for n in &mut decl.names {
                        n.is_rand = false;
                    }
                    members.push(ClassMember::Decl(decl));
                }
                Token::Rand | Token::RandC => {
                    self.advance();
                    let mut decl = self.parse_decl()?;
                    for n in &mut decl.names {
                        n.is_rand = true;
                    }
                    members.push(ClassMember::Decl(decl));
                }
                Token::Ident(name) if self.type_param_names.contains(&name) => {
                    let tp_name = name.clone();
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
                    members.push(ClassMember::Decl(crate::ast::types::Decl {
                        dtype: DataType::UserDefined(tp_name),
                        kind: crate::ast::types::DeclKind::Logic,
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
                Token::Constraint => {
                    self.advance();
                    let cname = self.expect_ident()?;
                    self.expect(Token::LBrace)?;
                    let mut body = Vec::new();
                    while self.peek() != &Token::RBrace {
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
                        let expr = self.parse_expr(0)?;
                        self.skip_semi();
                        body.push(ConstraintItem::Expr(expr));
                    }
                    self.advance(); // consume '}'
                    members.push(ClassMember::Constraint { name: cname, body });
                }
                _ => {
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
