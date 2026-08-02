// Parser submodule: declaration parsing
// Tanggung jawab: parse_decl, parse_decl_names, parse_enum_members, parse_struct_body,
// parse_typedef, parse_scoped_type_name, parse_type_expr, parse_param_list, parse_range

use super::Parser;
use crate::ast::*;
use crate::ast::types::const_eval_simple;
use crate::error::SimError;
use crate::intern::Symbol;
use crate::parser::lexer::*;

impl Parser {
    pub(crate) fn parse_scoped_type_name(&mut self) -> Option<DataType> {
        // Check if the next tokens are Ident(::Ident)? — a user-defined type name
        // that should be treated as the type of a declaration (e.g., wire pkg::type varname)
        if let Token::Ident(s) = self.peek() {
            let s = *s;
            let ahead = self.peek_ahead(1).clone();
            if ahead == Token::Scope {
                let pkg = s;
                self.advance(); // consume package name
                self.advance(); // consume ::
                if let Token::Ident(t) = self.peek() {
                    let type_name = *t;
                    self.advance();
                    Some(DataType::UserDefined(Symbol::intern(&format!("{}::{}", pkg, type_name))))
                } else {
                    None
                }
            } else if matches!(ahead, Token::Ident(_)) {
                self.advance();
                Some(DataType::UserDefined(s))
            } else {
                None
            }
        } else {
            None
        }
    }

