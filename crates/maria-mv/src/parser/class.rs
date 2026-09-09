//! Parser — class, function/task, arg-list, constraint (inside/dist/solve).
//! 1 file = 1 tanggung jawab.

use super::Parser;
use crate::ast::*;
use crate::lexer::Tok;
use crate::MvError;

impl Parser {
    /// `class Name [extends Base] { field/constraint/func/task }` (MARIA-HDL.md §8).
    /// Keyword `class`/`extends`/`field`/`rand`/`constraint` di-lex sebagai
    /// `Tok::Ident` — dibedakan lewat string.
    pub(crate) fn parse_class(&mut self) -> Result<MClass, MvError> {
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
                    return Err(MvError::new(
                        l,
                        c,
                        "class tidak ditutup dengan '}'".to_string(),
                    ));
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
                    return Err(MvError::new(
                        l,
                        c,
                        format!("item class tidak dikenal: {:?}", self.peek()),
                    ));
                }
            }
        }
        Ok(cls)
    }

    // ── Function / Task ──
    pub(crate) fn parse_func(&mut self) -> Result<MFunc, MvError> {
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
        Ok(MFunc {
            name,
            args,
            ret,
            body,
            line: l,
            col: c,
        })
    }

    pub(crate) fn parse_task(&mut self) -> Result<MTask, MvError> {
        self.expect(&Tok::Task)?;
        let (l, c) = self.pos_line();
        let name = self.expect_ident()?;
        let args = self.parse_arg_list()?;
        let body = self.parse_stmt_block()?;
        Ok(MTask {
            name,
            args,
            body,
            line: l,
            col: c,
        })
    }

    pub(crate) fn parse_arg_list(
        &mut self,
    ) -> Result<Vec<(String, MvType, Option<Dir>, Option<Expr>)>, MvError> {
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
            // default arg opsional: `b : int = 4`
            let default = if self.eat(&Tok::BlockingAssign) {
                Some(self.parse_expr()?)
            } else {
                None
            };
            args.push((name, ty, dir, default));
            self.eat(&Tok::Comma);
        }
        Ok(args)
    }

    // ── Constraint lanjutan (F12) ──

    /// Blok `{ item, item, ... }` di dalam `constraint c { ... }`.
    pub(crate) fn parse_constraint_block(&mut self) -> Result<Vec<ConstraintItem>, MvError> {
        self.expect(&Tok::LBrace)?;
        let mut items = Vec::new();
        while !self.eat(&Tok::RBrace) {
            items.push(self.parse_constraint_item()?);
            self.eat(&Tok::Comma);
        }
        Ok(items)
    }

    /// Satu item constraint: `if/else`, `solve var before a, b`, atau ekspresi.
    pub(crate) fn parse_constraint_item(&mut self) -> Result<ConstraintItem, MvError> {
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
    pub(crate) fn parse_dist_item(&mut self) -> Result<DistItem, MvError> {
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
    pub(crate) fn parse_dist_weight(&mut self) -> Result<(bool, Expr), MvError> {
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
}