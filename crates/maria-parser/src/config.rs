//! ──────────────────────────────────────────────────────────────────────────────
//! CATATAN: File ini adalah bagian dari pemisahan parser.rs (SRP Refactoring).
//! Tanggung jawab: Parsing config declaration (config ... endconfig).
//!
//! Fungsi:
//!   - parse_config_decl() — parsing konfigurasi library binding
//!
//! ──────────────────────────────────────────────────────────────────────────────

use super::Parser;
use maria_ast::*;
use maria_core::error::SimError;
use maria_core::intern::Symbol;
use crate::lexer::*;

impl Parser {
    pub(crate) fn parse_config_decl(&mut self) -> Result<ConfigDecl, SimError> {
        self.advance(); // consume 'config'
        let name = self.expect_ident()?;
        self.skip_semi();

        let mut design_top = None;
        let mut default_liblist = None;
        let mut rules = Vec::new();

        loop {
            match self.peek() {
                Token::EndConfig => {
                    self.advance();
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) {
                            self.advance();
                        }
                    }
                    break;
                }
                Token::Design => {
                    self.advance();
                    if let Token::Ident(name) = self.peek().clone() {
                        self.advance();
                        design_top = Some(name);
                    }
                    self.skip_semi();
                }
                Token::Default => {
                    self.advance();
                    if self.peek() == &Token::Liblist {
                        self.advance();
                        if let Token::Ident(name) = self.peek().clone() {
                            self.advance();
                            default_liblist = Some(name.as_str().to_string());
                        }
                    }
                    self.skip_semi();
                }
                Token::Instance => {
                    self.advance();
                    let mut instance_path = String::new();
                    if let Token::Ident(s) = self.peek().clone() {
                        self.advance();
                        instance_path = s.as_str().to_string();
                    }
                    // Handle hierarchical paths: top.sub1
                    while self.peek() == &Token::Dot {
                        self.advance();
                        if let Token::Ident(s) = self.peek().clone() {
                            self.advance();
                            instance_path.push('.');
                            instance_path.push_str(s.as_str());
                        }
                    }
                    if self.peek() == &Token::Liblist {
                        self.advance();
                        if let Token::Ident(lib) = self.peek().clone() {
                            self.advance();
                            rules.push(ConfigRule::InstanceLiblist {
                                instance: Symbol::intern(&instance_path),
                                liblist: lib.as_str().to_string(),
                            });
                        }
                    }
                    self.skip_semi();
                }
                Token::Cell => {
                    self.advance();
                    let mut cell_name = String::new();
                    if let Token::Ident(s) = self.peek().clone() {
                        self.advance();
                        cell_name = s.as_str().to_string();
                    }
                    if self.peek() == &Token::Liblist {
                        self.advance();
                        if let Token::Ident(lib) = self.peek().clone() {
                            self.advance();
                            rules.push(ConfigRule::CellLiblist {
                                cell: Symbol::intern(&cell_name),
                                liblist: lib.as_str().to_string(),
                            });
                        }
                    }
                    self.skip_semi();
                }
                Token::Use => {
                    self.advance();
                    if self.peek() == &Token::Liblist {
                        self.advance();
                        if let Token::Ident(lib) = self.peek().clone() {
                            self.advance();
                            rules.push(ConfigRule::UseLiblist { liblist: lib.as_str().to_string() });
                        }
                    }
                    self.skip_semi();
                }
                _ => {
                    self.advance();
                }
            }
        }

        Ok(ConfigDecl {
            name,
            design_top,
            default_liblist,
            rules,
        })
    }
}
