use crate::ast::*;
use crate::ast::types::const_eval_simple;
use crate::parser::lexer::*;

pub struct Parser {
    tokens: Vec<(Token, usize, usize)>,
    pos: usize,
    class_names: Vec<String>,
    typedef_names: Vec<String>,
}

impl Parser {
    pub fn new(tokens: Vec<(Token, usize, usize)>) -> Self {
        Self { tokens, pos: 0, class_names: Vec::new(), typedef_names: Vec::new() }
    }

    fn peek(&self) -> &Token {
        if self.pos >= self.tokens.len() {
            return &Token::Eof;
        }
        &self.tokens[self.pos].0
    }

    fn peek_line(&self) -> usize {
        if self.pos >= self.tokens.len() {
            return 0;
        }
        self.tokens[self.pos].1
    }

    fn peek_ahead(&self, n: usize) -> &Token {
        if self.tokens.is_empty() {
            return &Token::Eof;
        }
        let idx = (self.pos + n).min(self.tokens.len() - 1);
        &self.tokens[idx].0
    }

    fn advance(&mut self) {
        self.pos += 1;
    }

    fn expect(&mut self, expected: Token) -> Result<(), String> {
        let line = self.peek_line();
        if self.peek() == &expected {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!("line {}: expected {}, found {}", line, expected, self.peek()))
        }
    }

    fn skip_semi(&mut self) {
        if self.peek() == &Token::Semi {
            self.advance();
        }
    }

    fn expect_ident(&mut self) -> Result<String, String> {
        let tok = self.peek().clone();
        match &tok {
            Token::Ident(s) => { self.advance(); Ok(s.clone()) }
            Token::New => { self.advance(); Ok("new".to_string()) }
            Token::This => { self.advance(); Ok("this".to_string()) }
            _ => Err(format!("line {}: expected identifier, found {}", self.peek_line(), self.peek())),
        }
    }

    fn skip_paren_block(&mut self) {
        if self.peek() == &Token::LParen {
            self.advance();
            let mut depth = 1;
            while depth > 0 {
                match self.peek() {
                    Token::LParen => depth += 1,
                    Token::RParen => depth -= 1,
                    Token::Eof => break,
                    _ => {}
                }
                self.advance();
            }
        }
    }

    fn skip_brack_block(&mut self) {
        if self.peek() == &Token::LBrack {
            self.advance();
            let mut depth = 1;
            while depth > 0 {
                match self.peek() {
                    Token::LBrack => depth += 1,
                    Token::RBrack => depth -= 1,
                    Token::Eof => break,
                    _ => {}
                }
                self.advance();
            }
        }
    }

    pub fn parse_design(&mut self) -> Result<Design, String> {
        self.class_names.clear();
        let mut modules = Vec::new();
        let mut classes = Vec::new();
        let mut packages = Vec::new();
        // First pass: collect all class names
        let saved_pos = self.pos;
        while self.peek() != &Token::Eof {
            if self.peek() == &Token::Class {
                let start = self.pos;
                self.advance(); // consume 'class'
                if let Token::Ident(name) = self.peek() {
                    self.class_names.push(name.clone());
                }
                self.pos = start;
                let c = self.parse_class()?;
                classes.push(c);
            } else if matches!(self.peek(), Token::Module | Token::Interface) {
                let m = self.parse_module()?;
                modules.push(m);
            } else if self.peek() == &Token::Package {
                // consume 'package' keyword token if we defined one
                // For now, we'll handle in second pass
                self.parse_package_decl()?;
            } else {
                let line = self.peek_line();
                return Err(format!("line {}: expected module or class, found {}", line, self.peek()));
            }
        }
        self.pos = saved_pos;
        modules.clear();
        classes.clear();
        // Second pass: full parse with class names known
        while self.peek() != &Token::Eof {
            match self.peek() {
                Token::Module | Token::Interface => {
                    let m = self.parse_module()?;
                    modules.push(m);
                }
                Token::Class => {
                    let c = self.parse_class()?;
                    classes.push(c);
                }
                Token::Package => {
                    let p = self.parse_package_decl()?;
                    packages.push(p);
                }
                _ => {
                    let line = self.peek_line();
                    return Err(format!("line {}: expected module, class, or package, found {}", line, self.peek()));
                }
            }
        }
        Ok(Design { modules, classes, packages, top_module: None })
    }

    fn parse_package_decl(&mut self) -> Result<PackageDecl, String> {
        self.advance(); // consume 'package'
        let name = self.expect_ident()?;
        self.skip_semi();
        let mut items = Vec::new();
        loop {
            match self.peek() {
                Token::EndPackage => { self.advance(); break; }
                Token::Eof => return Err("unexpected EOF in package".to_string()),
                _ => {
                    match self.peek() {
                        Token::Param | Token::Parameter | Token::LocalParam => {
                            let tok = self.peek().clone();
                            self.advance();
                            let name_tok = self.peek().clone();
                            let pname = match &name_tok {
                                Token::Ident(s) => { self.advance(); s.clone() }
                                _ => return Err("expected parameter name".to_string()),
                            };
                            let default = if self.peek() == &Token::BlockingAssign {
                                self.advance();
                                Some(self.parse_expr(0)?)
                            } else { None };
                            self.skip_semi();
                            items.push(PackageItem::Param(ParamDecl { name: pname, dtype: None, default }));
                        }
                        Token::Function => {
                            items.push(PackageItem::Function(self.parse_function(false)?));
                        }
                        Token::Task => {
                            items.push(PackageItem::Task(self.parse_task(false)?));
                        }
                        Token::Typedef => {
                            let td = self.parse_typedef()?;
                            self.typedef_names.push(td.name.clone());
                            items.push(PackageItem::Typedef(td));
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

    fn parse_class(&mut self) -> Result<ClassDecl, String> {
        self.advance(); // consume 'class'
        let name = self.expect_ident()?;
        let extends = if self.peek() == &Token::Extends {
            self.advance();
            Some(self.expect_ident()?)
        } else {
            None
        };
        self.expect(Token::Semi)?;
        let mut members = Vec::new();
        loop {
            match self.peek() {
                Token::EndClass => { self.advance(); break; }
                Token::Function => {
                    members.push(ClassMember::Function(self.parse_function(false)?));
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
                        _ => return Err(format!("line {}: expected function/task after virtual", self.peek_line())),
                    }
                }
                Token::Task => {
                    members.push(ClassMember::Task(self.parse_task(false)?));
                }
                Token::Input | Token::Output | Token::Inout | Token::Reg | Token::Logic | Token::Wire | Token::Int | Token::Integer | Token::Signed => {
                    members.push(ClassMember::Decl(self.parse_decl()?));
                }
                _ => {
                    // Skip unknown tokens (constraints, etc.) to avoid getting stuck
                    self.advance();
                }
            }
        }
        Ok(ClassDecl { name, extends, members })
    }

    fn parse_module(&mut self) -> Result<Module, String> {
        self.advance(); // consume 'module' or 'interface'
        self.typedef_names.clear();

        let name_tok = self.peek().clone();
        let name = match &name_tok {
            Token::Ident(s) => {
                self.advance();
                s.clone()
            }
            _ => return Err(format!("line {}: expected module name", self.peek_line())),
        };

        let mut ports = Vec::new();
        let mut params = Vec::new();

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

        let mut decls = Vec::new();
        let mut items = Vec::new();

        loop {
            match self.peek() {
                Token::Endmodule | Token::EndInterface | Token::Eof => break,
                _ => {
                    if let Some(item) = self.parse_module_item()? {
                        match item {
                            ModuleItem::Decl(d) => decls.push(d),
                            other => items.push(other),
                        }
                    }
                }
            }
        }

        self.expect(Token::Endmodule)?;

        Ok(Module { name, ports, params, decls, items })
    }

    fn parse_param_list(&mut self, params: &mut Vec<ParamDecl>) -> Result<(), String> {
        loop {
            match self.peek() {
                Token::Param | Token::Parameter | Token::LocalParam => {
                    self.advance();
                }
                _ => {}
            }

            let tok = self.peek().clone();
            match tok {
                Token::Ident(_) | Token::Int | Token::Integer => {}
                _ => break,
            }

            let name_tok = self.peek().clone();
            let name = match &name_tok {
                Token::Ident(s) => {
                    self.advance();
                    s.clone()
                }
                Token::Int => { self.advance(); "int".to_string() }
                Token::Integer => { self.advance(); "integer".to_string() }
                _ => break,
            };

            let mut dtype = None;
            if self.peek() == &Token::Signed {
                self.advance();
                dtype = Some(DataType::Signed(Box::new(DataType::Int)));
            }

            let default = if self.peek() == &Token::BlockingAssign {
                self.advance();
                Some(self.parse_expr(0)?)
            } else {
                None
            };

            params.push(ParamDecl { name, dtype, default });

            if self.peek() == &Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        Ok(())
    }

    fn parse_port_list(&mut self, ports: &mut Vec<Port>) -> Result<(), String> {
        loop {
            if self.peek() == &Token::RParen || self.peek() == &Token::Eof {
                break;
            }

            let tok = self.peek().clone();
            match tok {
                Token::Dot => {
                    self.advance();
                    match self.peek() {
                        Token::Ident(_) => { self.advance(); }
                        _ => return Err(format!("line {}: expected port name", self.peek_line())),
                    }
                    self.expect(Token::LParen)?;
                    if self.peek() != &Token::RParen {
                        self.parse_expr(0)?;
                    }
                    self.expect(Token::RParen)?;
                }
                _ => {
                    let dir = match self.peek() {
                        Token::Input => { self.advance(); PortDirection::Input }
                        Token::Output => { self.advance(); PortDirection::Output }
                        Token::Inout => { self.advance(); PortDirection::Inout }
                        _ => PortDirection::Input,
                    };

                    if matches!(self.peek(), Token::Wire | Token::Reg | Token::Logic) {
                        self.advance();
                    }

                    if self.peek() == &Token::Signed {
                        self.advance();
                    }

                    let expr_range = if self.peek() == &Token::LBrack {
                        self.parse_range()?
                    } else {
                        None
                    };
                    let range = expr_range.as_ref().and_then(|er| {
                        if let (Ok(m), Ok(l)) = (const_eval_simple(&er.msb), const_eval_simple(&er.lsb)) {
                            Some(Range { msb: m as usize, lsb: l as usize })
                        } else {
                            None
                        }
                    });

                    loop {
                        let name_tok = self.peek().clone();
                        match &name_tok {
                            Token::Ident(name) => {
                                self.advance();
                                ports.push(Port {
                                    name: name.clone(),
                                    direction: dir.clone(),
                                    range: range.clone(),
                                    expr_range: expr_range.clone(),
                                });
                            }
                            _ => break,
                        }

                        if self.peek() == &Token::Comma && self.peek_ahead(1) != &Token::Input
                            && self.peek_ahead(1) != &Token::Output
                            && self.peek_ahead(1) != &Token::Inout
                        {
                            self.advance();
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

    fn parse_range(&mut self) -> Result<Option<ExprRange>, String> {
        self.expect(Token::LBrack)?;
        let msb = self.parse_expr(0)?;
        self.expect(Token::Colon)?;
        let lsb = self.parse_expr(0)?;
        self.expect(Token::RBrack)?;
        Ok(Some(ExprRange { msb, lsb }))
    }

    fn parse_module_item(&mut self) -> Result<Option<ModuleItem>, String> {
        match self.peek() {
            Token::Always | Token::AlwaysComb | Token::AlwaysFF | Token::AlwaysLatch => {
                let always = self.parse_always()?;
                Ok(Some(ModuleItem::Always(always)))
            }
            Token::Initial => {
                let initial = self.parse_initial()?;
                Ok(Some(ModuleItem::Initial(initial)))
            }
            Token::Assign => {
                let assign = self.parse_assign()?;
                Ok(Some(ModuleItem::Assign(assign)))
            }
            Token::Wire | Token::Reg | Token::Logic | Token::Int | Token::Integer
                | Token::Bit | Token::Byte | Token::Shortint | Token::Longint
                | Token::Enum | Token::Struct | Token::Union => {
                let decl = self.parse_decl()?;
                Ok(Some(ModuleItem::Decl(decl)))
            }
            Token::Ident(name) => {
                if self.class_names.contains(name) || self.typedef_names.contains(name) {
                    let dtype = DataType::UserDefined(name.clone());
                    self.advance();
                    let mut names = Vec::new();
                    loop {
                        if let Token::Ident(n) = self.peek() {
                            let vname = n.clone();
                            self.advance();
                            names.push(DeclVar {
                                name: vname, range: None, expr_range: None, array_range: None,
                            });
                        } else { break; }
                        if self.peek() == &Token::Comma { self.advance(); } else { break; }
                    }
                    self.skip_semi();
                    Ok(Some(ModuleItem::Decl(Decl { dtype, kind: DeclKind::Logic, names })))
                } else if matches!(self.peek_ahead(1), Token::Ident(_))
                    || self.peek_ahead(1) == &Token::Hash
                    || self.peek_ahead(1) == &Token::LParen
                {
                    let instance = self.parse_instance()?;
                    Ok(Some(ModuleItem::Instance(instance)))
                } else {
                    let line = self.peek_line();
                    Err(format!("line {}: unexpected token in module body: {}", line, self.peek()))
                }
            }
            Token::Generate => {
                let gen = self.parse_generate_block()?;
                Ok(Some(ModuleItem::Generate(gen)))
            }
            Token::GenVar | Token::Param | Token::Parameter | Token::LocalParam => {
                self.skip_until_semi_or_end()?;
                Ok(None)
            }
            Token::Function => {
                let func = self.parse_function(false)?;
                Ok(Some(ModuleItem::Func(func)))
            }
            Token::Task => {
                let task = self.parse_task(false)?;
                // Treat tasks as functions for now (the engine can handle them)
                Ok(Some(ModuleItem::Func(FunctionDecl {
                    name: task.name,
                    range: None,
                    return_type: None,
                    ports: task.ports,
                    decls: task.decls,
                    stmts: task.stmts,
                    virtual_flag: task.virtual_flag,
                })))
            }
            Token::And | Token::Or | Token::Nand | Token::Nor | Token::Xor | Token::Xnor => {
                let gate = self.parse_gate_primitive()?;
                Ok(Some(ModuleItem::Gate(gate)))
            }
            Token::Buf | Token::NotGate => {
                let gate = self.parse_gate_primitive()?;
                Ok(Some(ModuleItem::Gate(gate)))
            }
            Token::Typedef => {
                let td = self.parse_typedef()?;
                self.typedef_names.push(td.name.clone());
                Ok(Some(ModuleItem::Typedef(td)))
            }
            Token::Import => {
                self.advance();
                let pkg = self.expect_ident()?;
                self.expect(Token::Scope)?;
                let item = if self.peek() == &Token::Star {
                    self.advance();
                    "*".to_string()
                } else {
                    self.expect_ident()?
                };
                self.skip_semi();
                Ok(Some(ModuleItem::Import { package: pkg, item }))
            }
            Token::Assert | Token::Assume | Token::Cover | Token::Expect => {
                self.parse_immediate_assertion().map(Some)
            }
            _ => Ok(None),
        }
    }

    fn skip_until_semi_or_end(&mut self) -> Result<(), String> {
        let mut depth: i32 = 0;
        loop {
            match self.peek() {
                Token::Semi if depth == 0 => {
                    self.advance();
                    return Ok(());
                }
                Token::Endmodule | Token::EndFunction | Token::EndTask | Token::Eof => {
                    return Ok(());
                }
                Token::Begin => { depth += 1; self.advance(); }
                Token::End => { depth = depth.saturating_sub(1); self.advance(); }
                _ => { self.advance(); }
            }
        }
    }

    fn parse_decl(&mut self) -> Result<Decl, String> {
        let kind = match self.peek() {
            Token::Wire => DeclKind::Wire,
            Token::Reg => DeclKind::Reg,
            Token::Logic => DeclKind::Logic,
            Token::Int => DeclKind::Int,
            Token::Integer => DeclKind::Integer,
            Token::Bit | Token::Byte | Token::Shortint | Token::Longint => {
                let dt = match self.peek() {
                    Token::Bit => DataType::Bit,
                    Token::Byte => DataType::Byte,
                    Token::Shortint => DataType::Shortint,
                    _ => DataType::Longint,
                };
                self.advance();
                let mut dtype = dt;
                if self.peek() == &Token::Signed { self.advance(); dtype = DataType::Signed(Box::new(dtype)); }
                let decl_expr_range = if self.peek() == &Token::LBrack { self.parse_range()? } else { None };
                let names = self.parse_decl_names(decl_expr_range)?;
                self.skip_semi();
                return Ok(Decl { dtype, kind: DeclKind::Logic, names });
            }
            Token::Enum => {
                self.advance();
                let base = match self.peek() {
                    Token::Bit | Token::Logic | Token::Int | Token::Integer
                        | Token::Byte | Token::Shortint | Token::Longint => {
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
                let names = self.parse_decl_names(decl_expr_range)?;
                self.skip_semi();
                return Ok(Decl { dtype: DataType::EnumType { base, members }, kind: DeclKind::Logic, names });
            }
            Token::Struct => {
                self.advance();
                let members = self.parse_struct_body()?;
                let names = self.parse_decl_names(None)?;
                self.skip_semi();
                return Ok(Decl { dtype: DataType::StructType { members }, kind: DeclKind::Logic, names });
            }
            Token::Union => {
                self.advance();
                let members = self.parse_struct_body()?;
                let names = self.parse_decl_names(None)?;
                self.skip_semi();
                return Ok(Decl { dtype: DataType::UnionType { members }, kind: DeclKind::Logic, names });
            }
            _ => return Err(format!("line {}: expected wire/reg/logic/int/byte/shortint/longint/enum/struct/union", self.peek_line())),
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

        let decl_expr_range = if self.peek() == &Token::LBrack {
            self.parse_range()?
        } else {
            None
        };

        let names = self.parse_decl_names(decl_expr_range)?;
        self.skip_semi();

        Ok(Decl { dtype, kind, names })
    }

    fn parse_decl_names(&mut self, decl_expr_range: Option<ExprRange>) -> Result<Vec<DeclVar>, String> {
        let mut names = Vec::new();
        loop {
            let name_tok = self.peek().clone();
            match &name_tok {
                Token::Ident(name) => {
                    self.advance();
                    let (var_expr_range, array_range) = if decl_expr_range.is_some() {
                        let ar = if self.peek() == &Token::LBrack {
                            let er = self.parse_range()?;
                            er.as_ref().and_then(|er| {
                                if let (Ok(m), Ok(l)) = (const_eval_simple(&er.msb), const_eval_simple(&er.lsb)) {
                                    Some(Range { msb: m as usize, lsb: l as usize })
                                } else {
                                    None
                                }
                            })
                        } else {
                            None
                        };
                        (decl_expr_range.clone(), ar)
                    } else {
                        let ver = if self.peek() == &Token::LBrack {
                            self.parse_range()?
                        } else {
                            None
                        };
                        let ar = if self.peek() == &Token::LBrack {
                            let er = self.parse_range()?;
                            er.as_ref().and_then(|er| {
                                if let (Ok(m), Ok(l)) = (const_eval_simple(&er.msb), const_eval_simple(&er.lsb)) {
                                    Some(Range { msb: m as usize, lsb: l as usize })
                                } else {
                                    None
                                }
                            })
                        } else {
                            None
                        };
                        (ver, ar)
                    };
                    let var_range = var_expr_range.as_ref().and_then(|er| {
                        if let (Ok(m), Ok(l)) = (const_eval_simple(&er.msb), const_eval_simple(&er.lsb)) {
                            Some(Range { msb: m as usize, lsb: l as usize })
                        } else {
                            None
                        }
                    });
                    names.push(DeclVar {
                        name: name.clone(),
                        range: var_range,
                        expr_range: var_expr_range,
                        array_range,
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

    fn parse_enum_members(&mut self) -> Result<Vec<(String, Option<Expr>)>, String> {
        self.expect(Token::LBrace)?;
        let mut members = Vec::new();
        loop {
            match self.peek() {
                Token::Ident(name) => {
                    let name = name.clone();
                    self.advance();
                    let val = if self.peek() == &Token::Eq {
                        self.advance();
                        Some(self.parse_expr(0)?)
                    } else {
                        None
                    };
                    members.push((name, val));
                }
                _ => return Err(format!("line {}: expected identifier in enum", self.peek_line())),
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

    fn parse_struct_body(&mut self) -> Result<Vec<StructMember>, String> {
        self.expect(Token::LBrace)?;
        let mut members = Vec::new();
        loop {
            if self.peek() == &Token::RBrace {
                self.advance();
                return Ok(members);
            }
            let member_type = match self.peek() {
                Token::Logic => { self.advance(); DataType::Logic }
                Token::Int => { self.advance(); DataType::Int }
                Token::Integer => { self.advance(); DataType::Integer }
                Token::Bit => { self.advance(); DataType::Bit }
                Token::Byte => { self.advance(); DataType::Byte }
                Token::Shortint => { self.advance(); DataType::Shortint }
                Token::Longint => { self.advance(); DataType::Longint }
                Token::Reg => { self.advance(); DataType::Logic }
                Token::Signed => {
                    self.advance();
                    let inner = match self.peek() {
                        Token::Bit => { self.advance(); DataType::Bit }
                        Token::Logic => { self.advance(); DataType::Logic }
                        Token::Int => { self.advance(); DataType::Int }
                        Token::Integer => { self.advance(); DataType::Integer }
                        Token::Byte => { self.advance(); DataType::Byte }
                        Token::Shortint => { self.advance(); DataType::Shortint }
                        Token::Longint => { self.advance(); DataType::Longint }
                        _ => DataType::Logic,
                    };
                    DataType::Signed(Box::new(inner))
                }
                _ => return Err(format!("line {}: expected type in struct/union member", self.peek_line())),
            };
            let range = if self.peek() == &Token::LBrack {
                let er = self.parse_range()?;
                er.as_ref().and_then(|er| {
                    if let (Ok(m), Ok(l)) = (const_eval_simple(&er.msb), const_eval_simple(&er.lsb)) {
                        Some(Range { msb: m as usize, lsb: l as usize })
                    } else {
                        None
                    }
                })
            } else {
                None
            };
            let name = self.expect_ident()?;
            self.skip_semi();
            members.push(StructMember { name, dtype: Box::new(member_type), range });
        }
    }

    fn parse_typedef(&mut self) -> Result<TypedefDecl, String> {
        self.advance(); // consume typedef
        let (name, dtype) = match self.peek() {
            Token::Enum => {
                self.advance();
                let base = match self.peek() {
                    Token::Bit | Token::Logic | Token::Int | Token::Integer
                        | Token::Byte | Token::Shortint | Token::Longint => {
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
                if base.is_some() && self.peek() == &Token::LBrack {
                    self.parse_range()?;
                }
                let members = self.parse_enum_members()?;
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, DataType::EnumType { base, members })
                } else {
                    return Err(format!("line {}: expected name after typedef enum", self.peek_line()));
                }
            }
            Token::Bit => {
                self.advance();
                let mut dtype = DataType::Bit;
                if self.peek() == &Token::Signed { self.advance(); dtype = DataType::Signed(Box::new(dtype)); }
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype)
                } else {
                    return Err(format!("line {}: expected name after typedef bit", self.peek_line()));
                }
            }
            Token::Byte => {
                self.advance();
                let mut dtype = DataType::Byte;
                if self.peek() == &Token::Signed { self.advance(); dtype = DataType::Signed(Box::new(dtype)); }
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype)
                } else {
                    return Err(format!("line {}: expected name after typedef byte", self.peek_line()));
                }
            }
            Token::Shortint => {
                self.advance();
                let mut dtype = DataType::Shortint;
                if self.peek() == &Token::Signed { self.advance(); dtype = DataType::Signed(Box::new(dtype)); }
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype)
                } else {
                    return Err(format!("line {}: expected name after typedef shortint", self.peek_line()));
                }
            }
            Token::Longint => {
                self.advance();
                let mut dtype = DataType::Longint;
                if self.peek() == &Token::Signed { self.advance(); dtype = DataType::Signed(Box::new(dtype)); }
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype)
                } else {
                    return Err(format!("line {}: expected name after typedef longint", self.peek_line()));
                }
            }
            Token::Int => {
                self.advance();
                let mut dtype = DataType::Int;
                if self.peek() == &Token::Signed { self.advance(); dtype = DataType::Signed(Box::new(dtype)); }
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype)
                } else {
                    return Err(format!("line {}: expected name after typedef int", self.peek_line()));
                }
            }
            Token::Integer => {
                self.advance();
                let mut dtype = DataType::Integer;
                if self.peek() == &Token::Signed { self.advance(); dtype = DataType::Signed(Box::new(dtype)); }
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype)
                } else {
                    return Err(format!("line {}: expected name after typedef integer", self.peek_line()));
                }
            }
            Token::Logic => {
                self.advance();
                let mut dtype = DataType::Logic;
                if self.peek() == &Token::Signed { self.advance(); dtype = DataType::Signed(Box::new(dtype)); }
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype)
                } else {
                    return Err(format!("line {}: expected name after typedef logic", self.peek_line()));
                }
            }
            Token::Reg => {
                self.advance();
                let dtype = DataType::Logic;
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype)
                } else {
                    return Err(format!("line {}: expected name after typedef reg", self.peek_line()));
                }
            }
            Token::Struct => {
                self.advance();
                let members = self.parse_struct_body()?;
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, DataType::StructType { members })
                } else {
                    return Err(format!("line {}: expected name after typedef struct", self.peek_line()));
                }
            }
            Token::Union => {
                self.advance();
                let members = self.parse_struct_body()?;
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, DataType::UnionType { members })
                } else {
                    return Err(format!("line {}: expected name after typedef union", self.peek_line()));
                }
            }
            _ => return Err(format!("line {}: expected type after typedef", self.peek_line())),
        };
        self.skip_semi();
        Ok(TypedefDecl { name, dtype })
    }

    fn parse_generate_block(&mut self) -> Result<GenerateBlock, String> {
        self.advance(); // consume 'generate'
        let mut items = Vec::new();
        loop {
            match self.peek() {
                Token::EndGenerate => {
                    self.advance();
                    return Ok(GenerateBlock { items });
                }
                Token::Eof => {
                    return Err("line {}: unexpected EOF in generate block".to_string());
                }
                _ => {
                    let item = self.parse_generate_item()?;
                    items.push(item);
                }
            }
        }
    }

    fn parse_generate_item(&mut self) -> Result<GenerateItem, String> {
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
                Ok(GenerateItem::If { cond, true_items, false_items })
            }
            Token::For => {
                self.advance();
                self.expect(Token::LParen)?;
                let var_tok = self.peek().clone();
                let var = match &var_tok {
                    Token::Ident(n) => { self.advance(); n.clone() }
                    _ => return Err(format!("line {}: expected genvar name", self.peek_line())),
                };
                // Parse init: i = <expr>
                let _init = if self.peek() != &Token::Semi {
                    self.expect(Token::BlockingAssign)?;
                    let init_expr = self.parse_expr(0)?;
                    self.expect(Token::Semi)?;
                    Some(Stmt::BlockingAssign {
                        lhs: Expr::Ident(var.clone()),
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
                Ok(GenerateItem::For { var, init: None, cond, step, body_items })
            }
            Token::GenVar => {
                self.skip_until_semi_or_end()?;
                // genvar declaration - skip, handled by For loop above
                self.parse_generate_item()
            }
            Token::Case | Token::CaseX | Token::CaseZ => {
                // Simple case: skip until endcase
                self.advance();
                let _expr = self.parse_expr(0)?;
                let mut case_items = Vec::new();
                loop {
                    match self.peek() {
                        Token::Endcase => { self.advance(); break; }
                        Token::Default => {
                            self.advance(); self.expect(Token::Colon)?;
                            let _ = self.parse_generate_block_body()?;
                        }
                        _ => {
                            let _labels = vec![self.parse_expr(0)?];
                            self.expect(Token::Colon)?;
                            let body = self.parse_generate_block_body()?;
                            case_items.push(body);
                        }
                    }
                }
                // For now, flatten case by picking the matching arm (elaboration-time)
                // Keep as Items for the elaborator to handle
                Ok(GenerateItem::Items(vec![])) // Placeholder
            }
            _ => {
                let item = self.parse_module_item()?;
                match item {
                    Some(mi) => Ok(GenerateItem::Items(vec![mi])),
                    None => self.parse_generate_item(),
                }
            }
        }
    }

    fn parse_generate_block_body(&mut self) -> Result<Vec<ModuleItem>, String> {
        if self.peek() == &Token::Begin {
            self.advance();
            let mut items = Vec::new();
            loop {
                if matches!(self.peek(), Token::End | Token::Eof) {
                    self.advance();
                    break;
                }
                match self.parse_module_item()? {
                    Some(mi) => items.push(mi),
                    None => {}
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

    fn parse_function(&mut self, virtual_flag: bool) -> Result<FunctionDecl, String> {
        self.advance(); // consume 'function'
        // Parse optional return type
        let return_type = match self.peek() {
            Token::Void => { self.advance(); Some(Box::new(DataType::Bit)) } // void -> bit placeholder
            Token::Int => { self.advance(); Some(Box::new(DataType::Int)) }
            Token::Integer => { self.advance(); Some(Box::new(DataType::Integer)) }
            Token::String => { self.advance(); Some(Box::new(DataType::UserDefined("string".into()))) }
            Token::Byte => { self.advance(); Some(Box::new(DataType::Byte)) }
            Token::Shortint => { self.advance(); Some(Box::new(DataType::Shortint)) }
            Token::Longint => { self.advance(); Some(Box::new(DataType::Longint)) }
            Token::Bit => { self.advance(); Some(Box::new(DataType::Bit)) }
            Token::Logic => { self.advance(); Some(Box::new(DataType::Logic)) }
            Token::Signed => { self.advance(); Some(Box::new(DataType::Signed(Box::new(DataType::Logic)))) }
            _ => None,
        };
        let range = if self.peek() == &Token::LBrack {
            self.parse_range()?
        } else {
            None
        };
        let name_tok = self.peek().clone();
        let name = match &name_tok {
            Token::Ident(n) => { self.advance(); n.clone() }
            Token::New => { self.advance(); "new".to_string() }
            _ => return Err(format!("line {}: expected function name", self.peek_line())),
        };
        // Parse ANSI-style port list in parens (e.g., function new(int level, string name))
        let mut ports = Vec::new();
        let mut decls = Vec::new();
        if self.peek() == &Token::LParen {
            self.advance();
            while self.peek() != &Token::RParen && self.peek() != &Token::Eof {
                // Track whether we saw int/integer for 32-bit default width
                let is_int = matches!(self.peek(), Token::Int | Token::Integer);
                // Skip type keywords
                if matches!(self.peek(),
                    Token::Int | Token::Integer | Token::String | Token::Void |
                    Token::Reg | Token::Logic | Token::Wire | Token::Signed)
                {
                    self.advance();
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
                Token::Input | Token::Output | Token::Inout => {
                    let _direction = match self.peek() {
                        Token::Input => { self.advance(); PortDirection::Input }
                        Token::Output => { self.advance(); PortDirection::Output }
                        _ => { self.advance(); PortDirection::Inout }
                    };
                    let port_range = if self.peek() == &Token::LBrack {
                        let er = self.parse_range()?;
                        er.as_ref().and_then(|er| {
                            if let (Ok(m), Ok(l)) = (const_eval_simple(&er.msb), const_eval_simple(&er.lsb)) {
                                Some(Range { msb: m as usize, lsb: l as usize })
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
                Token::Begin => {
                    let stmts = self.parse_stmt_block()?;
                    self.expect(Token::EndFunction)?;
                    return Ok(FunctionDecl { name, range, return_type, ports, decls, stmts, virtual_flag });
                }
                Token::EndFunction => {
                    return Ok(FunctionDecl { name, range, return_type, ports, decls, stmts: vec![], virtual_flag });
                }
                _ => break,
            }
        }
        // No begin/end block - parse statements until endfunction
        let mut stmts = Vec::new();
        loop {
            if matches!(self.peek(), Token::EndFunction | Token::End | Token::Eof) {
                break;
            }
            stmts.push(self.parse_stmt()?);
        }
        self.expect(Token::EndFunction)?;
        Ok(FunctionDecl { name, range, return_type, ports, decls, stmts, virtual_flag })
    }

    fn parse_task(&mut self, virtual_flag: bool) -> Result<TaskDecl, String> {
        self.advance(); // consume 'task'
        let name = self.expect_ident()?;
        // Skip optional ANSI-style port list in parens
        if self.peek() == &Token::LParen {
            self.skip_paren_block();
        }
        self.skip_semi();
        let mut ports = Vec::new();
        let mut decls = Vec::new();
        loop {
            match self.peek() {
                Token::Input | Token::Output | Token::Inout => {
                    let _ = match self.peek() {
                        Token::Input => self.advance(),
                        Token::Output => self.advance(),
                        _ => self.advance(),
                    };
                    if self.peek() == &Token::LBrack {
                        self.skip_brack_block();
                    }
                    loop {
                        match self.peek() {
                            Token::Ident(pname) => {
                                let pn = pname.clone();
                                self.advance();
                                ports.push(FunctionPort { name: pn, range: None, expr_range: None });
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
                    return Ok(TaskDecl { name, ports, decls, stmts, virtual_flag });
                }
                Token::EndTask => {
                    return Ok(TaskDecl { name, ports, decls, stmts: vec![], virtual_flag });
                }
                _ => break,
            }
        }
        let mut stmts = Vec::new();
        loop {
            if matches!(self.peek(), Token::EndTask | Token::End | Token::Eof) {
                break;
            }
            stmts.push(self.parse_stmt()?);
        }
        self.expect(Token::EndTask)?;
        Ok(TaskDecl { name, ports, decls, stmts, virtual_flag })
    }

    fn parse_always(&mut self) -> Result<AlwaysBlock, String> {
        let kind = match self.peek() {
            Token::Always => { self.advance(); AlwaysKind::Always }
            Token::AlwaysComb => { self.advance(); AlwaysKind::AlwaysComb }
            Token::AlwaysFF => { self.advance(); AlwaysKind::AlwaysFF }
            Token::AlwaysLatch => { self.advance(); AlwaysKind::AlwaysLatch }
            _ => unreachable!(),
        };

        let sensitivity = if self.peek() == &Token::At {
            self.advance();
            Some(self.parse_sensitivity_list()?)
        } else {
            None
        };

        let stmts = self.parse_stmt_block()?;

        Ok(AlwaysBlock { kind, sensitivity, stmts })
    }

    fn parse_initial(&mut self) -> Result<InitialBlock, String> {
        self.advance();
        let stmts = self.parse_stmt_block()?;
        Ok(InitialBlock { stmts })
    }

    fn parse_sensitivity_events(&mut self) -> Result<Vec<SensitivityEvent>, String> {
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

    fn parse_sensitivity_list(&mut self) -> Result<SensitivityList, String> {
        self.expect(Token::LParen)?;
        let events = self.parse_sensitivity_events()?;
        self.expect(Token::RParen)?;
        Ok(SensitivityList { events })
    }

    fn parse_assign(&mut self) -> Result<ContinuousAssign, String> {
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

    fn parse_delay(&mut self) -> Result<Delay, String> {
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
        Ok(Delay { rise, fall, turnoff })
    }

    fn parse_instance(&mut self) -> Result<ModuleInstance, String> {
        let name_tok = self.peek().clone();
        let module_name = match &name_tok {
            Token::Ident(s) => {
                self.advance();
                s.clone()
            }
            _ => return Err(format!("line {}: expected module name", self.peek_line())),
        };

        let mut param_assigns = std::collections::HashMap::new();

        if self.peek() == &Token::Hash {
            self.advance();
            self.expect(Token::LParen)?;
            if self.peek() != &Token::RParen {
                loop {
                    if self.peek() == &Token::Dot {
                        self.advance();
                        let pname_tok = self.peek().clone();
                        let pname = match &pname_tok {
                            Token::Ident(s) => { self.advance(); s.clone() }
                            _ => return Err(format!("line {}: expected parameter name", self.peek_line())),
                        };
                        self.expect(Token::LParen)?;
                        let val = self.parse_expr(0)?;
                        self.expect(Token::RParen)?;
                        param_assigns.insert(pname, val);
                    } else {
                        let val = self.parse_expr(0)?;
                        param_assigns.insert(format!("__param{}", param_assigns.len()), val);
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
                s.clone()
            }
            _ => return Err(format!("line {}: expected instance name", self.peek_line())),
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
                            Token::Ident(s) => { self.advance(); s.clone() }
                            _ => return Err(format!("line {}: expected port name", self.peek_line())),
                        };

                        if self.peek() == &Token::LParen {
                            self.advance();
                            let expr = if self.peek() != &Token::RParen {
                                self.parse_expr(0)?
                            } else {
                                Expr::Value(Value::Decimal(0))
                            };
                            self.expect(Token::RParen)?;
                            port_conns.push(PortConnection::Named { port: port_name, expr });
                        } else {
                            port_conns.push(PortConnection::Named {
                                port: port_name.clone(),
                                expr: Expr::Ident(port_name),
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

        self.skip_semi();

        Ok(ModuleInstance { module_name, instance_name, param_assigns, port_conns })
    }

    fn parse_gate_primitive(&mut self) -> Result<GatePrimitive, String> {
        let gate_type = match self.peek() {
            Token::And => { self.advance(); GateType::And }
            Token::Or => { self.advance(); GateType::Or }
            Token::Nand => { self.advance(); GateType::Nand }
            Token::Nor => { self.advance(); GateType::Nor }
            Token::Xor => { self.advance(); GateType::Xor }
            Token::Xnor => { self.advance(); GateType::Xnor }
            Token::Buf => { self.advance(); GateType::Buf }
            Token::NotGate => { self.advance(); GateType::Not }
            _ => return Err(format!("line {}: expected gate type", self.peek_line())),
        };
        let instance_name = if self.peek() == &Token::LParen {
            None
        } else {
            let name = match self.peek().clone() {
                Token::Ident(s) => { self.advance(); Some(s) }
                _ => return Err(format!("line {}: expected gate instance name", self.peek_line())),
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
        Ok(GatePrimitive { gate_type, instance_name, ports })
    }

    fn parse_stmt_block(&mut self) -> Result<Vec<Stmt>, String> {
        if self.peek() == &Token::Begin {
            self.advance();
            if self.peek() == &Token::Colon {
                self.advance();
                // Skip the block name for stmt_block (just consume it)
                if let Token::Ident(_) = self.peek() {
                    self.advance();
                }
            }
            let mut stmts = Vec::new();
            loop {
                if self.peek() == &Token::End || self.peek() == &Token::Eof {
                    self.advance();
                    break;
                }
                stmts.push(self.parse_stmt()?);
            }
            Ok(stmts)
        } else {
            let stmt = self.parse_stmt()?;
            Ok(vec![stmt])
        }
    }

    fn parse_immediate_assertion(&mut self) -> Result<Stmt, String> {
        let kind = match self.peek() {
            Token::Assert => { self.advance(); "assert" }
            Token::Assume => { self.advance(); "assume" }
            Token::Cover => { self.advance(); "cover" }
            Token::Expect => { self.advance(); "expect" }
            _ => return Err("expected assert/assume/cover/expect".to_string()),
        };
        // Handle "assert property (...)" / "cover property (...)" — skip property for now
        if self.peek() == &Token::Property {
            self.advance();
            // Parse the property expression
            self.expect(Token::LParen)?;
            let _expr = self.parse_expr(0)?;
            self.expect(Token::RParen)?;
            // Parse optional action block
            let _pass_stmt = if self.peek() == &Token::Else {
                self.advance();
                Some(Box::new(self.parse_stmt()?))
            } else {
                None
            };
            self.skip_semi();
            // For now, treat property assertions as no-ops
            return Ok(Stmt::Null);
        }
        // Immediate assertions: assert (expr) [else stmt]
        self.expect(Token::LParen)?;
        let cond = self.parse_expr(0)?;
        self.expect(Token::RParen)?;
        let pass_stmt = if kind == "cover" {
            None
        } else if self.peek() != &Token::Semi && self.peek() != &Token::Else {
            let stmt = self.parse_stmt()?;
            Some(Box::new(stmt))
        } else {
            None
        };
        let fail_stmt = if self.peek() == &Token::Else {
            self.advance();
            Some(Box::new(self.parse_stmt()?))
        } else {
            None
        };
        self.skip_semi();
        match kind {
            "assert" => Ok(Stmt::Assert { cond, pass_stmt, fail_stmt }),
            "assume" => Ok(Stmt::Assume { cond, pass_stmt, fail_stmt }),
            "cover" => Ok(Stmt::Cover { cond, pass_stmt }),
            "expect" => Ok(Stmt::Expect { cond, pass_stmt, fail_stmt }),
            _ => unreachable!(),
        }
    }

    fn parse_wait_order(&mut self) -> Result<Stmt, String> {
        self.advance(); // consume 'wait_order'
        self.expect(Token::LParen)?;
        let mut events = Vec::new();
        if self.peek() != &Token::RParen {
            loop {
                events.push(self.expect_ident()?);
                if self.peek() == &Token::Comma {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        self.expect(Token::RParen)?;
        self.skip_semi();
        Ok(Stmt::WaitOrder { events })
    }

    fn parse_stmt(&mut self) -> Result<Stmt, String> {
        match self.peek() {
            Token::Assert | Token::Assume | Token::Cover | Token::Expect => {
                self.parse_immediate_assertion()
            }
            Token::Unique | Token::Priority | Token::Unique0 => {
                let qualifier = self.peek().clone();
                self.advance();
                match self.peek() {
                    Token::Case | Token::CaseX | Token::CaseZ => {
                        let stmt = self.parse_case_stmt()?;
                        // Wrap with qualifier
                        match stmt {
                            Stmt::Case { expr, items, default } => {
                                if qualifier == Token::Unique {
                                    Ok(Stmt::UniqueCase { expr, items, default })
                                } else {
                                    Ok(Stmt::PriorityCase { expr, items, default })
                                }
                            }
                            Stmt::CaseX { expr, items, default } => {
                                if qualifier == Token::Unique {
                                    Ok(Stmt::UniqueCase { expr, items, default })
                                } else {
                                    Ok(Stmt::PriorityCase { expr, items, default })
                                }
                            }
                            Stmt::CaseZ { expr, items, default } => {
                                if qualifier == Token::Unique {
                                    Ok(Stmt::UniqueCase { expr, items, default })
                                } else {
                                    Ok(Stmt::PriorityCase { expr, items, default })
                                }
                            }
                            _ => Ok(stmt),
                        }
                    }
                    Token::If => {
                        let stmt = self.parse_if_stmt()?;
                        match stmt {
                            Stmt::IfElse { cond, true_branch, false_branch } => {
                                if qualifier == Token::Unique {
                                    Ok(Stmt::UniqueIf { cond, true_branch, false_branch })
                                } else {
                                    Ok(Stmt::PriorityIf { cond, true_branch, false_branch })
                                }
                            }
                            _ => Ok(stmt),
                        }
                    }
                    _ => Err(format!("line {}: expected case or if after unique/priority/unique0", self.peek_line())),
                }
            }
            Token::Begin => {
                self.advance();
                let mut block_name = String::new();
                if self.peek() == &Token::Colon {
                    self.advance();
                    if let Token::Ident(name) = self.peek() {
                        block_name = name.clone();
                        self.advance();
                    }
                }
                let mut stmts = Vec::new();
                loop {
                    if self.peek() == &Token::End || self.peek() == &Token::Eof {
                        self.advance();
                        break;
                    }
                    stmts.push(self.parse_stmt()?);
                }
                if block_name.is_empty() {
                    Ok(Stmt::Block { stmts })
                } else {
                    Ok(Stmt::NamedBlock { name: block_name, stmts, decls: vec![] })
                }
            }
            Token::If => self.parse_if_stmt(),
            Token::Case | Token::CaseX | Token::CaseZ => self.parse_case_stmt(),
            Token::For => self.parse_for_stmt(),
            Token::Foreach => self.parse_foreach_stmt(),
            Token::While => self.parse_while_stmt(),
            Token::Forever => self.parse_forever_stmt(),
            Token::Repeat => self.parse_repeat_stmt(),
            Token::Break => {
                self.advance();
                self.skip_semi();
                Ok(Stmt::Break)
            }
            Token::Continue => {
                self.advance();
                self.skip_semi();
                Ok(Stmt::Continue)
            }
            Token::WaitOrder => self.parse_wait_order(),
            Token::Do => {
                self.advance();
                let stmts = self.parse_stmt_block()?;
                self.expect(Token::While)?;
                self.expect(Token::LParen)?;
                let cond = self.parse_expr(0)?;
                self.expect(Token::RParen)?;
                self.skip_semi();
                Ok(Stmt::DoWhile { cond, stmts })
            }
            Token::Disable => {
                self.advance();
                let tok = self.peek().clone();
                let name = match &tok {
                    Token::Ident(s) => { self.advance(); s.clone() }
                    _ => return Err(format!("line {}: expected identifier after disable", self.peek_line())),
                };
                self.skip_semi();
                Ok(Stmt::Disable { name })
            }
            Token::Force => {
                self.advance();
                let lhs = self.parse_expr(0)?;
                self.expect(Token::BlockingAssign)?;
                let rhs = self.parse_expr(0)?;
                self.skip_semi();
                Ok(Stmt::Force { lhs, rhs })
            }
            Token::Release => {
                self.advance();
                let expr = self.parse_expr(0)?;
                self.skip_semi();
                Ok(Stmt::Release { expr })
            }
            Token::Wait => {
                self.advance();
                self.expect(Token::LParen)?;
                let cond = self.parse_expr(0)?;
                self.expect(Token::RParen)?;
                if self.peek() == &Token::Semi {
                    self.advance();
                    Ok(Stmt::Wait { cond, stmt: None })
                } else {
                    let stmt = self.parse_stmt()?;
                    Ok(Stmt::Wait { cond, stmt: Some(Box::new(stmt)) })
                }
            }
            Token::Hash => {
                // #delay statement
                self.advance();
                let delay = if self.peek() == &Token::LParen {
                    self.advance();
                    let expr = self.parse_expr(0)?;
                    self.expect(Token::RParen)?;
                    expr
                } else {
                    self.parse_primary_expr()?
                };
                let stmt = self.parse_stmt()?;
                Ok(Stmt::Delay { delay, stmt: Box::new(stmt) })
            }
            Token::Dollar => self.parse_syscall(),
            Token::Return => {
                self.advance();
                if self.peek() == &Token::Semi {
                    self.advance();
                    Ok(Stmt::Return(None))
                } else {
                    let expr = self.parse_expr(0)?;
                    self.skip_semi();
                    Ok(Stmt::Return(Some(Box::new(expr))))
                }
            }
            Token::At => {
                self.advance();
                self.expect(Token::LParen)?;
                let events = self.parse_sensitivity_events()?;
                self.expect(Token::RParen)?;
                if self.peek() == &Token::Semi {
                    self.advance();
                    Ok(Stmt::EventControl { events, stmt: None })
                } else {
                    let stmt = self.parse_stmt()?;
                    Ok(Stmt::EventControl { events, stmt: Some(Box::new(stmt)) })
                }
            }
            Token::Arrow => {
                self.advance();
                let tok = self.peek().clone();
                let name = match tok {
                    Token::Ident(s) => { self.advance(); s }
                    _ => return Err(format!("line {}: expected event name after ->", self.peek_line())),
                };
                self.skip_semi();
                Ok(Stmt::EventTrigger { name })
            }
            Token::Semi => {
                self.advance();
                Ok(Stmt::Null)
            }
            _ => {
                let mut lhs = self.parse_primary_expr()?;
                // Consume postfix operators ([expr], .name) to build full lvalue
                loop {
                    match self.peek() {
                        Token::LBrack => {
                            // Check if this is a range or bit-select
                            self.advance();
                            let first = self.parse_expr(0)?;
                            if self.peek() == &Token::Colon {
                                self.advance();
                                let second = self.parse_expr(0)?;
                                self.expect(Token::RBrack)?;
                                lhs = Expr::RangeSelect {
                                    expr: Box::new(lhs),
                                    msb: Box::new(first),
                                    lsb: Box::new(second),
                                };
                            } else if self.peek() == &Token::PlusColon {
                                self.advance();
                                let width = self.parse_expr(0)?;
                                self.expect(Token::RBrack)?;
                                lhs = Expr::PartSelect {
                                    expr: Box::new(lhs),
                                    base: Box::new(first),
                                    width: Box::new(width),
                                };
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
                                lhs = Expr::BitSelect {
                                    expr: Box::new(lhs),
                                    index: Box::new(first),
                                };
                            }
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
                                        if self.peek() == &Token::Comma {
                                            self.advance();
                                        } else {
                                            break;
                                        }
                                    }
                                }
                                self.expect(Token::RParen)?;
                                lhs = Expr::MethodCall {
                                    obj: Box::new(lhs),
                                    method: member,
                                    args,
                                };
                            } else {
                                lhs = Expr::MemberAccess {
                                    obj: Box::new(lhs),
                                    field: member,
                                };
                            }
                        }
                        _ => break,
                    }
                }
                match self.peek() {
                    Token::BlockingAssign => {
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        self.skip_semi();
                        Ok(Stmt::BlockingAssign { lhs, rhs, delay: None })
                    }
                    Token::NonBlockingAssign => {
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        self.skip_semi();
                        Ok(Stmt::NonBlockingAssign { lhs, rhs, delay: None })
                    }
                    _ => {
                        if matches!(&lhs, Expr::MethodCall {..}) {
                            self.skip_semi();
                            Ok(Stmt::Expr { expr: lhs })
                        } else {
                            // Not an assignment - dummy assign (pattern for func calls)
                            self.skip_semi();
                            Ok(Stmt::StmtAssign { lhs, rhs: Expr::Value(Value::Decimal(0)) })
                        }
                    }
                }
            }
        }
    }

    fn parse_if_stmt(&mut self) -> Result<Stmt, String> {
        self.advance();
        self.expect(Token::LParen)?;
        let cond = self.parse_expr(0)?;
        self.expect(Token::RParen)?;
        let true_branch = self.parse_stmt_block()?;
        let true_stmt = if true_branch.len() == 1 {
            true_branch.into_iter().next().unwrap()
        } else {
            Stmt::Block { stmts: true_branch }
        };

        let false_branch = if self.peek() == &Token::Else {
            self.advance();
            let fb = self.parse_stmt_block()?;
            Some(Box::new(if fb.len() == 1 {
                fb.into_iter().next().unwrap()
            } else {
                Stmt::Block { stmts: fb }
            }))
        } else {
            None
        };

        Ok(Stmt::IfElse {
            cond,
            true_branch: Box::new(true_stmt),
            false_branch,
        })
    }

    fn parse_case_stmt(&mut self) -> Result<Stmt, String> {
        let is_casex = self.peek() == &Token::CaseX;
        let is_casez = self.peek() == &Token::CaseZ;
        let is_case_inside = if self.peek() == &Token::Case {
            // Check if "inside" follows "case"
            let saved = self.pos;
            self.advance();
            let is_inside = self.peek() == &Token::Inside;
            self.pos = saved; // backtrack
            is_inside
        } else {
            false
        };
        if is_case_inside {
            self.advance(); // consume 'case'
            self.advance(); // consume 'inside'
        } else {
            self.advance(); // consume 'case'/'casex'/'casez'
        }
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
                let stmts = self.parse_stmt_block()?;
                default = Some(Box::new(Stmt::Block { stmts }));
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
                let stmts = self.parse_stmt_block()?;
                items.push(CaseItem {
                    labels,
                    stmt: Box::new(Stmt::Block { stmts }),
                });
            }
        }

        self.expect(Token::Endcase)?;

        if is_case_inside {
            Ok(Stmt::CaseInside { expr, items, default })
        } else if is_casex {
            Ok(Stmt::CaseX { expr, items, default })
        } else if is_casez {
            Ok(Stmt::CaseZ { expr, items, default })
        } else {
            Ok(Stmt::Case { expr, items, default })
        }
    }

    fn parse_for_stmt(&mut self) -> Result<Stmt, String> {
        self.advance();
        self.expect(Token::LParen)?;
        let init = if self.peek() != &Token::Semi {
            let expr = self.parse_expr(0)?;
            let init_stmt = match self.peek() {
                Token::BlockingAssign => {
                    self.advance();
                    let rhs = self.parse_expr(0)?;
                    Stmt::BlockingAssign { lhs: expr, rhs, delay: None }
                }
                _ => Stmt::Null,
            };
            Some(Box::new(init_stmt))
        } else {
            None
        };
        self.expect(Token::Semi)?;
        let cond = if self.peek() != &Token::Semi {
            Some(self.parse_expr(0)?)
        } else {
            None
        };
        self.expect(Token::Semi)?;
        let step = if self.peek() != &Token::RParen {
            let expr = self.parse_expr(0)?;
            let step_stmt = match self.peek() {
                Token::BlockingAssign => {
                    self.advance();
                    let rhs = self.parse_expr(0)?;
                    Stmt::BlockingAssign { lhs: expr, rhs, delay: None }
                }
                _ => Stmt::Null,
            };
            Some(Box::new(step_stmt))
        } else {
            None
        };
        self.expect(Token::RParen)?;
        let stmts = self.parse_stmt_block()?;
        Ok(Stmt::LoopFor { init, cond, step, stmts })
    }

    fn parse_foreach_stmt(&mut self) -> Result<Stmt, String> {
        self.advance();
        self.expect(Token::LParen)?;
        let array_var = self.expect_ident()?;
        self.expect(Token::LBrack)?;
        let index_var = self.expect_ident()?;
        self.expect(Token::RBrack)?;
        self.expect(Token::RParen)?;
        let stmts = self.parse_stmt_block()?;
        Ok(Stmt::ForeachLoop { array_var, index_var, stmts })
    }

    fn parse_while_stmt(&mut self) -> Result<Stmt, String> {
        self.advance();
        self.expect(Token::LParen)?;
        let cond = self.parse_expr(0)?;
        self.expect(Token::RParen)?;
        let stmts = self.parse_stmt_block()?;
        Ok(Stmt::LoopWhile { cond, stmts })
    }

    fn parse_forever_stmt(&mut self) -> Result<Stmt, String> {
        self.advance();
        let stmts = self.parse_stmt_block()?;
        Ok(Stmt::LoopForever { stmts })
    }

    fn parse_repeat_stmt(&mut self) -> Result<Stmt, String> {
        self.advance();
        self.expect(Token::LParen)?;
        let count = self.parse_expr(0)?;
        self.expect(Token::RParen)?;
        let stmts = self.parse_stmt_block()?;
        Ok(Stmt::Repeat { count, stmts })
    }

    fn parse_syscall(&mut self) -> Result<Stmt, String> {
        self.advance();
        let name_tok = self.peek().clone();
        let name = match &name_tok {
            Token::Ident(s) => {
                self.advance();
                s.clone()
            }
            _ => return Err(format!("line {}: expected system call name after $", self.peek_line())),
        };

        match name.as_str() {
            "finish" | "stop" => {
                if self.peek() == &Token::LParen {
                    self.advance();
                    self.expect(Token::RParen)?;
                }
                self.skip_semi();
                Ok(Stmt::SysFinish)
            }
            "time" => {
                if self.peek() == &Token::LParen {
                    self.advance();
                    self.expect(Token::RParen)?;
                }
                Ok(Stmt::SysCall { name, args: vec![] })
            }
            _ => {
                self.expect(Token::LParen)?;
                let mut args = Vec::new();
                if self.peek() != &Token::RParen {
                    loop {
                        args.push(self.parse_expr(0)?);
            if self.peek() == &Token::Comma
                || matches!(self.peek(), Token::Input | Token::Output | Token::Inout | Token::Dot)
            {
                if self.peek() == &Token::Comma {
                    self.advance();
                }
            } else {
                break;
            }
                    }
                }
                self.expect(Token::RParen)?;
                self.skip_semi();
                Ok(Stmt::SysCall { name, args })
            }
        }
    }

    fn parse_expr(&mut self, min_prec: usize) -> Result<Expr, String> {
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
                Token::LBrack => {
                    self.advance();
                    if self.peek() == &Token::RBrack {
                        self.advance();
                        continue;
                    }
                    let first = self.parse_expr(0)?;
                    if self.peek() == &Token::Colon {
                        self.advance();
                        let second = self.parse_expr(0)?;
                        self.expect(Token::RBrack)?;
                        lhs = Expr::RangeSelect {
                            expr: Box::new(lhs),
                            msb: Box::new(first),
                            lsb: Box::new(second),
                        };
                    } else if self.peek() == &Token::PlusColon {
                        self.advance();
                        let width = self.parse_expr(0)?;
                        self.expect(Token::RBrack)?;
                        lhs = Expr::PartSelect {
                            expr: Box::new(lhs),
                            base: Box::new(first),
                            width: Box::new(width),
                        };
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
                        lhs = Expr::BitSelect {
                            expr: Box::new(lhs),
                            index: Box::new(first),
                        };
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
                                if self.peek() == &Token::Comma {
                                    self.advance();
                                } else {
                                    break;
                                }
                            }
                        }
                        self.expect(Token::RParen)?;
                        lhs = Expr::MethodCall {
                            obj: Box::new(lhs),
                            method: member,
                            args,
                        };
                    } else {
                        lhs = Expr::MemberAccess {
                            obj: Box::new(lhs),
                            field: member,
                        };
                    }
                    continue;
                }
                _ => None,
            };

            match op_info {
                Some((prec, op)) => {
                    if prec < min_prec {
                        break;
                    }
                    self.advance();
                    let rhs = self.parse_expr(prec + 1)?;
                    lhs = Expr::BinaryOp {
                        op,
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                    };
                }
                None => break,
            }
        }

        Ok(lhs)
    }

    fn parse_primary_expr(&mut self) -> Result<Expr, String> {
        let tok = self.peek().clone();
        match &tok {
            Token::Dollar => {
                // System function calls in expressions: $signed(...), $unsigned(...), etc.
                self.advance();
                let name_tok = self.peek().clone();
                let name = match &name_tok {
                    Token::Ident(n) => { self.advance(); n.clone() }
                    Token::Time => { self.advance(); "time".to_string() }
                    Token::Real => { self.advance(); "real".to_string() }
                    Token::RealTime => { self.advance(); "realtime".to_string() }
                    _ => return Err(format!("line {}: expected system function name", self.peek_line())),
                };
                let full_name = format!("${}", name);
                if self.peek() == &Token::LParen {
                    self.advance();
                    let mut args = Vec::new();
                    if self.peek() != &Token::RParen {
                        loop {
                            args.push(self.parse_expr(0)?);
                            if self.peek() == &Token::Comma {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(Token::RParen)?;
                    Ok(Expr::FuncCall { name: full_name, args })
                } else {
                    Ok(Expr::Ident(full_name))
                }
            }
            Token::Ident(name) => {
                self.advance();
                if self.peek() == &Token::LParen {
                    self.advance();
                    let mut args = Vec::new();
                    if self.peek() != &Token::RParen {
                        loop {
                            args.push(self.parse_expr(0)?);
                            if self.peek() == &Token::Comma {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(Token::RParen)?;
                    Ok(Expr::FuncCall { name: name.clone(), args })
                } else {
                    Ok(Expr::Ident(name.clone()))
                }
            }
            Token::Number { value, base, width } => {
                self.advance();
                let val = if let Some(base) = base {
                    match base {
                        2 => Expr::Value(Value::Binary {
                            bits: value.clone(),
                            width: *width,
                        }),
                        8 => Expr::Value(Value::Octal {
                            bits: value.clone(),
                            width: *width,
                        }),
                        10 => {
                            let n = value.parse::<i64>().unwrap_or(0);
                            Expr::Value(Value::Decimal(n))
                        }
                        16 => Expr::Value(Value::Hex {
                            bits: value.clone(),
                            width: *width,
                        }),
                        _ => Expr::Value(Value::Decimal(value.parse::<i64>().unwrap_or(0))),
                    }
                } else {
                    if let Ok(n) = value.parse::<i64>() {
                        Expr::Value(Value::Decimal(n))
                    } else {
                        Expr::Ident(value.clone())
                    }
                };
                Ok(val)
            }
            Token::RealNum(s) => {
                self.advance();
                Ok(Expr::Value(Value::Real(s.parse::<f64>().unwrap_or(0.0))))
            }
            Token::StringLit(s) => {
                self.advance();
                Ok(Expr::String(s.clone()))
            }
            Token::New => {
                self.advance();
                self.expect(Token::LParen)?;
                let mut args = Vec::new();
                if self.peek() != &Token::RParen {
                    loop {
                        args.push(self.parse_expr(0)?);
                        if self.peek() == &Token::Comma {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                }
                self.expect(Token::RParen)?;
                Ok(Expr::FuncCall { name: "new".to_string(), args })
            }
            Token::This => {
                self.advance();
                Ok(Expr::Ident("this".to_string()))
            }
            Token::Null => {
                self.advance();
                Ok(Expr::Null)
            }
            Token::Plus | Token::Minus | Token::Tilde
                | Token::Amp | Token::Pipe | Token::Caret
                | Token::TildeAmp | Token::TildePipe | Token::CaretTilde => {
                self.advance();
                let op = match &tok {
                    Token::Plus => UnaryOp::Plus,
                    Token::Minus => UnaryOp::Minus,
                    Token::Tilde => UnaryOp::BitNot,
                    Token::Amp => UnaryOp::ReductionAnd,
                    Token::Pipe => UnaryOp::ReductionOr,
                    Token::Caret => UnaryOp::ReductionXor,
                    Token::TildeAmp => UnaryOp::ReductionNand,
                    Token::TildePipe => UnaryOp::ReductionNor,
                    Token::CaretTilde => UnaryOp::ReductionXnor,
                    _ => unreachable!(),
                };
                let expr = self.parse_primary_expr()?;
                Ok(Expr::UnaryOp { op, expr: Box::new(expr) })
            }
            Token::Not => {
                self.advance();
                let expr = self.parse_primary_expr()?;
                Ok(Expr::UnaryOp {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                })
            }
            Token::LBrace => {
                self.advance();
                let mut exprs = Vec::new();
                loop {
                    if self.peek() == &Token::RBrace {
                        break;
                    }
                    // Check for replication: count{expr}
                    let item = self.parse_expr(0)?;
                    if self.peek() == &Token::LBrace {
                        self.advance();
                        let inner = self.parse_expr(0)?;
                        self.expect(Token::RBrace)?;
                        exprs.push(Expr::Replicate {
                            count: Box::new(item),
                            expr: Box::new(inner),
                        });
                    } else {
                        exprs.push(item);
                    }
                    if self.peek() == &Token::Comma {
                        self.advance();
                    } else {
                        break;
                    }
                }
                self.expect(Token::RBrace)?;
                if exprs.len() == 1 {
                    Ok(exprs.into_iter().next().unwrap())
                } else {
                    Ok(Expr::Concat(exprs))
                }
            }
            Token::LParen => {
                self.advance();
                let expr = self.parse_expr(0)?;
                self.expect(Token::RParen)?;
                Ok(Expr::Paren(Box::new(expr)))
            }
            Token::FillLit(val) => {
                self.advance();
                Ok(Expr::FillLit(*val))
            }
            _ => Err(format!("line {}: expected expression, found {}", self.peek_line(), tok)),
        }
    }
}
