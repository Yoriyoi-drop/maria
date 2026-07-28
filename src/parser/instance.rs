// Parser submodule: module/interface/instance parsing
// Tanggung jawab: parse_module, parse_interface, parse_interface_fast, parse_program_fast,
// skip_balanced_paren, parse_modport, parse_port_list, parse_instance, parse_gate_primitive

use super::Parser;
use crate::ast::*;
use crate::ast::types::const_eval_simple;
use crate::error::SimError;
use crate::intern::Symbol;
use crate::parser::lexer::*;
use crate::parser::util::*;

impl Parser {
    pub(crate) fn parse_module(&mut self) -> Result<Module, SimError> {
        self.debug_parse_trace("parse_module");
        self.advance(); // consume 'module', 'interface', or 'program'
        self.typedef_names.clear();

        // Skip (* ... *) attributes before module name
        while self.peek() == &Token::LParen && self.peek_ahead(1) == &Token::Star {
            self.skip_attribute();
        }

        let name_tok = self.peek().clone();
        let name = match &name_tok {
            Token::Ident(s) => {
                self.advance();
                s.clone()
            }
            _ => {
                return Err(self.err("expected module name"))
            }
        };

        let mut ports = Vec::new();
        let mut params = Vec::new();
        let mut decls = Vec::new();
        let mut items = Vec::new();

        // Handle import statements between module name and #( / (
        while self.peek() == &Token::Import {
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
            items.push(ModuleItem::Import {
                package: pkg,
                item: item.clone(),
            });
        }

        if self.peek() == &Token::Hash {
            self.advance();
            self.expect(Token::LParen)?;
            self.parse_param_list(&mut params)?;
            self.expect(Token::RParen)?;
        }

        if self.peek() == &Token::LParen {
            self.advance();
            if self.peek() != &Token::RParen {
                self.parse_port_list(&mut ports)?;
            }
            self.expect(Token::RParen)?;
        }
        self.skip_semi();

        let mut _heartbeat = 0;
        loop {
            _heartbeat += 1; if _heartbeat % 5000 == 0 { eprintln!("DEBUG: parse_module '{}' iter={} pos={}/{}", name, _heartbeat, self.pos, self.tokens.len()); }
            match self.peek() {
                Token::Endmodule | Token::EndInterface | Token::EndProgram | Token::Eof => break,
                _ => {
                    let before = self.pos;
                    let result = self.parse_module_item();
                    match result {
                        Ok(Some(item)) => {
                            if let ModuleItem::Covergroup(ref cg) = item {
                                self.class_names.push(cg.name.clone());
                            }
                            match item {
                                ModuleItem::Decl(d) => decls.push(d),
                                ModuleItem::Param(p) => params.push(p),
                                other => items.push(other),
                            }
                        }
                        Ok(None) => {
                            // If position didn't advance, skip the token to avoid infinite loop
                            if self.pos == before {
                                self.advance();
                            }
                        }
                        Err(e) => {
                            let diag = e.to_diagnostic();
                            self.errors.push(diag);
                            self.skip_until_semi_or_end()?;
                        }
                    }
                }
            }
        }

        match self.peek() {
            Token::EndProgram => {
                self.advance();
            }
            Token::EndInterface => {
                self.advance();
            }
            _ => {
                self.expect(Token::Endmodule)?;
            }
        }
        if self.peek() == &Token::Colon {
            self.advance();
            if matches!(self.peek(), Token::Ident(_)) {
                self.advance();
            }
        }

        Ok(Module {
            name,
            ports,
            params,
            decls,
            items,
        })
    }

