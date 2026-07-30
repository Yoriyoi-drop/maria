//! ──────────────────────────────────────────────────────────────────────────────
//! CATATAN: File ini adalah bagian dari pemisahan parser.rs (SRP Refactoring).
//! Tanggung jawab: Parsing package declaration (package ... endpackage).
//!
//! Fungsi:
//!   - parse_package_decl() — parsing deklarasi package
//! ──────────────────────────────────────────────────────────────────────────────

use super::Parser;
use crate::ast::*;
use crate::error::SimError;
use crate::intern::Symbol;
use crate::parser::lexer::*;

impl Parser {
    pub(crate) fn parse_package_decl(&mut self) -> Result<PackageDecl, SimError> {
        self.advance(); // consume 'package'
        let name = self.expect_ident()?;
        self.skip_semi();
        let mut items = Vec::new();
        loop {
            match self.peek() {
                Token::EndPackage => {
                    self.advance();
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) {
                            self.advance();
                        }
                    }
                    break;
                }
                Token::Eof => return Err(SimError::parse("unexpected EOF in package")),
                _ => {
                    match self.peek() {
                        Token::Param | Token::Parameter | Token::LocalParam => {
                            let is_localparam = self.peek() == &Token::LocalParam;
                            self.advance();

                            // Handle 'parameter type X = type_expr'
                            if self.peek() == &Token::Type {
                                self.advance();
                                let pname = self.expect_ident()?;
                                let type_default = if self.peek() == &Token::BlockingAssign {
                                    self.advance();
                                    Some(self.parse_type_expr()?)
                                } else {
                                    None
                                };
                                self.skip_semi();
                                items.push(PackageItem::Param(ParamDecl {
                                    name: pname,
                                    dtype: None,
                                    range: None,
                                    default: None,
                                    is_localparam,
                                    is_type_param: true,
                                    type_default,
                                }));
                                continue;
                            }

                            // Parse optional built-in type keyword
                            let mut dtype = None;
                            match self.peek() {
                                Token::Integer => {
                                    self.advance();
                                    dtype = Some(DataType::Integer);
                                }
                                Token::Int => {
                                    self.advance();
                                    dtype = Some(DataType::Int);
                                }
                                Token::Reg => {
                                    self.advance();
                                    dtype = Some(DataType::Logic);
                                }
                                Token::Logic => {
                                    self.advance();
                                    dtype = Some(DataType::Logic);
                                }
                                Token::Bit => {
                                    self.advance();
                                    dtype = Some(DataType::Bit);
                                }
                                Token::Byte => {
                                    self.advance();
                                    dtype = Some(DataType::Byte);
                                }
                                Token::Shortint => {
                                    self.advance();
                                    dtype = Some(DataType::Shortint);
                                }
                                Token::Longint => {
                                    self.advance();
                                    dtype = Some(DataType::Longint);
                                }
                                Token::Time => {
                                    self.advance();
                                    dtype = Some(DataType::Time);
                                }
                                _ => {}
                            }

                            // Handle signed/unsigned
                            if self.peek() == &Token::Signed {
                                self.advance();
                                let inner = dtype.take().unwrap_or(DataType::Int);
                                dtype = Some(DataType::Signed(Box::new(inner)));
                            }
                            if self.peek() == &Token::Unsigned {
                                self.advance();
                            }

                            // Handle user-defined type (ident followed by ident or [)
                            let mut type_ident = None;
                            if dtype.is_none() {
                                if let Token::Ident(s) = self.peek() {
                                    let ahead = self.peek_ahead(1).clone();
                                    if matches!(
                                        ahead,
                                        Token::Ident(_)
                                            | Token::LBrack
                                            | Token::Signed
                                            | Token::Unsigned
                                    ) {
                                        type_ident = Some(s.clone());
                                        self.advance();
                                    }
                                }
                            }

                            // Parse optional range [msb:lsb]
                            let mut range = None;
                            if self.peek() == &Token::LBrack {
                                self.advance();
                                let msb = self.parse_expr(0)?;
                                self.expect(Token::Colon)?;
                                let lsb = self.parse_expr(0)?;
                                self.expect(Token::RBrack)?;
                                range = Some((msb, lsb));
                                // Skip additional packed dimensions: [a:b][c:d]
                                while self.peek() == &Token::LBrack {
                                    self.advance();
                                    self.parse_expr(0)?;
                                    self.expect(Token::Colon)?;
                                    self.parse_expr(0)?;
                                    self.expect(Token::RBrack)?;
                                }
                            }

                            // Parse parameter name(s)
                            loop {
                                let pk = self.peek().clone();
                                let pname = match &pk {
                                    Token::Ident(s) => {
                                        self.advance();
                                        s.clone()
                                    }
                                    _ => break,
                                };
                                // Skip unpacked array dimension after name: name [N]
                                if self.peek() == &Token::LBrack
                                    && self.peek_ahead(1) != &Token::Colon
                                {
                                    self.advance();
                                    self.parse_expr(0)?;
                                    self.expect(Token::RBrack)?;
                                }
                                let default = if self.peek() == &Token::BlockingAssign {
                                    self.advance();
                                    Some(self.parse_expr(0)?)
                                } else {
                                    None
                                };
                                let resolved_dtype = if let Some(t) = &type_ident {
                                    Some(DataType::UserDefined(*t))
                                } else {
                                    dtype.clone()
                                };
                                items.push(PackageItem::Param(ParamDecl {
                                    name: pname,
                                    dtype: resolved_dtype,
                                    range: range.clone(),
                                    default,
                                    is_localparam,
                                    is_type_param: false,
                                    type_default: None,
                                }));
                                if self.peek() == &Token::Comma {
                                    self.advance();
                                } else {
                                    break;
                                }
                            }
                            self.skip_semi();
                        }
                        Token::Function => {
                            items.push(PackageItem::Function(self.parse_function(false)?));
                        }
                        Token::Task => {
                            items.push(PackageItem::Task(self.parse_task(false)?));
                        }
                        Token::Typedef => {
                            // Check for 'typedef class' (forward declaration)
                            if matches!(self.peek_ahead(1), Token::Class | Token::Virtual) {
                                self.advance(); // consume 'typedef'
                                while self.peek() != &Token::Semi && self.peek() != &Token::Eof {
                                    self.advance();
                                }
                                self.skip_semi();
                            } else {
                                let td = self.parse_typedef()?;
                                self.typedef_names.insert(td.name.clone());
                                self.package_tdefs
                                    .entry(name.clone())
                                    .or_default()
                                    .push(td.name.clone());
                                items.push(PackageItem::Typedef(td));
                            }
                        }
                        Token::Import => {
                            self.advance();
                            let pkg = self.expect_ident()?;
                            self.expect(Token::Scope)?;
                    let item = if self.peek() == &Token::Star {
                        self.advance();
                        Symbol::intern("*")
                    } else {
                        self.expect_ident()?
                    };
                            // Register imported typedef names
                            if let Some(tdefs) = self.package_tdefs.get(&pkg) {
                                if item == "*" {
                                    for name in tdefs {
                                        self.typedef_names.insert(*name);
                                    }
                                } else if tdefs.contains(&item) {
                                    self.typedef_names.insert(item);
                                }
                            }
                            self.skip_semi();
                            items.push(PackageItem::Import { package: pkg, item });
                        }
                        Token::Export => {
                            self.advance();
                            let pkg = self.expect_ident()?;
                            self.expect(Token::Scope)?;
                    let item = if self.peek() == &Token::Star {
                        self.advance();
                        Symbol::intern("*")
                    } else {
                        self.expect_ident()?
                    };
                            self.skip_semi();
                            items.push(PackageItem::Export { package: pkg, item });
                        }
                        _ => {
                            let decl = self.parse_decl()?;
                            items.push(PackageItem::Decl(decl));
                        }
                    }
                }
            }
        }
        Ok(PackageDecl { name, items })
    }
}
