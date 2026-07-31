//! ──────────────────────────────────────────────────────────────────────────────
//! CATATAN: File ini adalah bagian dari pemisahan parser.rs (SRP Refactoring).
//! Tanggung jawab: Parsing ekspresi (expression parsing).
//!
//! Fungsi:
//!   - is_type_token()       — cek apakah token saat ini adalah tipe data
//!   - parse_dist_item()     — parsing dist item untuk constraint randomization
//!   - parse_expr()          — parsing expression (Pratt parser)
//!   - parse_primary_expr()  — parsing primary expression (literal, ident, dll.)
//! ──────────────────────────────────────────────────────────────────────────────

use super::Parser;
use crate::ast::*;
use crate::ast::types::const_eval_simple;
use crate::error::SimError;
use crate::intern::Symbol;
use crate::parser::lexer::*;

impl Parser {
    pub(crate) fn is_type_token(&self) -> bool {
        matches!(
            self.peek(),
            Token::Bit
                | Token::Logic
                | Token::Int
                | Token::Integer
                | Token::Byte
                | Token::Shortint
                | Token::Longint
                | Token::Time
                | Token::Reg
                | Token::Real
                | Token::RealTime
                | Token::String
                | Token::Struct
                | Token::Union
                | Token::Enum
        )
    }

    pub(crate) fn parse_dist_item(&mut self) -> Result<DistItem, SimError> {
        // dist item: expr := weight  or  expr :/ weight  or  [lo:hi] := weight  or  [lo:hi] :/ weight
        if self.peek() == &Token::LBrack
            && self.peek_ahead(1) != &Token::RBrack
            && self.peek_ahead(1) != &Token::Colon
        {
            // Range item: [lo:hi] := weight or [lo:hi] :/ weight
            self.advance(); // [
            let lo = self.parse_expr(0)?;
            self.expect(Token::Colon)?;
            let hi = self.parse_expr(0)?;
            self.expect(Token::RBrack)?;
            if self.peek() == &Token::Equiv {
                // :=
                self.advance();
                let val = self.parse_expr(0)?;
                let weight = const_eval_simple(&val).unwrap_or(0) as u64;
                Ok(DistItem::Range(
                    Box::new(lo),
                    Box::new(hi),
                    DistWeight::Item(weight),
                ))
            } else if matches!(self.peek(), Token::Colon) && self.peek_ahead(1) == &Token::Slash {
                // :/
                self.advance(); // :
                self.advance(); // /
                let val = self.parse_expr(0)?;
                let weight = const_eval_simple(&val).unwrap_or(0) as u64;
                Ok(DistItem::Range(
                    Box::new(lo),
                    Box::new(hi),
                    DistWeight::Range(weight),
                ))
            } else {
                return Err(self.err("expected := or :/ after dist range"));
            }
        } else {
            // Single value: expr := weight or expr :/ weight
            let expr = self.parse_expr(0)?;
            if self.peek() == &Token::Equiv {
                // :=
                self.advance();
                let val = self.parse_expr(0)?;
                let weight = const_eval_simple(&val).unwrap_or(0) as u64;
                Ok(DistItem::Value(Box::new(expr), DistWeight::Item(weight)))
            } else if matches!(self.peek(), Token::Colon) && self.peek_ahead(1) == &Token::Slash {
                // :/
                self.advance(); // :
                self.advance(); // /
                let val = self.parse_expr(0)?;
                let weight = const_eval_simple(&val).unwrap_or(0) as u64;
                Ok(DistItem::Value(Box::new(expr), DistWeight::Range(weight)))
            } else {
                return Err(self.err("expected := or :/ after dist item"));
            }
        }
    }

