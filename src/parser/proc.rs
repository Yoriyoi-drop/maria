// Parser submodule: procedural block / function / task / generate parsing
// Tanggung jawab: parse_always, parse_initial, parse_final, parse_sensitivity_events,
// parse_sensitivity_list, parse_assign, parse_delay,
// parse_function, parse_task, parse_generate_block/Item/BlockBody

use super::Parser;
use crate::ast::*;
use crate::ast::types::const_eval_simple;
use crate::error::SimError;
use crate::intern::Symbol;
use crate::parser::lexer::*;

impl Parser {
    pub(crate) fn parse_always(&mut self) -> Result<AlwaysBlock, SimError> {
        let kind = match self.peek() {
            Token::Always => {
                self.advance();
                AlwaysKind::Always
            }
            Token::AlwaysComb => {
                self.advance();
                AlwaysKind::AlwaysComb
            }
            Token::AlwaysFF => {
                self.advance();
                AlwaysKind::AlwaysFF
            }
            Token::AlwaysLatch => {
                self.advance();
                AlwaysKind::AlwaysLatch
            }
            _ => unreachable!(),
        };

        let sensitivity = if self.peek() == &Token::At {
            self.advance();
            Some(self.parse_sensitivity_list()?)
        } else {
            None
        };

        let stmts = self.parse_stmt_block()?;

        Ok(AlwaysBlock {
            kind,
            sensitivity,
            stmts,
        })
    }

    pub(crate) fn parse_initial(&mut self) -> Result<InitialBlock, SimError> {
        self.advance();
        let stmts = self.parse_stmt_block()?;
        Ok(InitialBlock { stmts })
    }

    pub(crate) fn parse_final(&mut self) -> Result<InitialBlock, SimError> {
        self.advance();
        let stmts = self.parse_stmt_block()?;
        Ok(InitialBlock { stmts })
    }

