//! Parser — definisi tipe level file/package: alias, struct, union, enum, package.
//! 1 file = 1 tanggung jawab.

use super::Parser;
use crate::ast::*;
use crate::lexer::Tok;
use crate::MvError;

impl Parser {
    pub(crate) fn parse_typedef_alias(&mut self) -> Result<Typedef, MvError> {
        self.expect(&Tok::Type)?;
        // F11: posisi nama typedef (untuk error type-check ber-posisi)
        let (l, c) = self.pos_line();
        let name = self.expect_ident()?;
        self.expect(&Tok::BlockingAssign)?;
        let ty = self.parse_type()?;
        Ok(Typedef::Alias {
            name,
            ty,
            line: l,
            col: c,
        })
    }

    pub(crate) fn parse_typedef(&mut self) -> Result<Typedef, MvError> {
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
                Ok(Typedef::Struct {
                    name,
                    packed,
                    fields,
                    line: l,
                    col: c,
                })
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
                    members.push(EnumMember {
                        name: mname,
                        value,
                        line: ml,
                        col: mc,
                    });
                    self.eat(&Tok::Comma);
                }
                Ok(Typedef::Enum {
                    name,
                    width,
                    members,
                    line: l,
                    col: c,
                })
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
                Ok(Typedef::Union {
                    name,
                    packed: true,
                    fields,
                    line: l,
                    col: c,
                })
            }
            _ => {
                let (l, c) = self.pos_line();
                Err(MvError::new(l, c, "typedef tidak dikenal".to_string()))
            }
        }
    }

    pub(crate) fn parse_field(&mut self) -> Result<Field, MvError> {
        let (l, c) = self.pos_line();
        let mut names = vec![self.expect_ident()?];
        while self.eat(&Tok::Comma) {
            names.push(self.expect_ident()?);
        }
        self.expect(&Tok::Colon)?;
        let ty = self.parse_type()?;
        Ok(Field {
            names,
            ty,
            line: l,
            col: c,
        })
    }

    // ── Package ──
    pub(crate) fn parse_package(&mut self) -> Result<Package, MvError> {
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
                    return Err(MvError::new(
                        l,
                        c,
                        "package tidak ditutup dengan '}'".to_string(),
                    ));
                }
                Tok::Type => typedefs.push(self.parse_typedef_alias()?),
                Tok::Packed | Tok::Struct | Tok::Enum | Tok::Union => {
                    typedefs.push(self.parse_typedef()?)
                }
                Tok::Const => {
                    self.expect(&Tok::Const)?;
                    let cname = self.expect_ident()?;
                    let ty = if self.eat(&Tok::Colon) {
                        Some(self.parse_type()?)
                    } else {
                        None
                    };
                    self.expect(&Tok::BlockingAssign)?;
                    let value = self.parse_expr()?;
                    consts.push((cname, ty, value));
                }
                _ => {
                    let (l, c) = self.pos_line();
                    return Err(MvError::new(
                        l,
                        c,
                        format!("item package tidak dikenal: {:?}", self.peek()),
                    ));
                }
            }
        }
        Ok(Package {
            name,
            typedefs,
            consts,
            line: l,
            col: c,
        })
    }
}