    pub(crate) fn parse_expr(&mut self, min_prec: usize) -> Result<Expr, SimError> {
        let mut lhs = self.parse_primary_expr()?;

        loop {
            let op_info = match self.peek() {
                Token::Plus => Some((9, BinaryOp::Add)),
                Token::Minus => Some((9, BinaryOp::Sub)),
                Token::Star => Some((10, BinaryOp::Mul)),
                Token::Slash => Some((10, BinaryOp::Div)),
                Token::Percent => Some((10, BinaryOp::Mod)),
                Token::StarStar => Some((11, BinaryOp::Power)),
                Token::Eq => Some((6, BinaryOp::Eq)),
                Token::Neq => Some((6, BinaryOp::Neq)),
                Token::Equiv => Some((6, BinaryOp::CaseEq)),
                Token::NotEquiv => Some((6, BinaryOp::CaseNeq)),
                Token::CaseEq => Some((6, BinaryOp::EqWild)),
                Token::CaseNeq => Some((6, BinaryOp::NeqWild)),
                Token::WildcardEq => Some((6, BinaryOp::EqWild)),
                Token::WildcardNeq => Some((6, BinaryOp::NeqWild)),
                Token::Lt => Some((7, BinaryOp::Lt)),
                Token::Le | Token::NonBlockingAssign => Some((7, BinaryOp::Le)),
                Token::Gt => Some((7, BinaryOp::Gt)),
                Token::Ge => Some((7, BinaryOp::Ge)),
                Token::Shl => Some((8, BinaryOp::Shl)),
                Token::Shr => Some((8, BinaryOp::Shr)),
                Token::Sshl => Some((8, BinaryOp::Sshl)),
                Token::Sshr => Some((8, BinaryOp::Sshr)),
                Token::Amp => Some((5, BinaryOp::BitAnd)),
                Token::Pipe => Some((3, BinaryOp::BitOr)),
                Token::Caret => Some((4, BinaryOp::BitXor)),
                Token::CaretTilde => Some((4, BinaryOp::BitXnor)),
                Token::AmpAmp => Some((2, BinaryOp::LogicalAnd)),
                Token::PipePipe => Some((1, BinaryOp::LogicalOr)),
                Token::Question => {
                    self.advance();
                    let true_expr = self.parse_expr(0)?;
                    self.expect(Token::Colon)?;
                    let false_expr = self.parse_expr(0)?;
                    return Ok(Expr::TernaryOp {
                        cond: Box::new(lhs),
                        true_expr: Box::new(true_expr),
                        false_expr: Box::new(false_expr),
                    });
                }
                Token::Inside => {
                    if 7 < min_prec { break; }
                    self.advance();
                    self.expect(Token::LBrace)?;
                    let mut range_list = Vec::new();
                    if self.peek() != &Token::RBrace {
                        loop {
                            range_list.push(self.parse_expr(0)?);
                            if self.peek() == &Token::Comma { self.advance(); } else { break; }
                        }
                    }
                    self.expect(Token::RBrace)?;
                    lhs = Expr::Inside { expr: Box::new(lhs), range_list };
                    continue;
                }
                Token::Ident(ref s) if s == "dist" => {
                    if 7 < min_prec { break; }
                    self.advance();
                    self.expect(Token::LBrace)?;
                    let mut items = Vec::new();
                    if self.peek() != &Token::RBrace {
                        loop {
                            items.push(self.parse_dist_item()?);
                            if self.peek() == &Token::Comma { self.advance(); } else { break; }
                        }
                    }
                    self.expect(Token::RBrace)?;
                    lhs = Expr::Dist { expr: Box::new(lhs), items };
                    continue;
                }
                Token::Ident(ref s) if s == "with" => {
                    if 6 < min_prec { break; }
                    self.advance();
                    let with_expr = if self.peek() == &Token::LBrace {
                        self.advance();
                        let mut exprs: Vec<Expr> = Vec::new();
                        while self.peek() != &Token::RBrace && self.peek() != &Token::Eof {
                            let e = self.parse_expr(0)?;
                            exprs.push(e);
                            if self.peek() == &Token::Semi { self.advance(); }
                        }
                        self.expect(Token::RBrace)?;
                        exprs.into_iter()
                            .reduce(|acc, e| Expr::BinaryOp {
                                op: BinaryOp::LogicalAnd, lhs: Box::new(acc), rhs: Box::new(e),
                            })
                            .unwrap_or(Expr::Value(Value::Decimal(1)))
                    } else {
                        self.expect(Token::LParen)?;
                        let e = self.parse_expr(0)?;
                        self.expect(Token::RParen)?;
                        e
                    };
                    let old_lhs = std::mem::replace(&mut lhs, Expr::Value(Value::Decimal(0)));
                    match old_lhs {
                        Expr::MethodCall { obj, method, args, with_clause: None } => {
                            lhs = Expr::MethodCall { obj, method, args, with_clause: Some(Box::new(with_expr)) };
                        }
                        _ => return Err(self.err("'with' clause can only follow a method call")),
                    }
                    continue;
                }
                Token::LBrack => {
                    self.advance();
                    if self.peek() == &Token::RBrack { self.advance(); continue; }
                    let first = self.parse_expr(0)?;
                    if self.peek() == &Token::Colon {
                        self.advance();
                        let second = self.parse_expr(0)?;
                        self.expect(Token::RBrack)?;
                        lhs = Expr::RangeSelect { expr: Box::new(lhs), msb: Box::new(first), lsb: Box::new(second) };
                    } else if self.peek() == &Token::PlusColon {
                        self.advance();
                        let width = self.parse_expr(0)?;
                        self.expect(Token::RBrack)?;
                        lhs = Expr::PartSelect { expr: Box::new(lhs), base: Box::new(first), width: Box::new(width) };
                    } else if self.peek() == &Token::MinusColon {
                        self.advance();
                        let width = self.parse_expr(0)?;
                        self.expect(Token::RBrack)?;
                        lhs = Expr::PartSelect {
                            expr: Box::new(lhs),
                            base: Box::new(Expr::BinaryOp {
                                op: BinaryOp::Sub,
                                lhs: Box::new(first.clone()),
                                rhs: Box::new(Expr::BinaryOp {
                                    op: BinaryOp::Sub,
                                    lhs: Box::new(width.clone()),
                                    rhs: Box::new(Expr::Value(Value::Decimal(1))),
                                }),
                            }),
                            width: Box::new(width),
                        };
                    } else {
                        self.expect(Token::RBrack)?;
                        lhs = Expr::BitSelect { expr: Box::new(lhs), index: Box::new(first) };
                    }
                    continue;
                }
                Token::Dot => {
                    self.advance();
                    let member = self.expect_ident()?;
                    if self.peek() == &Token::LParen {
                        self.advance();
                        let mut args = Vec::new();
                        if self.peek() != &Token::RParen {
                            loop {
                                args.push(self.parse_expr(0)?);
                                if self.peek() == &Token::Comma { self.advance(); } else { break; }
                            }
                        }
                        self.expect(Token::RParen)?;
                        lhs = Expr::MethodCall { obj: Box::new(lhs), method: member, args, with_clause: None };
                    } else {
                        lhs = Expr::MemberAccess { obj: Box::new(lhs), field: member };
                    }
                    continue;
                }
                _ => None,
            };

            match op_info {
                Some((prec, op)) => {
                    if prec < min_prec { break; }
                    self.advance();
                    let rhs = self.parse_expr(prec + 1)?;
                    lhs = Expr::BinaryOp { op, lhs: Box::new(lhs), rhs: Box::new(rhs) };
                }
                None => break,
            }
        }

        Ok(lhs)
    }