    pub(crate) fn parse_interface_fast(&mut self) -> Result<(), SimError> {
        self.debug_parse_trace("parse_interface_fast");
        self.advance(); // consume 'interface'
        match self.peek() {
            Token::Ident(_) => {
                self.advance();
            }
            _ => return Err(self.err("expected interface name")),
        }
        self.skip_semi();
        loop {
            match self.peek() {
                Token::EndInterface | Token::Eof => {
                    self.advance();
                    break;
                }
                _ => {
                    match self.peek() {
                        Token::ModPort => {
                            self.advance(); // consume 'modport'
                            loop {
                                match self.peek() {
                                    Token::Ident(_) => {
                                        self.advance();
                                    }
                                    _ => {}
                                }
                                self.skip_until_semi_or_end()?;
                                break;
                            }
                        }
                        Token::Param
                        | Token::Parameter
                        | Token::LocalParam
                        | Token::Function
                        | Token::Task => {
                            self.skip_until_semi_or_end()?;
                        }
                        _ => {
                            self.parse_decl()?;
                        }
                    }
                }
            }
        }
        if self.peek() == &Token::Colon {
            self.advance();
            if matches!(self.peek(), Token::Ident(_)) {
                self.advance();
            }
        }
        Ok(())
    }

    pub(crate) fn parse_program_fast(&mut self) -> Result<(), SimError> {
        self.debug_parse_trace("parse_program_fast");
        self.advance(); // consume 'program'
        if let Token::Ident(_) = self.peek() {
            self.advance();
        }
        if self.peek() == &Token::Hash {
            self.advance(); // #
            if self.peek() == &Token::LParen {
                self.skip_balanced_paren()?;
            }
        }
        if self.peek() == &Token::LParen {
            self.skip_balanced_paren()?;
        }
        self.skip_semi();
        loop {
            match self.peek() {
                Token::EndProgram | Token::Eof => {
                    self.advance();
                    break;
                }
                _ => {
                    self.advance();
                }
            }
        }
        Ok(())
    }

    pub(crate) fn skip_balanced_paren(&mut self) -> Result<(), SimError> {
        let mut depth = 0;
        loop {
            match self.peek() {
                Token::LParen => {
                    depth += 1;
                    self.advance();
                }
                Token::RParen => {
                    depth -= 1;
                    self.advance();
                    if depth == 0 {
                        break;
                    }
                }
                Token::Eof => return Err(self.err("unexpected EOF in balanced paren")),
                _ => {
                    self.advance();
                }
            }
        }
        Ok(())
    }

    pub(crate) fn parse_interface(&mut self) -> Result<Interface, SimError> {
        self.debug_parse_trace("parse_interface");
        self.advance(); // consume 'interface'
        self.typedef_names.clear();

        let name = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => {
                return Err(self.err("expected interface name"))
            }
        };
        self.skip_semi();

        let params = Vec::new();
        let mut decls = Vec::new();
        let mut modports = Vec::new();

        loop {
            match self.peek() {
                Token::EndInterface | Token::Eof => {
                    break;
                }
                _ => match self.peek() {
                    Token::ModPort => {
                        modports.push(self.parse_modport()?);
                    }
                    Token::Param | Token::Parameter | Token::LocalParam => {
                        self.skip_until_semi_or_end()?;
                    }
                    _ => {
                        let decl = self.parse_decl()?;
                        decls.push(decl);
                    }
                },
            }
        }
        match self.peek() {
            Token::EndInterface => {
                self.advance();
            }
            _ => {
                return Err(self.err("expected endinterface"));
            }
        }
        if self.peek() == &Token::Colon {
            self.advance();
            if matches!(self.peek(), Token::Ident(_)) {
                self.advance();
            }
        }