    pub(crate) fn parse_decl(&mut self) -> Result<Decl, SimError> {
        let is_const = self.peek() == &Token::Const;
        if is_const {
            self.advance(); // consume 'const'
        }
        // Skip optional 'var' keyword
        if self.peek() == &Token::Var {
            self.advance(); // consume 'var'
        }
        let kind = match self.peek() {
            Token::Wire => DeclKind::Wire,
            Token::Wand => DeclKind::Wand,
            Token::Wor => DeclKind::Wor,
            Token::Tri => DeclKind::Tri,
            Token::Tri0 => DeclKind::Tri0,
            Token::Tri1 => DeclKind::Tri1,
            Token::TriAnd => DeclKind::TriAnd,
            Token::TriOr => DeclKind::TriOr,
            Token::Supply0 => DeclKind::Supply0,
            Token::Supply1 => DeclKind::Supply1,
            Token::Reg => DeclKind::Reg,
            Token::Logic => DeclKind::Logic,
            Token::Int => DeclKind::Int,
            Token::Integer => DeclKind::Integer,
            Token::Bit | Token::Byte | Token::Shortint | Token::Longint | Token::Time => {
                let dt = match self.peek() {
                    Token::Bit => DataType::Bit,
                    Token::Byte => DataType::Byte,
                    Token::Shortint => DataType::Shortint,
                    _ => DataType::Longint,
                };
                self.advance();
                let mut dtype = dt;
                if self.peek() == &Token::Signed { self.advance(); dtype = DataType::Signed(Box::new(dtype)); }
                if self.peek() == &Token::Unsigned { self.advance(); }
                let decl_expr_range = if self.peek() == &Token::LBrack { self.parse_range()? } else { None };
                let mut extra_packed: Vec<(ExprRange, Option<Range>)> = Vec::new();
                while self.peek_is_packed_dim() {
                    if let Some(er) = self.parse_range()? {
                        extra_packed.push((er, None));
                    }
                }
                let names = self.parse_decl_names(decl_expr_range, extra_packed)?;
                self.skip_semi();
                return Ok(Decl { dtype, kind: DeclKind::Logic, names });
            }
            Token::Enum => {
                self.advance();
                let base = match self.peek() {
                    Token::Bit | Token::Logic | Token::Int | Token::Integer
                        | Token::Byte | Token::Shortint | Token::Longint | Token::Time => {
                        let dt = match self.peek() {
                            Token::Bit => DataType::Bit,
                            Token::Logic => DataType::Logic,
                            Token::Int => DataType::Int,
                            Token::Integer => DataType::Integer,
                            Token::Byte => DataType::Byte,
                            Token::Shortint => DataType::Shortint,
                            _ => DataType::Longint,
                        };
                        self.advance();
                        let dt = if self.peek() == &Token::Signed { self.advance(); DataType::Signed(Box::new(dt)) } else { dt };
                        Some(Box::new(dt))
                    }
                    _ => None,
                };
                let decl_expr_range = if base.is_some() && self.peek() == &Token::LBrack {
                    self.parse_range()?
                } else {
                    None
                };
                let members = self.parse_enum_members()?;
                let mut extra_packed: Vec<(ExprRange, Option<Range>)> = Vec::new();
                while self.peek_is_packed_dim() {
                    if let Some(er) = self.parse_range()? {
                        extra_packed.push((er, None));
                    }
                }
                let names = self.parse_decl_names(decl_expr_range, extra_packed)?;
                self.skip_semi();
                return Ok(Decl { dtype: DataType::EnumType { base, members }, kind: DeclKind::Logic, names });
            }
            Token::Struct => {
                self.advance();
                if matches!(self.peek(), Token::Ident(s) if s == "packed") { self.advance(); }
                let members = self.parse_struct_body()?;
                let names = self.parse_decl_names(None, vec![])?;
                self.skip_semi();
                return Ok(Decl { dtype: DataType::StructType { members }, kind: DeclKind::Logic, names });
            }
            Token::Union => {
                self.advance();
                if matches!(self.peek(), Token::Ident(s) if s == "packed") { self.advance(); }
                let members = self.parse_struct_body()?;
                let names = self.parse_decl_names(None, vec![])?;
                self.skip_semi();
                return Ok(Decl { dtype: DataType::UnionType { members }, kind: DeclKind::Logic, names });
            }
            Token::String => {
                self.advance();
                let names = self.parse_decl_names(None, vec![])?;
                self.skip_semi();
                return Ok(Decl { dtype: DataType::String, kind: DeclKind::Reg, names });
            }
            Token::Real => {
                self.advance();
                let names = self.parse_decl_names(None, vec![])?;
                self.skip_semi();
                return Ok(Decl { dtype: DataType::Real, kind: DeclKind::Reg, names });
            }
            Token::WReal => {
                self.advance();
                let names = self.parse_decl_names(None, vec![])?;
                self.skip_semi();
                return Ok(Decl { dtype: DataType::Real, kind: DeclKind::Wire, names });
            }
            Token::RealTime => {
                self.advance();
                let names = self.parse_decl_names(None, vec![])?;
                self.skip_semi();
                return Ok(Decl { dtype: DataType::Realtime, kind: DeclKind::Reg, names });
            }
            Token::Mailbox => {
                self.advance();
                let names = self.parse_decl_names(None, vec![])?;
                self.skip_semi();
                return Ok(Decl { dtype: DataType::UserDefined(Symbol::intern("__mailbox")), kind: DeclKind::Reg, names });
            }
            Token::Semaphore => {
                self.advance();
                let names = self.parse_decl_names(None, vec![])?;
                self.skip_semi();
                return Ok(Decl { dtype: DataType::UserDefined(Symbol::intern("__semaphore")), kind: DeclKind::Reg, names });
            }
            Token::Ident(_) => {
                let name = self.expect_ident()?;
                let mut dtype = DataType::UserDefined(name);
                // Handle scoped type: pkg::type
                if self.peek() == &Token::Scope {
                    self.advance();
                    let type_name = self.expect_ident()?;
                    dtype = DataType::UserDefined(Symbol::intern(&format!("{}::{}", match &dtype { DataType::UserDefined(s) => s.as_str(), _ => "", }, type_name)));
                }
                let decl_expr_range = if self.peek() == &Token::LBrack {
                    self.parse_range()?
                } else {
                    None
                };
                let mut extra_packed: Vec<(ExprRange, Option<Range>)> = Vec::new();
                while self.peek_is_packed_dim() {
                    if let Some(er) = self.parse_range()? {
                        extra_packed.push((er, None));
                    }
                }
                let names = self.parse_decl_names(decl_expr_range, extra_packed)?;
                self.skip_semi();
                return Ok(Decl { dtype, kind: DeclKind::Logic, names });
            }
            _ => return Err(self.err("expected wire/reg/logic/int/byte/shortint/longint/enum/struct/union/wand/wor/tri")),
        };
        self.advance();

        let mut dtype = match kind {
            DeclKind::Logic => DataType::Logic,
            DeclKind::Int => DataType::Int,
            DeclKind::Integer => DataType::Integer,
            _ => DataType::Logic,
        };

        if self.peek() == &Token::Signed {
            self.advance();
            dtype = DataType::Signed(Box::new(dtype));
        }
        if self.peek() == &Token::Unsigned {
            self.advance();
            // unsigned = default, no-op
        }

        let decl_expr_range = if self.peek() == &Token::LBrack {
            self.parse_range()?
        } else {
            None
        };

        // Handle scoped type name after wire/reg/logic: wire pkg::type varname
        // Only try when no range precedes (to avoid misinterpreting "wire [7:0] arr")
        // or when we see :: which is unambiguous scoped type
        let scoped_dtype = if matches!(self.peek(), Token::Ident(_))
            && (decl_expr_range.is_none() || self.peek_ahead(1) == &Token::Scope)
        {
            if let Some(sdt) = self.parse_scoped_type_name() {
                Some(sdt)
            } else {
                None
            }
        } else {
            None
        };
        let effective_dtype = scoped_dtype.unwrap_or(dtype);

        let mut extra_packed: Vec<(ExprRange, Option<Range>)> = Vec::new();
        while self.peek_is_packed_dim() {
            if let Some(er) = self.parse_range()? {
                extra_packed.push((er, None));
            }
        }

        let names = self.parse_decl_names(decl_expr_range, extra_packed)?;
        self.skip_semi();

        Ok(Decl {
            dtype: effective_dtype,
            kind,
            names,
        })
    }