    pub(crate) fn parse_sensitivity_events(&mut self) -> Result<Vec<SensitivityEvent>, SimError> {
        let mut events = Vec::new();
        loop {
            if self.peek() == &Token::RParen {
                break;
            }
            if self.peek() == &Token::Star {
                self.advance();
                events.push(SensitivityEvent::Wildcard);
            } else if self.peek() == &Token::PosEdge {
                self.advance();
                let expr = self.parse_primary_expr()?;
                events.push(SensitivityEvent::PosEdge(expr));
            } else if self.peek() == &Token::NegEdge {
                self.advance();
                let expr = self.parse_primary_expr()?;
                events.push(SensitivityEvent::NegEdge(expr));
            } else {
                let expr = self.parse_primary_expr()?;
                events.push(SensitivityEvent::Level(expr));
            }
            if self.peek() == &Token::Or || self.peek() == &Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        Ok(events)
    }

    pub(crate) fn parse_sensitivity_list(&mut self) -> Result<SensitivityList, SimError> {
        // Handle @* or @(*)
        if self.peek() == &Token::Star {
            self.advance();
            return Ok(SensitivityList {
                events: vec![SensitivityEvent::Wildcard],
            });
        }
        if self.peek() == &Token::LParen && self.peek_ahead(1) == &Token::Star {
            self.advance(); // (
            self.advance(); // *
                            // Check for @( * ) — closing paren may or may not be present
            if self.peek() == &Token::RParen {
                self.advance();
            }
            return Ok(SensitivityList {
                events: vec![SensitivityEvent::Wildcard],
            });
        }
        self.expect(Token::LParen)?;
        let events = self.parse_sensitivity_events()?;
        self.expect(Token::RParen)?;
        Ok(SensitivityList { events })
    }

    pub(crate) fn parse_assign(&mut self) -> Result<ContinuousAssign, SimError> {
        self.advance();

        let delay = if self.peek() == &Token::Hash {
            Some(self.parse_delay()?)
        } else {
            None
        };

        let lhs = self.parse_expr(0)?;
        self.expect(Token::BlockingAssign)?;
        let rhs = self.parse_expr(0)?;
        self.skip_semi();

        Ok(ContinuousAssign { lhs, rhs, delay })
    }

    pub(crate) fn parse_delay(&mut self) -> Result<Delay, SimError> {
        self.advance();
        self.expect(Token::LParen)?;
        let rise = Some(self.parse_expr(0)?);
        let fall = if self.peek() == &Token::Comma {
            self.advance();
            Some(self.parse_expr(0)?)
        } else {
            None
        };
        let turnoff = if self.peek() == &Token::Comma {
            self.advance();
            Some(self.parse_expr(0)?)
        } else {
            None
        };
        self.expect(Token::RParen)?;
        Ok(Delay {
            rise,
            fall,
            turnoff,
        })
    }

    pub(crate) fn parse_function(&mut self, virtual_flag: bool) -> Result<FunctionDecl, SimError> {
        self.advance(); // consume 'function'
                        // Capture optional 'static' qualifier
        let is_static = if matches!(self.peek(), Token::Static) {
            self.advance();
            true
        } else {
            if matches!(self.peek(), Token::Auto) {
                self.advance();
            }
            false
        };
        // Parse optional return type
        let return_type = match self.peek() {
            Token::Void => {
                self.advance();
                Some(Box::new(DataType::Void))
            }
            Token::Int => {
                self.advance();
                Some(Box::new(DataType::Int))
            }
            Token::Integer => {
                self.advance();
                Some(Box::new(DataType::Integer))
            }
            Token::String => {
                self.advance();
                Some(Box::new(DataType::String))
            }
            Token::Byte => {
                self.advance();
                Some(Box::new(DataType::Byte))
            }
            Token::Shortint => {
                self.advance();
                Some(Box::new(DataType::Shortint))
            }
            Token::Longint => {
                self.advance();
                Some(Box::new(DataType::Longint))
            }
            Token::Time => {
                self.advance();
                Some(Box::new(DataType::Time))
            }
            Token::Bit => {
                self.advance();
                Some(Box::new(DataType::Bit))
            }
            Token::Logic => {
                self.advance();
                Some(Box::new(DataType::Logic))
            }
            Token::Signed => {
                self.advance();
                Some(Box::new(DataType::Signed(Box::new(DataType::Logic))))
            }
    Token::Ident(name) if self.type_param_names.contains(&name) => {
        let tp_name = name.clone();
        self.advance();
        Some(Box::new(DataType::UserDefined(tp_name)))
    }
            Token::Ident(_) if matches!(self.peek_ahead(1), Token::Ident(_) | Token::LBrack | Token::Scope) => {
                let first = self.expect_ident()?;
                let tp_name = if self.peek() == &Token::Scope {
                    self.advance();
                    let second = self.expect_ident()?;
                    Symbol::intern(&format!("{}::{}", first, second))
                } else {
                    first
                };
                Some(Box::new(DataType::UserDefined(tp_name)))
            }
            _ => None,
        };
        if self.peek() == &Token::Unsigned {
            self.advance();
        }
        let range = if self.peek() == &Token::LBrack {
            self.parse_range()?
        } else {
            None
        };
        self.skip_extra_packed_dims()?;
        let name_tok = self.peek().clone();
        let name = match &name_tok {
            Token::Ident(n) => {
                self.advance();
                *n
            }
            Token::New => {
                self.advance();
                Symbol::intern("new")
            }
            _ => {
                return Err(self.err("expected function name"))
            }
        };
        // Handle out-of-body method: class_name :: method_name
        let name = if self.peek() == &Token::Scope {
            self.advance(); // consume ::
            let tok = self.peek().clone();
            match &tok {
                Token::Ident(m) => {
                    self.advance();
                    *m
                }
                Token::New => {
                    self.advance();
                    Symbol::intern("new")
                }
                _ => {
                    return Err(self.err("expected method name after ::"));
                }
            }
        } else {
            name
        };
        // Parse ANSI-style port list in parens (e.g., function new(int level, string name))
        let mut ports = Vec::new();
        let mut decls = Vec::new();
        let mut last_direction: Option<PortDirection> = None;
        if self.peek() == &Token::LParen {
            self.advance();
            while self.peek() != &Token::RParen && self.peek() != &Token::Eof {
                // Track whether we saw int/integer for 32-bit default width
                let is_int = matches!(self.peek(), Token::Int | Token::Integer);
                // Skip type keywords and direction keywords
                if matches!(
                    self.peek(),
                    Token::Int
                        | Token::Integer
                        | Token::String
                        | Token::Void
                        | Token::Reg
                        | Token::Logic
                        | Token::Wire
                        | Token::Signed
                        | Token::Unsigned
                        | Token::Input
                        | Token::Output
                        | Token::Inout
                        | Token::Ref
                ) {
                    if matches!(
                        self.peek(),
                        Token::Input | Token::Output | Token::Inout | Token::Ref
                    ) {
                        last_direction = Some(match self.peek() {
                            Token::Input => PortDirection::Input,
                            Token::Output => PortDirection::Output,
                            Token::Ref => PortDirection::Ref,
                            _ => PortDirection::Inout,
                        });
                    }
                    self.advance();
                } else if let Token::Ident(name) = self.peek() {
                    if self.type_param_names.contains(name) {
                        self.advance();
                    } else if matches!(self.peek_ahead(1), Token::Ident(_) | Token::LBrack) {
                        // User-defined type name followed by port name or range
                        self.advance();
                    }
                } else {
                    // Unknown token in port list — advance to avoid infinite loop
                    self.advance();
                    continue;
                }
                // Skip range like [7:0]
                let range = if self.peek() == &Token::LBrack {
                    let _ = self.parse_range();
                    None
                } else if is_int {
                    Some(Range { msb: 31, lsb: 0 })
                } else {
                    None
                };
                self.skip_extra_packed_dims()?;
                // Parse port name(s)
                loop {
                    match self.peek() {
                        Token::Ident(pname) => {
                            let pn = pname.clone();
                            self.advance();
                            ports.push(FunctionPort {
                                name: pn,
                                range: range.clone(),
                                expr_range: None,
                                direction: last_direction,
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
            }
            self.expect(Token::RParen)?;
        }
        if self.peek() == &Token::Semi {
            self.advance();
        }
        // Parse ports and declarations until 'begin' or statement
        loop {
            match self.peek() {
                Token::Input | Token::Output | Token::Inout | Token::Ref => {
                    let direction = match self.peek() {
                        Token::Input => {
                            self.advance();
                            PortDirection::Input
                        }
                        Token::Output => {
                            self.advance();
                            PortDirection::Output
                        }
                        Token::Ref => {
                            self.advance();
                            PortDirection::Ref
                        }
                        _ => {
                            self.advance();
                            PortDirection::Inout
                        }
                    };
                    let port_range = if self.peek() == &Token::LBrack {
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
                    loop {
                        match self.peek() {
                        Token::Ident(pname) => {
                            let pn = pname.clone();
                            self.advance();
                            ports.push(FunctionPort {
                                name: pn,
                                range: port_range.clone(),
                                expr_range: None,
                                direction: Some(direction),
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
                    self.skip_semi();
                }
                Token::Wire | Token::Reg | Token::Logic | Token::Int | Token::Integer => {
                    let decl = self.parse_decl()?;
                    decls.push(decl);
                }
                Token::Bit | Token::Byte | Token::Shortint | Token::Longint | Token::Time => {
                    let decl = self.parse_decl()?;
                    decls.push(decl);
                }
                Token::Ident(_) => {
                    // User-defined type declaration: ident followed by ident or ::
                    match self.peek_ahead(1) {
                        Token::Ident(_) | Token::Scope => {
                            let decl = self.parse_decl()?;
                            decls.push(decl);
                        }
                        _ => break,
                    }
                }
                Token::Auto | Token::Static => {
                    // automatic/static variable declaration in function body
                    self.advance();
                    // Try to parse as declaration
                    if let Ok(decl) = self.parse_decl() {
                        decls.push(decl);
                    } else {
                        return Err(self.err("expected declaration after automatic/static"));
                    }
                }
                Token::Begin => {
                    let stmts = self.parse_stmt_block()?;
                    self.expect(Token::EndFunction)?;
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) {
                            self.advance();
                        }
                    }
                    return Ok(FunctionDecl {
                        name,
                        range,
                        return_type,
                        ports,
                        decls,
                        stmts,
                        virtual_flag,
                        is_static,
                    });
                }
                Token::EndFunction => {
                    self.advance(); // consume 'endfunction'
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) {
                            self.advance();
                        }
                    }
                    return Ok(FunctionDecl {
                        name,
                        range,
                        return_type,
                        ports,
                        decls,
                        stmts: vec![],
                        virtual_flag,
                        is_static,
                    });
                }
                _ => break,
            }
        }
        // No begin/end block - parse statements until endfunction
        let mut stmts = Vec::new();
        loop {
            if matches!(self.peek(), Token::EndFunction | Token::End | Token::EndClass | Token::EndInterface | Token::EndPackage | Token::Eof) {
                break;
            }
            stmts.push(self.parse_stmt()?);
        }
        match self.peek() {
            Token::EndFunction => {
                self.advance();
            }
            _ => {
                return Err(self.err("expected endfunction"));
            }
        }
        if self.peek() == &Token::Colon {
            self.advance();
            if matches!(self.peek(), Token::Ident(_)) {
                self.advance();
            }
        }
        Ok(FunctionDecl {
            name,
            range,
            return_type,
            ports,
            decls,
            stmts,
            virtual_flag,
            is_static,
        })
    }

    pub(crate) fn parse_task(&mut self, virtual_flag: bool) -> Result<TaskDecl, SimError> {
        self.advance(); // consume 'task'
                        // Capture optional 'static' qualifier
        let is_static = if matches!(self.peek(), Token::Static) {
            self.advance();
            true
        } else {
            if matches!(self.peek(), Token::Auto) {
                self.advance();
            }
            false
        };
        let name = self.expect_ident()?;
        // Handle out-of-body method: class_name :: method_name
        let name = if self.peek() == &Token::Scope {
            self.advance(); // consume ::
            let tok = self.peek().clone();
            match &tok {
                Token::Ident(m) => {
                    self.advance();
                    *m
                }
                _ => {
                    return Err(self.err("expected method name after ::"));
                }
            }
        } else {
            name
        };
        let mut ports = Vec::new();
        let mut decls = Vec::new();
        let mut last_direction: Option<PortDirection> = None;
        // Parse ANSI-style port list in parens (e.g., task set_val(input [7:0] x))
        if self.peek() == &Token::LParen {
            self.advance();
            while self.peek() != &Token::RParen && self.peek() != &Token::Eof {
                let is_int = matches!(self.peek(), Token::Int | Token::Integer);
                if matches!(
                    self.peek(),
                    Token::Int
                        | Token::Integer
                        | Token::String
                        | Token::Void
                        | Token::Reg
                        | Token::Logic
                        | Token::Wire
                        | Token::Signed
                        | Token::Input
                        | Token::Output
                        | Token::Inout
                        | Token::Ref
                ) {
                    if matches!(
                        self.peek(),
                        Token::Input | Token::Output | Token::Inout | Token::Ref
                    ) {
                        last_direction = Some(match self.peek() {
                            Token::Input => PortDirection::Input,
                            Token::Output => PortDirection::Output,
                            Token::Ref => PortDirection::Ref,
                            _ => PortDirection::Inout,
                        });
                    }
                    self.advance();
                } else if !matches!(self.peek(), Token::LBrack | Token::Ident(_) | Token::Comma) {
                    self.advance();
                    continue;
                }
                let range: Option<Range> = if self.peek() == &Token::LBrack {
                    if let Ok(Some(er)) = self.parse_range() {
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
                    } else {
                        None
                    }
                } else if is_int {
                    Some(Range { msb: 31, lsb: 0 })
                } else {
                    None
                };
                loop {
                    match self.peek() {
                        Token::Ident(pname) => {
                            let pn = pname.clone();
                            self.advance();
                            ports.push(FunctionPort {
                                name: pn,
                                range: range.clone(),
                                expr_range: None,
                                direction: last_direction,
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
            }
            self.expect(Token::RParen)?;
        }
        if self.peek() == &Token::Semi {
            self.advance();
        }

        // Parse non-ANSI port declarations and body
        loop {
            match self.peek() {
                Token::Input | Token::Output | Token::Inout | Token::Ref => {
                    let direction = match self.peek() {
                        Token::Input => {
                            self.advance();
                            PortDirection::Input
                        }
                        Token::Output => {
                            self.advance();
                            PortDirection::Output
                        }
                        Token::Ref => {
                            self.advance();
                            PortDirection::Ref
                        }
                        _ => {
                            self.advance();
                            PortDirection::Inout
                        }
                    };
                    let port_range = if self.peek() == &Token::LBrack {
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
                    loop {
                        match self.peek() {
                        Token::Ident(pname) => {
                            let pn = pname.clone();
                            self.advance();
                            ports.push(FunctionPort {
                                name: pn,
                                range: port_range.clone(),
                                expr_range: None,
                                direction: Some(direction),
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
                    self.skip_semi();
                }
                Token::Wire | Token::Reg | Token::Logic | Token::Int | Token::Integer => {
                    decls.push(self.parse_decl()?);
                }
                Token::Begin => {
                    let stmts = self.parse_stmt_block()?;
                    self.expect(Token::EndTask)?;
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) {
                            self.advance();
                        }
                    }
                    return Ok(TaskDecl {
                        name,
                        ports,
                        decls,
                        stmts,
                        virtual_flag,
                        is_static,
                    });
                }
                Token::EndTask => {
                    self.advance(); // consume 'endtask'
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) {
                            self.advance();
                        }
                    }
                    return Ok(TaskDecl {
                        name,
                        ports,
                        decls,
                        stmts: vec![],
                        virtual_flag,
                        is_static,
                    });
                }
                _ => break,
            }
        }
        let mut stmts = Vec::new();
        loop {
            if matches!(self.peek(), Token::EndTask | Token::End | Token::EndClass | Token::EndInterface | Token::EndPackage | Token::Eof) {
                break;
            }
            stmts.push(self.parse_stmt()?);
        }
        match self.peek() {
            Token::EndTask => {
                self.advance();
            }
            _ => {
                return Err(self.err("expected endtask"));
            }
        }
        if self.peek() == &Token::Colon {
            self.advance();
            if matches!(self.peek(), Token::Ident(_)) {
                self.advance();
            }
        }
        Ok(TaskDecl {
            name,
            ports,
            decls,
            stmts,
            virtual_flag,
            is_static,
        })
    }

    pub(crate) fn parse_generate_block(&mut self) -> Result<GenerateBlock, SimError> {
        self.advance(); // consume 'generate'
        let mut items = Vec::new();
        loop {
            match self.peek() {
                Token::EndGenerate => {
                    self.advance();
                    return Ok(GenerateBlock { items });
                }
                Token::Eof => {
                    return Err(self.err("unexpected EOF in generate block"));
                }
                _ => {
                    let item = self.parse_generate_item()?;
                    items.push(item);
                }
            }
        }
    }

    pub(crate) fn parse_generate_item(&mut self) -> Result<GenerateItem, SimError> {
        match self.peek() {
            Token::If => {
                self.advance();
                self.expect(Token::LParen)?;
                let cond = self.parse_expr(0)?;
                self.expect(Token::RParen)?;
                let true_items = self.parse_generate_block_body()?;
                let false_items = if self.peek() == &Token::Else {
                    self.advance();
                    self.parse_generate_block_body()?
                } else {
                    Vec::new()
                };
                Ok(GenerateItem::If {
                    cond,
                    true_items,
                    false_items,
                })
            }
            Token::For => {
                self.advance();
                self.expect(Token::LParen)?;
                // Skip optional 'genvar' keyword
                if self.peek() == &Token::GenVar {
                    self.advance();
                }
                let var_tok = self.peek().clone();
                let var = match &var_tok {
                    Token::Ident(n) => {
                        self.advance();
                        *n
                    }
                    _ => {
                        return Err(self.err("expected genvar name"))
                    }
                };
                // Parse init: i = <expr>
                let _init = if self.peek() != &Token::Semi {
                    self.expect(Token::BlockingAssign)?;
                    let init_expr = self.parse_expr(0)?;
                    self.expect(Token::Semi)?;
                    Some(Stmt::BlockingAssign {
                        lhs: Expr::Ident { name: var, line: 0, col: 0 },
                        rhs: init_expr,
                        delay: None,
                    })
                } else {
                    self.advance();
                    None
                };
                // Parse condition
                let cond = if self.peek() != &Token::Semi {
                    let c = Some(self.parse_expr(0)?);
                    self.expect(Token::Semi)?;
                    c
                } else {
                    self.advance();
                    None
                };
                // Parse step
                let step = if self.peek() != &Token::RParen {
                    Some(self.parse_stmt()?)
                } else {
                    None
                };
                self.expect(Token::RParen)?;
                let body_items = self.parse_generate_block_body()?;
                Ok(GenerateItem::For {
                    var,
                    init: _init,
                    cond,
                    step,
                    body_items,
                })
            }
            Token::GenVar => {
                self.skip_until_semi_or_end()?;
                // genvar declaration - skip, handled by For loop above
                self.parse_generate_item()
            }
            Token::Case | Token::CaseX | Token::CaseZ => {
                let case_type = match self.peek() {
                    Token::Case => GenerateCaseType::Normal,
                    Token::CaseX => GenerateCaseType::CaseX,
                    Token::CaseZ => GenerateCaseType::CaseZ,
                    _ => unreachable!(),
                };
                self.advance();
                self.expect(Token::LParen)?;
                let expr = self.parse_expr(0)?;
                self.expect(Token::RParen)?;

                let mut items = Vec::new();
                let mut default = None;

                loop {
                    if self.peek() == &Token::Endcase || self.peek() == &Token::Eof {
                        break;
                    }

                    if self.peek() == &Token::Default {
                        self.advance();
                        self.expect(Token::Colon)?;
                        default = Some(self.parse_generate_block_body()?);
                    } else {
                        let mut labels = Vec::new();
                        loop {
                            let label = self.parse_expr(0)?;
                            labels.push(label);
                            if self.peek() == &Token::Comma {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                        self.expect(Token::Colon)?;
                        let body = self.parse_generate_block_body()?;
                        items.push(CaseGenerateItem { labels, body });
                    }
                }

                self.expect(Token::Endcase)?;
                Ok(GenerateItem::Case {
                    case_type,
                    expr,
                    items,
                    default,
                })
            }
            _ => {
                let item = self.parse_module_item()?;
                match item {
                    Some(mi) => Ok(GenerateItem::Items(vec![mi])),
                    None => return Err(self.err("expected generate item")),
                }
            }
        }
    }

    pub(crate) fn parse_generate_block_body(&mut self) -> Result<Vec<ModuleItem>, SimError> {
        if self.peek() == &Token::Begin {
            self.advance();
            // Skip optional begin label
            if matches!(self.peek(), Token::Ident(_)) {
                self.advance();
            }
            if self.peek() == &Token::Colon {
                self.advance();
                if matches!(self.peek(), Token::Ident(_)) {
                    self.advance();
                }
            }
            let mut items = Vec::new();
            loop {
                if matches!(self.peek(), Token::End | Token::Eof) {
                    self.advance();
                    // Handle optional : name after end
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) {
                            self.advance();
                        }
                    }
                    break;
                }
                match self.parse_module_item()? {
                    Some(mi) => items.push(mi),
                    None => {
                        self.skip_until_semi_or_end()?;
                    }
                }
            }
            Ok(items)
        } else {
            match self.parse_module_item()? {
                Some(mi) => Ok(vec![mi]),
                None => Ok(Vec::new()),
            }
        }
    }

}