        Ok(Interface {
            name,
            params,
            decls,
            modports,
        })
    }

    pub(crate) fn parse_modport(&mut self) -> Result<Modport, SimError> {
        self.debug_parse_trace("parse_modport");
        self.advance(); // consume 'modport'
        let name = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => {
                return Err(self.err("expected modport name"))
            }
        };
        self.expect(Token::LParen)?;
        let mut items = Vec::new();
        loop {
            let dir = match self.peek() {
                Token::Input => {
                    self.advance();
                    PortDirection::Input
                }
                Token::Output => {
                    self.advance();
                    PortDirection::Output
                }
                Token::Inout => {
                    self.advance();
                    PortDirection::Inout
                }
                _ => {
                    return Err(self.err("expected direction in modport"))
                }
            };
            // Collect all signals under this direction, comma-separated
            loop {
                let sig_name = match self.peek() {
                    Token::Ident(s) => {
                        let n = s.clone();
                        self.advance();
                        n
                    }
                    _ => {
                        return Err(self.err("expected signal name in modport"))
                    }
                };
                items.push(ModportItem {
                    name: sig_name,
                    direction: dir.clone(),
                });
                match self.peek() {
                    Token::Comma => {
                        self.advance();
                        // Check if next token is a direction (then break inner loop)
                        match self.peek() {
                            Token::Input | Token::Output | Token::Inout => break,
                            _ => continue,
                        }
                    }
                    _ => break,
                }
            }
            match self.peek() {
                Token::RParen => {
                    self.advance();
                    break;
                }
                _ => continue,
            }
        }
        self.skip_semi();
        Ok(Modport { name, items })
    }

    pub(crate) fn parse_port_list(&mut self, ports: &mut Vec<Port>) -> Result<(), SimError> {
        self.debug_parse_trace("parse_port_list");
        loop {
            if self.peek() == &Token::RParen || self.peek() == &Token::Eof {
                break;
            }

            let tok = self.peek().clone();
            match tok {
                Token::Dot => {
                    self.advance();
                    match self.peek() {
                        Token::Ident(_) => {
                            self.advance();
                        }
                        _ => {
                            return Err(self.err("expected port name"))
                        }
                    }
                    self.expect(Token::LParen)?;
                    if self.peek() != &Token::RParen {
                        self.parse_expr(0)?;
                    }
                    self.expect(Token::RParen)?;
                }
                Token::Comma => {
                    self.advance(); // skip stray comma
                }
                _ => {
                    let dir = match self.peek() {
                        Token::Input => {
                            self.advance();
                            PortDirection::Input
                        }
                        Token::Output => {
                            self.advance();
                            PortDirection::Output
                        }
                        Token::Inout => {
                            self.advance();
                            PortDirection::Inout
                        }
                        _ => PortDirection::Input,
                    };

                    if matches!(
                        self.peek(),
                        Token::Wire
                            | Token::Reg
                            | Token::Logic
                            | Token::Bit
                            | Token::Byte
                            | Token::Shortint
                            | Token::Longint
                            | Token::Time
                            | Token::Int
                            | Token::Integer
                    ) {
                        self.advance();
                    }

                    // Check for type parameter reference (identifier before port name or range)
                    let mut dtype_name = None;
                    if let Token::Ident(_s) = self.peek() {
                        let ah1 = self.peek_ahead(1).clone();
                        if ah1 == Token::Scope {
                            let pkg = self.expect_ident()?;
                            self.expect(Token::Scope)?;
                            let typ = self.expect_ident()?;
                            dtype_name = Some(format!("{}::{}", pkg, typ));
                        } else if matches!(ah1, Token::Ident(_) | Token::LBrack) {
                            let name = self.expect_ident()?;
                            dtype_name = Some(name.as_str().to_string());
                        }
                    }

                    if self.peek() == &Token::Signed {
                        self.advance();
                    }

                    let expr_range = if self.peek() == &Token::LBrack {
                        self.parse_range()?
                    } else {
                        None
                    };
                    // Parse additional packed dimensions before port name: [a:b][c:d]
                    while self.peek() == &Token::LBrack {
                        self.parse_range()?;
                    }
                    let range = expr_range.as_ref().and_then(|er| {
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

                    loop {
                        let name_tok = self.peek().clone();
                        match &name_tok {
                            Token::Ident(name) => {
                                self.advance();
                                // Skip unpacked array dimensions after port name: data_i [N]
                                while self.peek() == &Token::LBrack
                                    && self.peek_ahead(1) != &Token::Colon
                                {
                                    self.advance(); // [
                                    self.parse_expr(0)?;
                                    self.expect(Token::RBrack)?;
                                }
                                ports.push(Port {
                                    name: *name,
                                    direction: dir.clone(),
                                    range: range.clone(),
                                    expr_range: expr_range.clone(),
                                    dtype_name: dtype_name.as_ref().map(|s| Symbol::intern(s)),
                                });
                            }
                            _ => break,
                        }

                        if self.peek() == &Token::Comma {
                            let ahead = self.peek_ahead(1).clone();
                            let is_new_port = ahead == Token::Input
                                || ahead == Token::Output
                                || ahead == Token::Inout
                                || (matches!(&ahead, Token::Ident(_))
                                    && matches!(self.peek_ahead(2), Token::Scope));
                            if !is_new_port {
                                self.advance();
                            } else {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                }
            }

            if self.peek() == &Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        Ok(())
    }

    pub(crate) fn parse_instance(&mut self) -> Result<ModuleInstance, SimError> {
        self.debug_parse_trace("parse_instance");
        let name_tok = self.peek().clone();
        let module_name = match &name_tok {
            Token::Ident(s) => {
                self.advance();
                                *s
            }
            _ => {
                return Err(self.err("expected module name"))
            }
        };

        let mut param_assigns: std::collections::HashMap<Symbol, crate::ast::Expr> = std::collections::HashMap::new();
        let mut type_param_assigns: std::collections::HashMap<Symbol, crate::ast::DataType> = std::collections::HashMap::new();

        if self.peek() == &Token::Hash {
            self.advance();
            self.expect(Token::LParen)?;
            if self.peek() != &Token::RParen {
                loop {
                    if self.peek() == &Token::Dot {
                        self.advance();
                        let pname_tok = self.peek().clone();
                        let pname: Symbol = match &pname_tok {
                            Token::Ident(s) => {
                                self.advance();
                                *s
                            }
                            _ => {
                                return Err(self.err("expected parameter name"))
                            }
                        };
                        self.expect(Token::LParen)?;
                        if self.is_type_token() {
                            let dt = self.parse_type_expr()?;
                            self.expect(Token::RParen)?;
                            type_param_assigns.insert(pname, dt);
                        } else {
                            let val = self.parse_expr(0)?;
                            self.expect(Token::RParen)?;
                            param_assigns.insert(pname, val);
                        }
                    } else {
                        let val = self.parse_expr(0)?;
                        param_assigns.insert(Symbol::intern(&format!("__param{}", param_assigns.len())), val);
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

        let inst_tok = self.peek().clone();
        let instance_name = match &inst_tok {
            Token::Ident(s) => {
                self.advance();
                *s
            }
            _ => {
                return Err(self.err("expected instance name"))
            }
        };

        // Parse optional array range [msb:lsb] for arrayed instances
        let range = if self.peek() == &Token::LBrack {
            self.parse_range()?
        } else {
            None
        };

        let mut port_conns = Vec::new();
        if self.peek() == &Token::LParen {
            self.advance();
            if self.peek() != &Token::RParen {
                loop {
                    if self.peek() == &Token::Dot {
                        self.advance();

                        if self.peek() == &Token::Star {
                            self.advance();
                            continue;
                        }

                        let port_tok = self.peek().clone();
                        let port_name = match &port_tok {
                            Token::Ident(s) => {
                                self.advance();
                                *s
                            }
                            _ => {
                            return Err(self.err("expected port name"))
                            }
                        };

                        if self.peek() == &Token::LParen {
                            self.advance();
                            let expr = if self.peek() != &Token::RParen {
                                self.parse_expr(0)?
                            } else {
                                Expr::Value(Value::Decimal(0))
                            };
                            self.expect(Token::RParen)?;
                            port_conns.push(PortConnection::Named {
                                port: port_name,
                                expr,
                            });
                        } else {
                            port_conns.push(PortConnection::Named {
                                port: port_name,
                                expr: Expr::Ident { name: port_name, line: 0, col: 0 },
                            });
                        }
                    } else {
                        let expr = self.parse_expr(0)?;
                        port_conns.push(PortConnection::Positional(expr));
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

        if self.peek() != &Token::Semi {
            // If we have trailing tokens (e.g., = new() from misidentified class type), skip them
            self.skip_until_semi_or_end()?;
        } else {
            self.advance();
        }

        Ok(ModuleInstance {
            module_name,
            instance_name,
            range,
            param_assigns,
            type_param_assigns,
            port_conns,
        })
    }

    pub(crate) fn parse_gate_primitive(&mut self) -> Result<GatePrimitive, SimError> {
        self.debug_parse_trace("parse_gate_primitive");
        let gate_type = match self.peek() {
            Token::And => {
                self.advance();
                GateType::And
            }
            Token::Or => {
                self.advance();
                GateType::Or
            }
            Token::Nand => {
                self.advance();
                GateType::Nand
            }
            Token::Nor => {
                self.advance();
                GateType::Nor
            }
            Token::Xor => {
                self.advance();
                GateType::Xor
            }
            Token::Xnor => {
                self.advance();
                GateType::Xnor
            }
            Token::Buf => {
                self.advance();
                GateType::Buf
            }
            Token::NotGate => {
                self.advance();
                GateType::Not
            }
            _ => {
                return Err(self.err("expected gate type"))
            }
        };

        // Parse optional drive strength: (strength1, strength0)
        let mut drive_strength = None;
        if self.peek() == &Token::LParen && matches!(self.peek_ahead(1), Token::Ident(_)) {
            // Check if this looks like drive strength, not port list
            let saved = self.pos;
            self.advance(); // consume (
            if let Token::Ident(s1) = self.peek().clone() {
                if is_strength_keyword(s1.as_str()) {
                    self.advance();
                    if self.peek() == &Token::Comma {
                        self.advance();
                        if let Token::Ident(s2) = self.peek().clone() {
                            if is_strength_keyword(s2.as_str()) {
                                self.advance();
                                if self.peek() == &Token::RParen {
                                    self.advance();
                                    drive_strength = Some((s1.as_str().to_lowercase(), s2.as_str().to_lowercase()));
                                }
                            }
                        }
                    }
                }
            }
            if drive_strength.is_none() {
                self.pos = saved; // Not drive strength, restore position
            }
        }

        // Parse optional delay: #(rise, fall, turnoff) or #delay
        let mut delay = None;
        if self.peek() == &Token::Hash {
            self.advance();
            if self.peek() == &Token::LParen {
                self.advance();
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
                delay = Some(crate::ast::types::Delay {
                    rise,
                    fall,
                    turnoff,
                });
            } else {
                // Single delay value
                let d = Some(self.parse_expr(0)?);
                delay = Some(crate::ast::types::Delay {
                    rise: d.clone(),
                    fall: d,
                    turnoff: None,
                });
            }
        }

        let instance_name = if self.peek() == &Token::LParen {
            None
        } else {
            let name = match self.peek().clone() {
                Token::Ident(s) => {
                    self.advance();
                    Some(s)
                }
                _ => {
                    return Err(self.err("expected gate instance name"))
                }
            };
            name
        };
        self.expect(Token::LParen)?;
        let mut ports = Vec::new();
        if self.peek() != &Token::RParen {
            loop {
                let expr = self.parse_expr(0)?;
                ports.push(expr);
                if self.peek() == &Token::Comma {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        self.expect(Token::RParen)?;
        self.skip_semi();
        Ok(GatePrimitive {
            gate_type,
            instance_name,
            ports,
            drive_strength,
            delay,
        })
    }

}