    pub(crate) fn parse_primary_expr(&mut self) -> Result<Expr, SimError> {
        self.push_depth()?;
        let result = self.parse_primary_expr_impl();
        self.pop_depth();
        result
    }

    fn parse_primary_expr_impl(&mut self) -> Result<Expr, SimError> {
        let tok = self.peek().clone();
        match tok {
            Token::Dollar => {
                self.advance();
                let name_tok = self.peek().clone();
                let name_sym = match &name_tok {
                    Token::Ident(n) => { self.advance(); *n }
                    Token::Time => { self.advance(); Symbol::intern("time") }
                    Token::Real => { self.advance(); Symbol::intern("real") }
                    Token::RealTime => { self.advance(); Symbol::intern("realtime") }
                    Token::Signed => { self.advance(); Symbol::intern("signed") }
                    Token::Unsigned => { self.advance(); Symbol::intern("unsigned") }
                    _ => return Err(self.err("expected system function name")),
                };
                // SAFETY: Stack buffer 128 bytes cukup untuk semua system function SV
                // ($display, $monitor, $urandom, dll.). Symbol.as_str() selalu valid UTF-8.
                // Untuk nama >127 chars (sangat jarang), fallback ke format!()
                let full_name = {
                    let name_str = name_sym.as_str();
                    if name_str.len() > 127 {
                        // Nama panjang: pakai heap alloc (sangat jarang)
                        Symbol::intern(&format!("${}", name_str))
                    } else {
                        let mut buf = [0u8; 128];
                        buf[0] = b'$';
                        let name_bytes = name_str.as_bytes();
                        // Safe: name_bytes valid UTF-8, buf[1..] cukup untuk copy
                        unsafe {
                            std::ptr::copy_nonoverlapping(name_bytes.as_ptr(), buf.as_mut_ptr().add(1), name_bytes.len());
                            Symbol::intern(std::str::from_utf8_unchecked(&buf[..name_bytes.len() + 1]))
                        }
                    }
                };
                if self.peek() == &Token::LParen {
                    self.advance();
                    let mut args = Vec::new();
                    if self.peek() != &Token::RParen {
                        loop {
                            args.push(self.parse_expr(0)?);
                            if self.peek() == &Token::Comma { self.advance(); } else { break; }
                        }
                    }
                    self.expect(Token::RParen)?;
                    Ok(Expr::FuncCall { name: full_name, args })
                } else {
                    Ok(Expr::Ident { name: full_name, line: 0, col: 0 })
                }
            }
            Token::Ident(name) => {
                let line = self.peek_line();
                let col = self.peek_col();
                self.advance();
                // pkg::item resolution
                if self.peek() == &Token::Scope {
                    self.advance();
                    let item = self.expect_ident()?;
                    if self.peek() == &Token::LParen {
                        self.advance();
                        let mut args = Vec::new();
                        if self.peek() != &Token::RParen {
                            loop {
                                args.push(self.parse_expr(0)?);
                                if self.peek() == &Token::Comma { self.advance(); } else { break; }
                            }
                        }
                        self.expect(Token::RParen)?;
                        return Ok(Expr::FuncCall { name: Symbol::intern(&format!("{}::{}", name, item)), args });
                    }
                    return Ok(Expr::ScopedIdent { package: name, item });
                }
                // Class#(Type)::method resolution
                if self.peek() == &Token::Hash {
                    self.advance();
                    self.expect(Token::LParen)?;
                    let mut type_specs = Vec::new();
                    loop {
                        if self.peek() == &Token::RParen { break; }
                        type_specs.push(self.parse_type_expr()?);
                        if self.peek() == &Token::Comma { self.advance(); } else { break; }
                    }
                    self.expect(Token::RParen)?;
                    let class_prefix = if type_specs.is_empty() {
                        name
                    } else {
                        let type_strs: Vec<String> = type_specs.iter().map(|dt| dt.to_string()).collect();
                        let suffix = type_strs.join(",");
                        Symbol::intern(&format!("{}#{}", name, suffix))
                    };
                    if self.peek() == &Token::Scope {
                        self.advance();
                        let item = self.expect_ident()?;
                        if self.peek() == &Token::LParen {
                            if self.peek() == &Token::Quote && self.peek_ahead(1) == &Token::LParen {
                                self.advance(); self.advance();
                                let expr = self.parse_expr(0)?;
                                self.expect(Token::RParen)?;
                                return Ok(Expr::Cast { dtype: Symbol::intern(&format!("{}::{}", class_prefix, item)), expr: Box::new(expr) });
                            }
                            self.advance();
                            let mut args = Vec::new();
                            if self.peek() != &Token::RParen {
                                loop {
                                    args.push(self.parse_expr(0)?);
                                    if self.peek() == &Token::Comma { self.advance(); } else { break; }
                                }
                            }
                            self.expect(Token::RParen)?;
                            return Ok(Expr::FuncCall { name: Symbol::intern(&format!("{}::{}", class_prefix, item)), args });
                        }
                        return Ok(Expr::ScopedIdent { package: class_prefix, item });
                    }
                    return Ok(Expr::Ident { name: class_prefix, line: 0, col: 0 });
                }
                // Type cast: type_name'(expr)
                if self.peek() == &Token::Quote {
                    self.advance();
                    self.expect(Token::LParen)?;
                    let expr = self.parse_expr(0)?;
                    self.expect(Token::RParen)?;
                    return Ok(Expr::Cast { dtype: name, expr: Box::new(expr) });
                }
                if self.peek() == &Token::LParen {
                    self.advance();
                    let mut args = Vec::new();
                    if self.peek() != &Token::RParen {
                        loop {
                            args.push(self.parse_expr(0)?);
                            if self.peek() == &Token::Comma { self.advance(); } else { break; }
                        }
                    }
                    self.expect(Token::RParen)?;
                    Ok(Expr::FuncCall { name, args })
                } else {
                    Ok(Expr::Ident { name, line, col })
                }
            }
            Token::Number { value, base, width, is_signed } => {
                self.advance();
                if self.peek() == &Token::Quote && self.peek_ahead(1) == &Token::LParen {
                    self.advance(); self.advance();
                    let expr = self.parse_expr(0)?;
                    self.expect(Token::RParen)?;
                    let n = value.as_str().parse::<i64>().unwrap_or(0);
                    return Ok(Expr::Cast { dtype: Symbol::intern(&format!("{}", n)), expr: Box::new(expr) });
                }
                let val = if let Some(base) = base {
                    match base {
                        2 => Expr::Value(Value::Binary { bits: value.as_str().to_string(), width, is_signed }),
                        8 => Expr::Value(Value::Octal { bits: value.as_str().to_string(), width, is_signed }),
                        10 => Expr::Value(Value::Decimal(value.as_str().parse::<i64>().unwrap_or(0))),
                        16 => Expr::Value(Value::Hex { bits: value.as_str().to_string(), width, is_signed }),
                        _ => Expr::Value(Value::Decimal(value.as_str().parse::<i64>().unwrap_or(0))),
                    }
                } else {
                    if let Ok(n) = value.as_str().parse::<i64>() { Expr::Value(Value::Decimal(n)) }
                    else { Expr::Ident { name: value, line: 0, col: 0 } }
                };
                Ok(val)
            }
            Token::RealNum(s) => { self.advance(); Ok(Expr::Value(Value::Real(s.as_str().parse::<f64>().unwrap_or(0.0)))) }
            Token::StringLit(s) => { self.advance(); Ok(Expr::String(s.as_str().to_string())) }
            Token::New => {
                self.advance();
                if self.peek() == &Token::LBrack {
                    self.advance();
                    let size = self.parse_expr(0)?;
                    self.expect(Token::RBrack)?;
                    let _init = if self.peek() == &Token::LParen { self.advance(); let val = self.parse_expr(0)?; self.expect(Token::RParen)?; Some(Box::new(val)) } else { None };
                    Ok(Expr::FuncCall { name: Symbol::intern("new"), args: vec![size] })
                } else {
                    self.expect(Token::LParen)?;
                    let mut args = Vec::new();
                    if self.peek() != &Token::RParen {
                        loop {
                            args.push(self.parse_expr(0)?);
                            if self.peek() == &Token::Comma { self.advance(); } else { break; }
                        }
                    }
                    self.expect(Token::RParen)?;
                    Ok(Expr::FuncCall { name: Symbol::intern("new"), args })
                }
            }
            Token::This => { self.advance(); Ok(Expr::Ident { name: Symbol::intern("this"), line: 0, col: 0 }) }
            Token::Null => { self.advance(); Ok(Expr::Null) }
            Token::Plus | Token::Minus | Token::Tilde | Token::Amp | Token::Pipe | Token::Caret
            | Token::TildeAmp | Token::TildePipe | Token::CaretTilde => {
                let saved_tok = tok.clone();
                self.advance();
                let op = match saved_tok {
                    Token::Plus => UnaryOp::Plus, Token::Minus => UnaryOp::Minus, Token::Tilde => UnaryOp::BitNot,
                    Token::Amp => UnaryOp::ReductionAnd, Token::Pipe => UnaryOp::ReductionOr,
                    Token::Caret => UnaryOp::ReductionXor, Token::TildeAmp => UnaryOp::ReductionNand,
                    Token::TildePipe => UnaryOp::ReductionNor, Token::CaretTilde => UnaryOp::ReductionXnor,
                    _ => unreachable!(),
                };
                let expr = self.parse_expr(12)?;
                Ok(Expr::UnaryOp { op, expr: Box::new(expr) })
            }
            Token::Not => { self.advance(); let expr = self.parse_expr(12)?; Ok(Expr::UnaryOp { op: UnaryOp::Not, expr: Box::new(expr) }) }
            Token::LBrace => {
                self.advance();
                if matches!(self.peek(), Token::Shl | Token::Shr | Token::Sshl | Token::Sshr) {
                    let op = if matches!(self.peek(), Token::Shl | Token::Sshl) { String::from("<<") } else { String::from(">>") };
                    self.advance();
                    let slice_size = if !matches!(self.peek(), Token::LBrace) { Some(Box::new(self.parse_expr(0)?)) } else { None };
                    self.expect(Token::LBrace)?;
                    let mut slices = Vec::new();
                    loop {
                        if self.peek() == &Token::RBrace { break; }
                        slices.push(self.parse_expr(0)?);
                        if self.peek() == &Token::Comma { self.advance(); } else { break; }
                    }
                    self.expect(Token::RBrace)?; self.expect(Token::RBrace)?;
                    return Ok(Expr::StreamingConcat { op, slice_size, slices });
                }
                let mut exprs = Vec::new();
                loop {
                    if self.peek() == &Token::RBrace { break; }
                    let item = self.parse_expr(0)?;
                    if self.peek() == &Token::LBrace {
                        self.advance();
                        let mut inner_exprs = Vec::new();
                        loop {
                            inner_exprs.push(self.parse_expr(0)?);
                            if self.peek() == &Token::Comma { self.advance(); } else { break; }
                        }
                        self.expect(Token::RBrace)?;
                        let inner = if inner_exprs.len() == 1 {
                            inner_exprs.into_iter().next().unwrap()
                        } else {
                            Expr::Concat(inner_exprs)
                        };
                        exprs.push(Expr::Replicate { count: Box::new(item), expr: Box::new(inner) });
                    } else { exprs.push(item); }
                    if self.peek() == &Token::Comma { self.advance(); } else { break; }
                }
                self.expect(Token::RBrace)?;
                if exprs.len() == 1 { Ok(exprs.into_iter().next().unwrap()) }
                else { Ok(Expr::Concat(exprs)) }
            }
            Token::LParen => { self.advance(); let expr = self.parse_expr(0)?; self.expect(Token::RParen)?; Ok(Expr::Paren(Box::new(expr))) }
            Token::FillLit(val) => { self.advance(); Ok(Expr::FillLit(val)) }
            Token::Auto => { self.advance(); Ok(Expr::Ident { name: Symbol::intern("automatic"), line: 0, col: 0 }) }
            Token::String => { self.advance(); Ok(Expr::Ident { name: Symbol::intern("string"), line: 0, col: 0 }) }
            Token::Class | Token::EndClass => { self.advance(); Ok(Expr::Ident { name: Symbol::intern("class"), line: 0, col: 0 }) }
            Token::Quote => {
                self.advance();
                if self.peek() == &Token::LBrace {
                    self.advance();
                    let saved = self.pos.get();
                    let mut elements = Vec::new();
                    let mut ok = true;
                    loop {
                        if self.peek() == &Token::RBrace {
                            self.advance();
                            break;
                        }
                        if self.peek() == &Token::Eof {
                            ok = false;
                            break;
                        }
                        match self.parse_expr(0) {
                            Ok(first) => {
                                if self.peek() == &Token::Colon {
                                    // Named-field assignment pattern: fall back to skip
                                    ok = false;
                                    break;
                                }
                                elements.push(first);
                            }
                            Err(_) => {
                                ok = false;
                                break;
                            }
                        }
                        if self.peek() == &Token::Comma {
                            self.advance();
                        } else if self.peek() == &Token::RBrace {
                            self.advance();
                            break;
                        } else {
                            ok = false;
                            break;
                        }
                    }
                    if ok && !elements.is_empty() {
                        if elements.len() == 1 {
                            return Ok(elements.into_iter().next().unwrap());
                        }
                        return Ok(Expr::Concat(elements));
                    }
                    // Fallback: skip to matching brace (behavior lama)
                    self.pos.set(saved);
                    let mut depth = 1usize;
                    while depth > 0 && self.peek() != &Token::Eof {
                        match self.peek() {
                            Token::LBrace => {
                                depth += 1;
                                self.advance();
                            }
                            Token::RBrace => {
                                depth -= 1;
                                if depth > 0 {
                                    self.advance();
                                }
                            }
                            _ => {
                                self.advance();
                            }
                        }
                    }
                    if self.peek() == &Token::RBrace {
                        self.advance();
                    }
                    Ok(Expr::FillLit(crate::ir::LogicVal::Zero))
                } else {
                    Ok(Expr::FillLit(crate::ir::LogicVal::Zero))
                }
            }
            Token::Increment => { self.advance(); let expr = self.parse_expr(12)?; Ok(Expr::BinaryOp { op: BinaryOp::Add, lhs: Box::new(expr), rhs: Box::new(Expr::Value(Value::Decimal(1))) }) }
            Token::Decrement => { self.advance(); let expr = self.parse_expr(12)?; Ok(Expr::BinaryOp { op: BinaryOp::Sub, lhs: Box::new(expr), rhs: Box::new(Expr::Value(Value::Decimal(1))) }) }
            Token::Void | Token::Int | Token::Integer | Token::Logic | Token::Bit | Token::Byte
            | Token::Shortint | Token::Longint | Token::Time | Token::Signed | Token::Unsigned | Token::Real | Token::RealTime => {
                self.advance();
                let type_name = match tok {
                    Token::Void => "void", Token::Int => "int", Token::Integer => "integer",
                    Token::Logic => "logic", Token::Bit => "bit", Token::Byte => "byte",
                    Token::Shortint => "shortint", Token::Longint => "longint", Token::Time => "time",
                    Token::Signed => "signed", Token::Unsigned => "unsigned", Token::Real => "real", Token::RealTime => "realtime",
                    _ => unreachable!(),
                };
                if self.peek() == &Token::Quote {
                    self.advance(); self.expect(Token::LParen)?;
                    let expr = self.parse_expr(0)?; self.expect(Token::RParen)?;
                    Ok(Expr::Cast { dtype: Symbol::intern(type_name), expr: Box::new(expr) })
                } else { Ok(Expr::Ident { name: Symbol::intern(type_name), line: 0, col: 0 }) }
            }
            ref other => Err(self.err(format!("expected expression, found {:?}", other))),
        }
    }
}