    pub(crate) fn parse_decl_names(
        &mut self,
        decl_expr_range: Option<ExprRange>,
        extra_packed_dims: Vec<(ExprRange, Option<Range>)>,
    ) -> Result<Vec<DeclVar>, SimError> {
        let mut names = Vec::new();
        loop {
            let name_tok = self.peek().clone();
            match &name_tok {
                Token::Ident(name) => {
                    self.advance();
                    let mut is_dynamic = false;
                    let mut is_queue = false;
                    let mut is_associative = false;
                    let mut assoc_key_type: Option<DataType> = None;
                    let (var_expr_range, array_range, array_size_expr) =
                        if decl_expr_range.is_some() {
                        let ar = if self.peek() == &Token::LBrack {
                            if self.peek_ahead(1) == &Token::RBrack {
                                self.advance();
                                self.advance();
                                is_dynamic = true;
                                (None, None)
                            } else if self.peek_ahead(1) == &Token::Dollar
                                && self.peek_ahead(2) == &Token::RBrack
                            {
                                self.advance();
                                self.advance();
                                self.advance();
                                is_queue = true;
                                (None, None)
                            } else if self.peek_ahead(1) == &Token::Int {
                                // int-key associative array
                                self.advance(); // [
                                self.advance(); // int
                                self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::Int);
                                (None, None)
                            } else if self.peek_ahead(1) == &Token::String {
                                // string-key associative array
                                self.advance(); // [
                                self.advance(); // string
                                self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::String);
                                (None, None)
                            } else if self.peek_ahead(1) == &Token::Bit {
                                // bit-key associative array
                                self.advance();
                                self.advance();
                                self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::Bit);
                                (None, None)
                            } else if self.peek_ahead(1) == &Token::Logic {
                                // logic-key associative array
                                self.advance();
                                self.advance();
                                self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::Logic);
                                (None, None)
                            } else if self.peek_ahead(1) == &Token::Byte {
                                // byte-key associative array
                                self.advance();
                                self.advance();
                                self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::Byte);
                                (None, None)
                            } else if self.peek_ahead(1) == &Token::Shortint {
                                // shortint-key associative array
                                self.advance();
                                self.advance();
                                self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::Shortint);
                                (None, None)
                            } else if self.peek_ahead(1) == &Token::Longint {
                                // longint-key associative array
                                self.advance();
                                self.advance();
                                self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::Longint);
                                (None, None)
                            } else if self.peek_ahead(1) == &Token::Star
                                && self.peek_ahead(2) == &Token::RBrack
                            {
                                // wildcard [*] associative array
                                self.advance();
                                self.advance();
                                self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::Int);
                                (None, None)
                            } else if self.peek_ahead(1) == &Token::Colon
                                || self.peek_ahead(2) == &Token::Colon
                            {
                                // `[msb:lsb]` unpacked — bukan size-expr.
                                let er = self.parse_range()?;
                                let r = er.and_then(|er| {
                                    if let (Ok(m), Ok(l)) =
                                        (const_eval_simple(&er.msb), const_eval_simple(&er.lsb))
                                    {
                                        Some(Range {
                                            msb: m as usize,
                                            lsb: l as usize,
                                        })
                                    } else {
                                        None
                                    }
                                });
                                (r, None)
                            } else {
                                // `[N]` / `[Width]`: unpacked array size. Resolve
                                // literal sekarang; simpan ekspresi untuk parameter.
                                self.advance(); // [
                                let sz = self.parse_expr(0)?;
                                self.expect(Token::RBrack)?;
                                match const_eval_simple(&sz) {
                                    Ok(n) if n > 0 => (
                                        Some(Range {
                                            msb: (n - 1) as usize,
                                            lsb: 0,
                                        }),
                                        None,
                                    ),
                                    _ => (None, Some(sz)),
                                }
                            }
                        } else {
                            (None, None)
                        };
                        (decl_expr_range.clone(), ar.0, ar.1)
                    } else {
                        if self.peek() == &Token::LBrack {
                            if self.peek_ahead(1) == &Token::RBrack {
                                self.advance();
                                self.advance();
                                is_dynamic = true;
                                (None, None, None)
                            } else if self.peek_ahead(1) == &Token::Dollar
                                && self.peek_ahead(2) == &Token::RBrack
                            {
                                self.advance();
                                self.advance();
                                self.advance();
                                is_queue = true;
                                (None, None, None)
                            } else if self.peek_ahead(1) != &Token::Colon {
                                // `[N]` / `[Width]`: unpacked array size.
                                self.advance(); // [
                                let sz = self.parse_expr(0)?;
                                self.expect(Token::RBrack)?;
                                match const_eval_simple(&sz) {
                                    Ok(n) if n > 0 => (
                                        None,
                                        Some(Range {
                                            msb: (n - 1) as usize,
                                            lsb: 0,
                                        }),
                                        None,
                                    ),
                                    _ => (None, None, Some(sz)),
                                }
                            } else {
                                let ver = self.parse_range()?;
                                let ar = if self.peek() == &Token::LBrack {
                                    let er = self.parse_range()?;
                                    er.as_ref().and_then(|er| {
                                        if let (Ok(m), Ok(l)) =
                                            (const_eval_simple(&er.msb), const_eval_simple(&er.lsb))
                                        {
                                            Some(Range {
                                                msb: m as usize,
                                                lsb: l as usize,
                                            })
                                        } else {
                                            None
                                        }
                                    })
                                } else {
                                    None
                                };
                                (ver, ar, None)
                            }
                        } else {
                            (None, None, None)
                        }
                    };
                    let var_range = var_expr_range.as_ref().and_then(|er| {
                        if let (Ok(m), Ok(l)) =
                            (const_eval_simple(&er.msb), const_eval_simple(&er.lsb))
                        {
                            Some(Range {
                                msb: m as usize,
                                lsb: l as usize,
                            })
                        } else {
                            None
                        }
                    });
                    let init_expr = if self.peek() == &Token::BlockingAssign {
                        self.advance();
                        Some(self.parse_expr(0)?)
                    } else {
                        None
                    };
                    names.push(DeclVar {
                        name: *name,
                        range: var_range,
                        expr_range: var_expr_range,
                        array_range,
                        array_size_expr,
                        extra_packed_dims: extra_packed_dims.clone(),
                        is_dynamic,
                        is_queue,
                        is_associative,
                        assoc_key_type,
                        is_rand: false,
                        is_const: false,
                        expr: init_expr,
                    });
                }
                _ => break,
            }

            if self.peek() == &Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        Ok(names)
    }

    pub(crate) fn parse_enum_members(&mut self) -> Result<Vec<(Symbol, Option<Expr>)>, SimError> {
        self.expect(Token::LBrace)?;
        let mut members = Vec::new();
        loop {
            match self.peek() {
                Token::Ident(name) => {
                    let name = *name;
                    self.advance();
                    let range: Option<(i64, Option<i64>)> = if self.peek() == &Token::LBrack {
                        self.advance();
                        let lo = self.parse_enum_range_literal()?;
                        let hi = if self.peek() == &Token::Colon {
                            self.advance();
                            Some(self.parse_enum_range_literal()?)
                        } else {
                            None
                        };
                        self.expect(Token::RBrack)?;
                        Some((lo, hi))
                    } else {
                        None
                    };
                    let val = if matches!(self.peek(), Token::Eq | Token::BlockingAssign) {
                        self.advance();
                        Some(self.parse_expr(0)?)
                    } else {
                        None
                    };
                    match range {
                        None => members.push((name, val)),
                        Some((lo, hi)) => {
                            let indices: Vec<i64> = match hi {
                                // name[N] -> name0 .. nameN-1
                                None => (0..lo).collect(),
                                // name[N:M] -> nameN .. nameM (inclusive, direction follows N->M)
                                Some(m) => {
                                    if lo <= m {
                                        (lo..=m).collect()
                                    } else {
                                        (m..=lo).rev().collect()
                                    }
                                }
                            };
                            for (k, idx) in indices.into_iter().enumerate() {
                                let member_val = if k == 0 { val.clone() } else { None };
                                let member_name =
                                    Symbol::intern(&format!("{}{}", name.as_str(), idx));
                                members.push((member_name, member_val));
                            }
                        }
                    }
                }
                _ => {
                    return Err(self.err("expected identifier in enum"))
                }
            }
            if self.peek() == &Token::Comma {
                self.advance();
                continue;
            }
            break;
        }
        self.expect(Token::RBrace)?;
        Ok(members)
    }

    /// Parse integer literal untuk enum range (hanya literal konstan per LRM).
    fn parse_enum_range_literal(&mut self) -> Result<i64, SimError> {
        match self.peek().clone() {
            Token::Number { value, base, .. } => {
                let s = value.as_str().to_string();
                self.advance();
                let n = match base {
                    None => s.parse::<i64>(),
                    Some(10) => s.parse::<i64>(),
                    Some(b) => i64::from_str_radix(&s, b as u32),
                }
                .map_err(|_| self.err("invalid integer literal in enum range"))?;
                Ok(n)
            }
            _ => Err(self.err("expected integer literal in enum range")),
        }
    }

    pub(crate) fn parse_struct_body(&mut self) -> Result<Vec<StructMember>, SimError> {
        self.push_depth()?;
        let result = self.parse_struct_body_impl();
        self.pop_depth();
        result
    }

    fn parse_struct_body_impl(&mut self) -> Result<Vec<StructMember>, SimError> {
        self.expect(Token::LBrace)?;
        let mut members = Vec::new();
        loop {
            if self.peek() == &Token::RBrace {
                self.advance();
                return Ok(members);
            }
            let member_type = match self.peek() {
                Token::Logic => {
                    self.advance();
                    DataType::Logic
                }
                Token::Int => {
                    self.advance();
                    DataType::Int
                }
                Token::Integer => {
                    self.advance();
                    DataType::Integer
                }
                Token::Bit => {
                    self.advance();
                    DataType::Bit
                }
                Token::Byte => {
                    self.advance();
                    DataType::Byte
                }
                Token::Shortint => {
                    self.advance();
                    DataType::Shortint
                }
                Token::Longint => {
                    self.advance();
                    DataType::Longint
                }
                Token::Time => {
                    self.advance();
                    DataType::Time
                }
                Token::Reg => {
                    self.advance();
                    DataType::Logic
                }
                Token::Signed => {
                    self.advance();
                    let inner = match self.peek() {
                        Token::Bit => {
                            self.advance();
                            DataType::Bit
                        }
                        Token::Logic => {
                            self.advance();
                            DataType::Logic
                        }
                        Token::Int => {
                            self.advance();
                            DataType::Int
                        }
                        Token::Integer => {
                            self.advance();
                            DataType::Integer
                        }
                        Token::Byte => {
                            self.advance();
                            DataType::Byte
                        }
                        Token::Shortint => {
                            self.advance();
                            DataType::Shortint
                        }
                        Token::Longint => {
                            self.advance();
                            DataType::Longint
                        }
                        Token::Time => {
                            self.advance();
                            DataType::Time
                        }
                        _ => DataType::Logic,
                    };
                    DataType::Signed(Box::new(inner))
                }
                Token::Struct => {
                    self.advance();
                    if matches!(self.peek(), Token::Ident(s) if s == "packed") {
                        self.advance();
                    }
                    DataType::StructType {
                        members: self.parse_struct_body()?,
                    }
                }
                Token::Ident(name) => {
                    let name = *name;
                    self.advance();
                    // Handle scoped type: pkg::type
                    if self.peek() == &Token::Scope {
                        self.advance();
                        let type_name = self.expect_ident()?;
                        DataType::UserDefined(Symbol::intern(&format!("{}::{}", name, type_name)))
                    } else {
                        DataType::UserDefined(name)
                    }
                }
                _ => {
                    return Err(self.err("expected type in struct/union member"))
                }
            };
            let range = if self.peek() == &Token::LBrack {
                let er = self.parse_range()?;
                er.as_ref().and_then(|er| {
                    if let (Ok(m), Ok(l)) = (const_eval_simple(&er.msb), const_eval_simple(&er.lsb))
                    {
                        Some(Range {
                            msb: m as usize,
                            lsb: l as usize,
                        })
                    } else {
                        None
                    }
                })
            } else {
                None
            };
            self.skip_extra_packed_dims()?;
            let name = self.expect_ident()?;
            self.skip_semi();
            members.push(StructMember {
                name,
                dtype: Box::new(member_type),
                range,
            });
        }
    }

    pub(crate) fn parse_typedef(&mut self) -> Result<TypedefDecl, SimError> {
        self.advance(); // consume typedef
        let (name, dtype, range) = match self.peek() {
            Token::Enum => {
                self.advance();
                let base = match self.peek() {
                    Token::Bit
                    | Token::Logic
                    | Token::Int
                    | Token::Integer
                    | Token::Byte
                    | Token::Shortint
                    | Token::Longint
                    | Token::Time => {
                        let dt = match self.peek() {
                            Token::Bit => DataType::Bit,
                            Token::Logic => DataType::Logic,
                            Token::Int => DataType::Int,
                            Token::Integer => DataType::Integer,
                            Token::Byte => DataType::Byte,
                            Token::Shortint => DataType::Shortint,
                            _ => DataType::Longint,
                        };
                        self.advance();
                        let dt = if self.peek() == &Token::Signed {
                            self.advance();
                            DataType::Signed(Box::new(dt))
                        } else {
                            dt
                        };
                        if self.peek() == &Token::Unsigned {
                            self.advance();
                        }
                        Some(Box::new(dt))
                    }
                    // User-defined base type: typedef enum lc_state_t {...} or
                    // typedef enum pkg::type {...}
                    Token::Ident(name) => {
                        let name = *name;
                        self.advance();
                        let dtype = if self.peek() == &Token::Scope {
                            self.advance();
                            let type_name = self.expect_ident()?;
                            DataType::UserDefined(Symbol::intern(&format!(
                                "{}::{}",
                                name,
                                type_name
                            )))
                        } else {
                            DataType::UserDefined(name)
                        };
                        Some(Box::new(dtype))
                    }
                    _ => None,
                };
                if base.is_some() && self.peek() == &Token::LBrack {
                    self.parse_range()?;
                }
                let members = self.parse_enum_members()?;
                if let Token::Ident(name) = self.peek() {
                    let name = *name;
                    self.advance();
                    (name, DataType::EnumType { base, members }, None)
                } else {
                    return Err(self.err("expected name after typedef enum"));
                }
            }
            Token::Bit => {
                self.advance();
                let mut dtype = DataType::Bit;
                if self.peek() == &Token::Signed {
                    self.advance();
                    dtype = DataType::Signed(Box::new(dtype));
                }
                if self.peek() == &Token::Unsigned {
                    self.advance();
                }
                let range = if self.peek() == &Token::LBrack {
                    self.parse_range()?
                } else {
                    None
                };
                self.skip_extra_packed_dims()?;
                if let Token::Ident(name) = self.peek() {
                    let name = *name;
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(self.err("expected name after typedef bit"));
                }
            }
            Token::Byte => {
                self.advance();
                let mut dtype = DataType::Byte;
                if self.peek() == &Token::Signed {
                    self.advance();
                    dtype = DataType::Signed(Box::new(dtype));
                }
                if self.peek() == &Token::Unsigned {
                    self.advance();
                }
                let range = if self.peek() == &Token::LBrack {
                    self.parse_range()?
                } else {
                    None
                };
                self.skip_extra_packed_dims()?;
                if let Token::Ident(name) = self.peek() {
                    let name = *name;
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(self.err("expected name after typedef byte"));
                }
            }
            Token::Shortint => {
                self.advance();
                let mut dtype = DataType::Shortint;
                if self.peek() == &Token::Signed {
                    self.advance();
                    dtype = DataType::Signed(Box::new(dtype));
                }
                if self.peek() == &Token::Unsigned {
                    self.advance();
                }
                let range = if self.peek() == &Token::LBrack {
                    self.parse_range()?
                } else {
                    None
                };
                self.skip_extra_packed_dims()?;
                if let Token::Ident(name) = self.peek() {
                    let name = *name;
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(self.err("expected name after typedef shortint"));
                }
            }
            Token::Longint => {
                self.advance();
                let mut dtype = DataType::Longint;
                if self.peek() == &Token::Signed {
                    self.advance();
                    dtype = DataType::Signed(Box::new(dtype));
                }
                if self.peek() == &Token::Unsigned {
                    self.advance();
                }
                let range = if self.peek() == &Token::LBrack {
                    self.parse_range()?
                } else {
                    None
                };
                self.skip_extra_packed_dims()?;
                if let Token::Ident(name) = self.peek() {
                    let name = *name;
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(self.err("expected name after typedef longint"));
                }
            }
            Token::Time => {
                self.advance();
                let range = if self.peek() == &Token::LBrack {
                    self.parse_range()?
                } else {
                    None
                };
                self.skip_extra_packed_dims()?;
                if let Token::Ident(name) = self.peek() {
                    let name = *name;
                    self.advance();
                    (name, DataType::Time, range)
                } else {
                    return Err(self.err("expected name after typedef time"));
                }
            }
            Token::Int => {
                self.advance();
                let mut dtype = DataType::Int;
                if self.peek() == &Token::Signed {
                    self.advance();
                    dtype = DataType::Signed(Box::new(dtype));
                }
                if self.peek() == &Token::Unsigned {
                    self.advance();
                }
                let range = if self.peek() == &Token::LBrack {
                    self.parse_range()?
                } else {
                    None
                };
                self.skip_extra_packed_dims()?;
                if let Token::Ident(name) = self.peek() {
                    let name = *name;
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(self.err("expected name after typedef int"));
                }
            }
            Token::Integer => {
                self.advance();
                let mut dtype = DataType::Integer;
                if self.peek() == &Token::Signed {
                    self.advance();
                    dtype = DataType::Signed(Box::new(dtype));
                }
                if self.peek() == &Token::Unsigned {
                    self.advance();
                }
                let range = if self.peek() == &Token::LBrack {
                    self.parse_range()?
                } else {
                    None
                };
                self.skip_extra_packed_dims()?;
                if let Token::Ident(name) = self.peek() {
                    let name = *name;
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(self.err("expected name after typedef integer"));
                }
            }
            Token::Logic => {
                self.advance();
                let mut dtype = DataType::Logic;
                if self.peek() == &Token::Signed {
                    self.advance();
                    dtype = DataType::Signed(Box::new(dtype));
                }
                if self.peek() == &Token::Unsigned {
                    self.advance();
                }
                let range = if self.peek() == &Token::LBrack {
                    self.parse_range()?
                } else {
                    None
                };
                self.skip_extra_packed_dims()?;
                if let Token::Ident(name) = self.peek() {
                    let name = *name;
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(self.err("expected name after typedef logic"));
                }
            }
            Token::Reg => {
                self.advance();
                let dtype = DataType::Logic;
                let range = if self.peek() == &Token::LBrack {
                    self.parse_range()?
                } else {
                    None
                };
                self.skip_extra_packed_dims()?;
                if let Token::Ident(name) = self.peek() {
                    let name = *name;
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(self.err("expected name after typedef reg"));
                }
            }
            Token::Struct => {
                self.advance();
                if matches!(self.peek(), Token::Ident(s) if s == "packed") {
                    self.advance();
                }
                let members = self.parse_struct_body()?;
                if let Token::Ident(name) = self.peek() {
                    let name = *name;
                    self.advance();
                    (name, DataType::StructType { members }, None)
                } else {
                    return Err(self.err("expected name after typedef struct"));
                }
            }
            Token::Union => {
                self.advance();
                if matches!(self.peek(), Token::Ident(s) if s == "packed") {
                    self.advance();
                }
                let members = self.parse_struct_body()?;
                if let Token::Ident(name) = self.peek() {
                    let name = *name;
                    self.advance();
                    (name, DataType::UnionType { members }, None)
                } else {
                    return Err(self.err("expected name after typedef union"));
                }
            }
            // User-defined base type: typedef some_type_t name; or
            // typedef some_type_t [range] name; or typedef pkg::type name;
            Token::Ident(_) => {
                let type_name = self.expect_ident()?;
                let mut dtype = DataType::UserDefined(type_name);
                if self.peek() == &Token::Scope {
                    self.advance();
                    let t = self.expect_ident()?;
                    dtype = DataType::UserDefined(Symbol::intern(&format!(
                        "{}::{}",
                        match &dtype {
                            DataType::UserDefined(s) => s.as_str(),
                            _ => "",
                        },
                        t
                    )));
                }
                // Class parameterization: typedef some_class #(.P(...), ...) name;
                // Parameter values diabaikan untuk DataType::UserDefined.
                if self.peek() == &Token::Hash {
                    self.advance();
                    if self.peek() == &Token::LParen {
                        self.skip_balanced_paren_light()?;
                    }
                }
                let range = if self.peek() == &Token::LBrack {
                    self.parse_range()?
                } else {
                    None
                };
                self.skip_extra_packed_dims()?;
                if let Token::Ident(name) = self.peek() {
                    let name = *name;
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(self.err("expected name after typedef type"));
                }
            }
            _ => {
                return Err(self.err("expected type after typedef"))
            }
        };
        self.skip_semi();
        Ok(TypedefDecl { name, dtype, range })
    }

    pub(crate) fn parse_type_expr(&mut self) -> Result<DataType, SimError> {
        let dt = match self.peek() {
            Token::Bit => DataType::Bit,
            Token::Logic => DataType::Logic,
            Token::Int => DataType::Int,
            Token::Integer => DataType::Integer,
            Token::Byte => DataType::Byte,
            Token::Shortint => DataType::Shortint,
            Token::Longint => DataType::Longint,
            Token::Time => DataType::Time,
            Token::Reg => DataType::Logic,
            Token::Real => DataType::Real,
            Token::RealTime => DataType::Realtime,
            Token::String => DataType::String,
            Token::Ident(_) => {
                let name = self.expect_ident()?;
                DataType::UserDefined(name)
            }
            _ => return Err(self.err("expected type")),
        };
        self.advance();
        if self.peek() == &Token::Signed {
            self.advance();
            Ok(DataType::Signed(Box::new(dt)))
        } else {
            Ok(dt)
        }
    }

    pub(crate) fn parse_param_list(&mut self, params: &mut Vec<ParamDecl>) -> Result<(), SimError> {
        let mut is_localparam = false;
        loop {
            match self.peek() {
                Token::Param | Token::Parameter => {
                    is_localparam = false;
                    self.advance();
                }
                Token::LocalParam => {
                    is_localparam = true;
                    self.advance();
                }
                _ => {}
            }

            // Skip optional type keyword (integer, int, reg, logic, bit, string)
            let mut type_ident = None;
            match self.peek() {
                Token::Integer
                | Token::Int
                | Token::Reg
                | Token::Logic
                | Token::Bit
                | Token::String => {
                    self.advance();
                }
                Token::Ident(_)
                    if matches!(
                        self.peek_ahead(1),
                        Token::Ident(_) | Token::LBrack | Token::Scope
                    ) =>
                {
                    // User-defined type: ident followed by name, range, or ::
                    if let Token::Ident(s) = self.peek() {
                        type_ident = Some(*s);
                        self.advance();
                        // Handle scoped type: pkg::type
                        if self.peek() == &Token::Scope {
                            self.advance();
                            let _ = self.expect_ident();
                        }
                    }
                }
                _ => {}
            }

            // Handle signed/unsigned
            if self.peek() == &Token::Signed {
                self.advance();
            }
            if self.peek() == &Token::Unsigned {
                self.advance();
            }

            // Parse optional range(s): [msb:lsb] or [msb:lsb][msb:lsb]...
            let mut range = None;
            if self.peek() == &Token::LBrack {
                self.advance();
                let msb = self.parse_expr(0)?;
                self.expect(Token::Colon)?;
                let lsb = self.parse_expr(0)?;
                self.expect(Token::RBrack)?;
                range = Some((msb, lsb));
                // Skip additional packed dimensions [a:b] (used in packed arrays like logic [3:0][1:0])
                while self.peek() == &Token::LBrack {
                    self.advance();
                    self.parse_expr(0)?;
                    self.expect(Token::Colon)?;
                    self.parse_expr(0)?;
                    self.expect(Token::RBrack)?;
                }
            }

            let tok = self.peek().clone();
            match tok {
                Token::Ident(_) | Token::Int | Token::Integer | Token::Type | Token::LBrack => {}
                _ => break,
            }

            let is_type_param = self.peek() == &Token::Type;
            if is_type_param {
                self.advance(); // consume 'type'
            }

            let name_tok = self.peek().clone();
            let name = match &name_tok {
                Token::Ident(s) => {
                    self.advance();
                    s.as_str().to_string()
                }
                Token::Int => {
                    self.advance();
                    "int".to_string()
                }
                Token::Integer => {
                    self.advance();
                    "integer".to_string()
                }
                _ => break,
            };

            // Type param dari header (module m #(parameter type T = int))
            // juga harus terdaftar agar `T x;` di body diparse sebagai deklarasi.
            if is_type_param {
                self.module_type_params.insert(Symbol::intern(&name));
            }

            let mut dtype = None;
            if self.peek() == &Token::Signed {
                self.advance();
                dtype = Some(DataType::Signed(Box::new(DataType::Int)));
            }

            // Skip unpacked array dimension(s) after name:
            // name [N] atau name [msb:lsb] (multi-dimensi diperbolehkan)
            while self.peek() == &Token::LBrack {
                self.advance(); // [
                let _ = self.parse_expr(0);
                if self.peek() == &Token::Colon {
                    self.advance();
                    let _ = self.parse_expr(0);
                }
                self.expect(Token::RBrack)?;
            }

            let default = if self.peek() == &Token::BlockingAssign {
                self.advance();
                if is_type_param {
                    // Parse default type expression: logic [7:0], bit, int, etc.
                    let _ = self.parse_type_expr()?;
                    // Skip optional range after type: logic [7:0]
                    if self.peek() == &Token::LBrack {
                        self.parse_range()?;
                    }
                    // For MVP, store dummy expression; width resolved in elaborator
                    Some(Expr::Value(Value::Decimal(0)))
                } else {
                    Some(self.parse_expr(0)?)
                }
            } else {
                None
            };

            let type_default = None; // Type default parsing TBD for full feature

            // Use type_ident as UserDefined dtype if set
            let resolved_dtype = type_ident
                .as_ref()
                .map(|t| DataType::UserDefined(*t))
                .or(dtype);

            params.push(ParamDecl {
                name: Symbol::intern(&name),
                dtype: resolved_dtype,
                range,
                default,
                is_localparam,
                is_type_param,
                type_default,
            });

            if self.peek() == &Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        Ok(())
    }

    pub(crate) fn parse_range(&mut self) -> Result<Option<ExprRange>, SimError> {
        self.expect(Token::LBrack)?;
        let msb = self.parse_expr(0)?;
        self.expect(Token::Colon)?;
        let lsb = self.parse_expr(0)?;
        self.expect(Token::RBrack)?;
        Ok(Some(ExprRange { msb, lsb }))
    }

    /// Skip packed dimensions tambahan `[msb:lsb][msb:lsb]...` setelah range pertama.
    /// Dipakai di typedef agar `typedef logic [W-1:0][N-1:0] name;` tidak gagal parse.
    pub(crate) fn skip_extra_packed_dims(&mut self) -> Result<(), SimError> {
        while self.peek() == &Token::LBrack {
            self.parse_range()?;
        }
        Ok(())
    }

    /// True jika token saat ini adalah packed dimension `[msb:lsb]` (bukan
    /// unpacked `[N]`, dynamic `[$]`, atau associative `[int]`).
    pub(crate) fn peek_is_packed_dim(&self) -> bool {
        if self.peek() != &Token::LBrack {
            return false;
        }
        let mut depth = 0usize;
        let mut i = 0usize;
        loop {
            match self.peek_ahead(i) {
                Token::LBrack => depth += 1,
                Token::RBrack => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return false;
                    }
                }
                Token::Colon if depth == 1 => return true,
                Token::Eof => return false,
                _ => {}
            }
            i += 1;
        }
    }

}
