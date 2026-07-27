// Module declarations: submodule files dari restrukturisasi parser/
pub mod lexer;
pub mod preprocessor;
pub mod util;
pub mod specify;
pub mod udp;
pub mod config;
pub mod package;
pub mod class;
pub mod expr;
pub mod stmt;
pub mod decl;
pub mod instance;
pub mod proc;
use crate::ast::*;
use crate::error::{ErrorContext, SimError};
use crate::intern::Symbol;
use crate::parser::lexer::*;

pub struct Parser {
    tokens: Vec<(Token, usize, usize)>,
    pos: usize,
    source_file: String,
    source_lines: Vec<String>,
    class_names: Vec<Symbol>,
    typedef_names: Vec<Symbol>,
    package_tdefs: std::collections::HashMap<Symbol, Vec<Symbol>>,
    type_param_names: Vec<Symbol>,
    file_line_map: Vec<(usize, String)>,
    recursion_depth: usize,
}

impl Parser {
    pub fn new(tokens: Vec<(Token, usize, usize)>, source_file: &str) -> Self {
        Self {
            tokens,
            pos: 0,
            source_file: source_file.to_string(),
            source_lines: Vec::new(),
            class_names: vec![
                Symbol::intern("process"),
                Symbol::intern("uvm_object"),
                Symbol::intern("uvm_component"),
                Symbol::intern("uvm_sequence_item"),
                Symbol::intern("uvm_sequence"),
                Symbol::intern("uvm_sequencer"),
                Symbol::intern("uvm_driver"),
                Symbol::intern("uvm_monitor"),
                Symbol::intern("uvm_scoreboard"),
                Symbol::intern("uvm_analysis_port"),
                Symbol::intern("uvm_analysis_imp"),
                Symbol::intern("uvm_test"),
                Symbol::intern("uvm_config_db"),
                Symbol::intern("uvm_report_object"),
                Symbol::intern("uvm_factory"),
                Symbol::intern("uvm_resource_db"),
            ],
            typedef_names: Vec::new(),
            package_tdefs: std::collections::HashMap::new(),
            type_param_names: Vec::new(),
            file_line_map: Vec::new(),
            recursion_depth: 0,
        }
    }

    pub fn with_source_lines(mut self, source: &str) -> Self {
        self.source_lines = source.lines().map(|s| s.to_string()).collect();
        self
    }

    pub fn with_file_line_map(mut self, map: Vec<(usize, String)>) -> Self {
        self.file_line_map = map;
        self
    }

    fn resolve_source_file(&self, cumulative_line: usize) -> (String, usize) {
        let mut best_file = self.source_file.clone();
        let mut best_line: usize = 0;
        for (d_line, file) in &self.file_line_map {
            if *d_line < cumulative_line && *d_line >= best_line {
                best_line = *d_line;
                best_file = file.clone();
            }
        }
        let file_relative = if best_line > 0 {
            cumulative_line - best_line
        } else {
            cumulative_line
        };
        (best_file, file_relative)
    }

    fn push_depth(&mut self) -> Result<(), SimError> {
        self.recursion_depth += 1;
        if self.recursion_depth > 4096 {
            self.recursion_depth = 0;
            return Err(self.err("parser recursion depth exceeded (possible infinite recursion)"));
        }
        Ok(())
    }

    fn pop_depth(&mut self) {
        self.recursion_depth = self.recursion_depth.saturating_sub(1);
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

    fn peek_col(&self) -> usize {
        if self.pos >= self.tokens.len() {
            return 0;
        }
        self.tokens[self.pos].2
    }

    fn err(&self, msg: impl Into<String>) -> SimError {
        let msg_str = msg.into();
        let cumulative_line = self.peek_line();
        let col = self.peek_col();
        let (display_file, display_line) = self.resolve_source_file(cumulative_line);
        let source_line = if cumulative_line > 0 && cumulative_line <= self.source_lines.len() {
            Some(self.source_lines[cumulative_line - 1].clone())
        } else {
            None
        };

        let mut ctx = ErrorContext::new()
            .with_file(&display_file)
            .with_line(display_line)
            .with_col(col);

        if let Some(sl) = source_line {
            ctx = ctx.with_source(&sl);
        }

        let simple_err = SimError::parse(format!(
            "{}:{}:{}: {}",
            display_file, display_line, col, msg_str
        ));

        // If we have source lines available, format with rich context
        if !self.source_lines.is_empty() {
            SimError::parse(simple_err.format_with_context(&ctx))
        } else {
            simple_err
        }
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

    fn expect(&mut self, expected: Token) -> Result<(), SimError> {
        if self.peek() == &expected {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err(format!("expected {}, found {}", expected, self.peek())))
        }
    }

    fn skip_semi(&mut self) {
        if self.peek() == &Token::Semi {
            self.advance();
        }
    }

    fn expect_ident(&mut self) -> Result<Symbol, SimError> {
        let tok = self.peek().clone();
        match &tok {
            Token::Ident(s) => {
                self.advance();
                Ok(*s)
            }
            Token::New => {
                self.advance();
                Ok(Symbol::intern("new"))
            }
            Token::This => {
                self.advance();
                Ok(Symbol::intern("this"))
            }
            _ => Err(SimError::parse(format!(
                "line {}: expected identifier, found {}",
                self.peek_line(),
                self.peek()
            ))),
        }
    }

    pub fn parse_design(&mut self) -> Result<Design, SimError> {
        self.class_names.clear();
        self.class_names.push(Symbol::intern("process"));
        self.class_names.push(Symbol::intern("uvm_object"));
        self.class_names.push(Symbol::intern("uvm_component"));
        self.class_names.push(Symbol::intern("uvm_sequence_item"));
        self.class_names.push(Symbol::intern("uvm_sequence"));
        self.class_names.push(Symbol::intern("uvm_sequencer"));
        self.class_names.push(Symbol::intern("uvm_driver"));
        self.class_names.push(Symbol::intern("uvm_monitor"));
        self.class_names.push(Symbol::intern("uvm_scoreboard"));
        self.class_names.push(Symbol::intern("uvm_analysis_port"));
        self.class_names.push(Symbol::intern("uvm_analysis_imp"));
        self.class_names.push(Symbol::intern("uvm_test"));
        self.class_names.push(Symbol::intern("uvm_config_db"));
        self.class_names.push(Symbol::intern("uvm_report_object"));
        self.class_names.push(Symbol::intern("uvm_factory"));
        self.class_names.push(Symbol::intern("uvm_resource_db"));
        let mut modules = Vec::new();
        let mut classes = Vec::new();
        let mut packages = Vec::new();
        let mut interfaces = Vec::new();
        let mut unit_imports = Vec::new();
        let mut unit_funcs: Vec<FunctionDecl> = Vec::new();
        let mut unit_tasks: Vec<TaskDecl> = Vec::new();
        let mut unit_typedefs: Vec<TypedefDecl> = Vec::new();
        let mut unit_params: Vec<ParamDecl> = Vec::new();
        let mut binds = Vec::new();
        let mut clocking_blocks = Vec::new();
        let mut configs = Vec::new();
        let mut udp_defs = Vec::new();
        // First pass: collect all class names
        let saved_pos = self.pos;
        while self.peek() != &Token::Eof {
            if self.peek() == &Token::Class {
                let start = self.pos;
                self.advance(); // consume 'class'
                if self.peek() == &Token::Hash {
                    self.advance(); // consume #
                    self.expect(Token::LParen)?;
                    while self.peek() != &Token::RParen && self.peek() != &Token::Eof {
                        self.advance();
                    }
                    let _ = self.expect(Token::RParen);
                }
                if let Token::Ident(name) = self.peek() {
                    self.class_names.push(*name);
                }
                self.pos = start;
                let c = self.parse_class()?;
                classes.push(c);
            } else if self.peek() == &Token::Module {
                let m = self.parse_module()?;
                modules.push(m);
            } else if self.peek() == &Token::Interface {
                // skip interface in first pass (no class deps needed)
                self.parse_interface_fast()?;
            } else if self.peek() == &Token::Program {
                // skip program in first pass
                self.parse_program_fast()?;
            } else if self.peek() == &Token::Package {
                self.parse_package_decl()?;
            } else if self.peek() == &Token::Import {
                // Skip import statements in first pass
                self.advance();
                while self.peek() != &Token::Semi && self.peek() != &Token::Eof {
                    self.advance();
                }
                if self.peek() == &Token::Semi {
                    self.advance();
                }
            } else if self.peek() == &Token::LParen && self.peek_ahead(1) == &Token::Star {
                // Skip (* ... *) attributes
                self.skip_attribute();
            } else if self.peek() == &Token::Virtual && self.peek_ahead(1) == &Token::Class {
                // virtual class — collect class name
                let start = self.pos;
                self.advance(); // consume virtual
                self.advance(); // consume class
                if self.peek() == &Token::Hash {
                    self.advance();
                    if self.peek() == &Token::LParen {
                        let _ = self.skip_balanced_paren();
                    }
                }
                if let Token::Ident(name) = self.peek() {
                    self.class_names.push(*name);
                }
                self.pos = start;
                self.advance(); // consume virtual so parse_class() sees 'class'
                let c = self.parse_class()?;
                classes.push(c);
            } else if self.peek() == &Token::Covergroup {
                // Skip covergroup in first pass — collect name
                let cg = self.parse_covergroup()?;
                self.class_names.push(cg.name.clone());
            } else if self.peek() == &Token::Bind {
                // Skip bind in first pass
                self.advance(); // consume 'bind'
                while self.peek() != &Token::Semi && self.peek() != &Token::Eof {
                    self.advance();
                }
                if self.peek() == &Token::Semi {
                    self.advance();
                }
            } else if self.peek() == &Token::Clocking {
                // Skip clocking block in first pass
                self.advance(); // consume 'clocking'
                while self.peek() != &Token::EndClocking && self.peek() != &Token::Eof {
                    self.advance();
                }
                if self.peek() == &Token::EndClocking {
                    self.advance();
                }
            } else if self.peek() == &Token::Config {
                // Skip config in first pass
                self.advance(); // consume 'config'
                while self.peek() != &Token::EndConfig && self.peek() != &Token::Eof {
                    self.advance();
                }
                if self.peek() == &Token::EndConfig {
                    self.advance();
                }
            } else if self.peek() == &Token::Primitive {
                // Skip UDP in first pass
                self.advance(); // consume 'primitive'
                while self.peek() != &Token::EndPrimitive && self.peek() != &Token::Eof {
                    self.advance();
                }
                if self.peek() == &Token::EndPrimitive {
                    self.advance();
                }
            } else if self.peek() == &Token::Sequence {
                self.advance(); // consume 'sequence'
                while self.peek() != &Token::EndSequence && self.peek() != &Token::Eof {
                    self.advance();
                }
                if self.peek() == &Token::EndSequence {
                    self.advance();
                }
            } else if self.peek() == &Token::Function {
                self.advance(); // consume 'function'
                while self.peek() != &Token::EndFunction && self.peek() != &Token::Eof {
                    self.advance();
                }
                if self.peek() == &Token::EndFunction {
                    self.advance();
                    // Consume optional 'endfunction : name'
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) {
                            self.advance();
                        }
                    }
                }
            } else if self.peek() == &Token::Task {
                self.advance(); // consume 'task'
                while self.peek() != &Token::EndTask && self.peek() != &Token::Eof {
                    self.advance();
                }
                if self.peek() == &Token::EndTask {
                    self.advance();
                    // Consume optional 'endtask : name'
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) {
                            self.advance();
                        }
                    }
                }
            } else {
                // Gracefully skip unknown top-level constructs
                eprintln!(
                    "warning: skipping top-level construct at line {} in {}: {}",
                    self.peek_line(),
                    self.source_file,
                    self.peek()
                );
                // Try to advance past the unknown construct
                self.advance();
            }
        }
        self.pos = saved_pos;
        modules.clear();
        classes.clear();
        // Second pass: full parse with class names known
        while self.peek() != &Token::Eof {
            match self.peek() {
                Token::Module => {
                    let m = self.parse_module()?;
                    modules.push(m);
                }
                Token::Interface => {
                    let iface = self.parse_interface()?;
                    interfaces.push(iface);
                }
                Token::Class => {
                    let c = self.parse_class()?;
                    classes.push(c);
                }
                Token::Package => {
                    let p = self.parse_package_decl()?;
                    packages.push(p);
                }
                Token::Program => {
                    let m = self.parse_module()?;
                    modules.push(m);
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
                    self.skip_semi();
                    unit_imports.push((pkg, item));
                }
                Token::LParen if self.peek_ahead(1) == &Token::Star => {
                    self.skip_attribute();
                }
                Token::Virtual if self.peek_ahead(1) == &Token::Class => {
                    self.advance(); // consume 'virtual' so parse_class() sees 'class'
                    let c = self.parse_class()?;
                    classes.push(c);
                }
                Token::Covergroup => {
                    let cg = self.parse_covergroup()?;
                    // Store covergroup in first module as module item for elaboration
                    if let Some(m) = modules.first_mut() {
                        m.items.push(ModuleItem::Covergroup(cg));
                    }
                }
                Token::Bind => {
                    // bind target_module_name module_name #(...) inst_name (...);
                    self.advance(); // consume 'bind'
                    let target = self.expect_ident()?;
                    let instance = self.parse_instance()?;
                    binds.push(BindDecl { target, instance });
                }
                Token::Clocking => {
                    let cb = self.parse_clocking_block()?;
                    clocking_blocks.push(cb);
                }
                Token::Export => {
                    // export "DPI-C" function/task ...
                    self.advance();
                    if self.peek() == &Token::StringLit(Symbol::intern("DPI-C"))
                        || self.peek() == &Token::StringLit(Symbol::intern("DPI"))
                    {
                        self.parse_dpi_import()?;
                    } else {
                        // Not recognized — skip to semi
                        self.skip_until_semi_or_end()?;
                    }
                }
                Token::Config => {
                    let cfg = self.parse_config_decl()?;
                    configs.push(cfg);
                }
                Token::Primitive => {
                    let udp = self.parse_udp_declaration()?;
                    udp_defs.push(udp);
                }
                Token::Function => {
                    let func = self.parse_function(false)?;
                    unit_funcs.push(func);
                }
                Token::Task => {
                    let task = self.parse_task(false)?;
                    unit_tasks.push(task);
                }
                Token::Typedef => {
                    let td = self.parse_typedef()?;
                    // Store typedef as a declaration
                    unit_typedefs.push(td);
                }
                Token::Parameter | Token::LocalParam => {
                    let is_local = self.peek() == &Token::LocalParam;
                    self.advance();
                    let mut params = Vec::new();
                    self.parse_param_list(&mut params)?;
                    for p in params {
                        if !is_local {
                            unit_params.push(p);
                        }
                    }
                }
                _ => {
                    if matches!(
                        self.peek(),
                        Token::Wire
                            | Token::Wand
                            | Token::Wor
                            | Token::Tri
                            | Token::TriAnd
                            | Token::TriOr
                            | Token::Tri0
                            | Token::Tri1
                            | Token::Supply0
                            | Token::Supply1
                            | Token::Reg
                            | Token::Logic
                            | Token::Int
                            | Token::Integer
                            | Token::Bit
                            | Token::Byte
                            | Token::Shortint
                            | Token::Longint
                            | Token::Time
                            | Token::Real
                            | Token::RealTime
                            | Token::String
                            | Token::Enum
                            | Token::Struct
                            | Token::Union
                    ) {
                        return Err(SimError::parse(format!(
                            "line {}: declaration outside of module",
                            self.peek_line()
                        )));
                    }
                    let line = self.peek_line();
                    let tok = self.peek().clone();
                    // Gracefully skip unknown constructs at top level
                    self.advance();
                    eprintln!(
                        "warning: skipping top-level construct at line {} in {}: {}",
                        line,
                        self.source_file,
                        tok
                    );
                }
            }
        }
        Ok(Design {
            modules,
            classes,
            packages,
            interfaces,
            binds,
            clocking_blocks,
            configs,
            udp_defs,
            top_module: None,
            unit_imports,
            unit_decls: Vec::new(),
            unit_funcs,
            unit_tasks,
            unit_typedefs,
            unit_params,
            timescale: None,
        })
    }

    fn parse_module_item(&mut self) -> Result<Option<ModuleItem>, SimError> {
        // Guard: if the token is `=`, skip to semi/end to avoid infinite loop
        if matches!(
            self.peek(),
            Token::BlockingAssign | Token::NonBlockingAssign
        ) {
            self.skip_until_semi_or_end()?;
            return Ok(None);
        }
        // Skip (* ... *) attribute annotations before module items
        if self.peek() == &Token::LParen && self.peek_ahead(1) == &Token::Star {
            self.skip_attribute();
            return self.parse_module_item();
        }
        match self.peek() {
            Token::Always | Token::AlwaysComb | Token::AlwaysFF | Token::AlwaysLatch => {
                let always = self.parse_always()?;
                Ok(Some(ModuleItem::Always(always)))
            }
            Token::Initial => {
                let initial = self.parse_initial()?;
                Ok(Some(ModuleItem::Initial(initial)))
            }
            Token::Final => {
                let final_block = self.parse_final()?;
                Ok(Some(ModuleItem::Final(final_block)))
            }
            Token::Assign => {
                let assign = self.parse_assign()?;
                Ok(Some(ModuleItem::Assign(assign)))
            }
            Token::Const => {
                self.advance(); // consume 'const'
                let mut decl = self.parse_decl()?;
                for n in &mut decl.names {
                    n.is_const = true;
                }
                Ok(Some(ModuleItem::Decl(decl)))
            }
            Token::Var => {
                self.advance();
                // var followed by type or identifier
                if matches!(
                    self.peek(),
                    Token::Wire
                        | Token::Reg
                        | Token::Logic
                        | Token::Int
                        | Token::Integer
                        | Token::Bit
                        | Token::Byte
                        | Token::Shortint
                        | Token::Longint
                        | Token::Time
                        | Token::String
                        | Token::Real
                        | Token::RealTime
                        | Token::Enum
                        | Token::Struct
                        | Token::Union
                ) {
                    Ok(Some(ModuleItem::Decl(self.parse_decl()?)))
                } else if let Token::Ident(_) = self.peek() {
                    // Implicit var with type inference (treated as logic)
                    let vname = self.expect_ident()?;
                    let names = vec![DeclVar {
                        name: vname,
                        range: None,
                        expr_range: None,
                        array_range: None,
                        extra_packed_dims: vec![],
                        is_dynamic: false,
                        is_queue: false,
                        is_associative: false,
                        assoc_key_type: None,
                        is_rand: false,
                        is_const: false,
                        expr: None,
                    }];
                    self.skip_semi();
                    Ok(Some(ModuleItem::Decl(Decl {
                        dtype: DataType::Logic,
                        kind: DeclKind::Logic,
                        names,
                    })))
                } else {
                    Ok(None)
                }
            }
            Token::Wire
            | Token::Reg
            | Token::Logic
            | Token::Int
            | Token::Integer
            | Token::Bit
            | Token::Byte
            | Token::Shortint
            | Token::Longint
            | Token::Time
            | Token::String
            | Token::Real
            | Token::RealTime
            | Token::Mailbox
            | Token::Semaphore
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
                let decl = self.parse_decl()?;
                Ok(Some(ModuleItem::Decl(decl)))
            }
            Token::Ident(name) => {
                if self.class_names.contains(name) || self.typedef_names.contains(name) {                            let dtype = DataType::UserDefined(*name);
                    self.advance();
                    // Handle parameterized class: Class #(type) varname — skip type args, use base class name
                    if self.peek() == &Token::Hash {
                        self.advance();
                        self.expect(Token::LParen)?;
                        while self.peek() != &Token::RParen && self.peek() != &Token::Eof {
                            self.advance();
                        }
                        let _ = self.expect(Token::RParen);
                    }
                    let mut names = Vec::new();
                    loop {
                        if let Token::Ident(n) = self.peek() {
                            let vname = n.clone();
                            self.advance();
                            names.push(DeclVar {
                                name: vname,
                                range: None,
                                expr_range: None,
                                array_range: None,
                                extra_packed_dims: vec![],
                                is_dynamic: false,
                                is_queue: false,
                                is_associative: false,
                                assoc_key_type: None,
                                is_rand: false,
                                is_const: false,
                                expr: None,
                            });
                        } else {
                            if self.peek() == &Token::BlockingAssign {
                                // = new() or = expr — skip and continue to skip_semi
                                self.skip_semi();
                                return Ok(Some(ModuleItem::Decl(Decl {
                                    dtype,
                                    kind: DeclKind::Logic,
                                    names,
                                })));
                            }
                            break;
                        }
                        if self.peek() == &Token::Comma {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.skip_semi();
                    Ok(Some(ModuleItem::Decl(Decl {
                        dtype,
                        kind: DeclKind::Logic,
                        names,
                    })))
                } else if matches!(self.peek_ahead(1), Token::Ident(_))
                    || self.peek_ahead(1) == &Token::Hash
                    || self.peek_ahead(1) == &Token::LParen
                    || self.peek_ahead(1) == &Token::LBrack
                {
                    // Check if Ident + [range] is a declaration (type [msb:lsb] name) or instance
                    if self.peek_ahead(1) == &Token::LBrack {
                        let decl = self.parse_decl();
                        match decl {
                            Ok(decl) => return Ok(Some(ModuleItem::Decl(decl))),
                            Err(_) => {}
                        }
                    }
                    let instance = self.parse_instance()?;
                    Ok(Some(ModuleItem::Instance(instance)))
                } else if self.peek_ahead(1) == &Token::Colon {
                    self.skip_until_semi_or_end()?;
                    Ok(None)
                } else if self.peek_ahead(1) == &Token::BlockingAssign
                    || self.peek_ahead(1) == &Token::NonBlockingAssign
                    || self.peek_ahead(1) == &Token::Semi
                {
                    // Treat as implicit wire/reg declaration: `name;` or `name <= expr;` or `name = expr;`
                    let vname = self.expect_ident()?;
                    let expr = if self.peek() == &Token::BlockingAssign {
                        self.advance();
                        self.parse_expr(0).ok()
                    } else if self.peek() == &Token::NonBlockingAssign {
                        self.advance();
                        self.parse_expr(0).ok()
                    } else {
                        None
                    };
                    self.skip_semi();
                    let names = vec![DeclVar {
                        name: vname,
                        range: None,
                        expr_range: None,
                        array_range: None,
                        extra_packed_dims: vec![],
                        is_dynamic: false,
                        is_queue: false,
                        is_associative: false,
                        assoc_key_type: None,
                        is_rand: false,
                        is_const: false,
                        expr,
                    }];
                    Ok(Some(ModuleItem::Decl(Decl {
                        dtype: DataType::Logic,
                        kind: DeclKind::Wire,
                        names,
                    })))
                } else {
                    // Not recognized — skip silently
                    let line = self.peek_line();
                    let tok = self.peek().clone();
                    eprintln!(
                        "warning: skipping unknown construct at line {} in {}: {}",
                        line,
                        self.source_file,
                        tok
                    );
                    self.skip_until_semi_or_end()?;
                    Ok(None)
                }
            }
            Token::For | Token::If | Token::Case | Token::CaseX | Token::CaseZ => {
                let gen_item = self.parse_generate_item()?;
                Ok(Some(ModuleItem::Generate(GenerateBlock {
                    items: vec![gen_item],
                })))
            }
            Token::Generate => {
                let gen = self.parse_generate_block()?;
                Ok(Some(ModuleItem::Generate(gen)))
            }
            Token::GenVar => {
                self.skip_until_semi_or_end()?;
                Ok(None)
            }
            Token::Param | Token::Parameter | Token::LocalParam => {
                let is_localparam = self.peek() == &Token::LocalParam;
                self.advance(); // consume param/localparam/parameter
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
                    _ => {}
                }
                if self.peek() == &Token::Signed {
                    self.advance();
                    if dtype.is_none() {
                        dtype = Some(DataType::Signed(Box::new(DataType::Int)));
                    }
                }
                let mut range = None;
                if self.peek() == &Token::LBrack {
                    self.advance();
                    let msb = self.parse_expr(0)?;
                    self.expect(Token::Colon)?;
                    let lsb = self.parse_expr(0)?;
                    self.expect(Token::RBrack)?;
                    range = Some((msb, lsb));
                }
                let mut params = Vec::new();
                loop {
                    let pk = self.peek().clone();
                    let name = match &pk {
                        Token::Ident(s) => {
                            self.advance();
                            s.clone()
                        }
                        _ => break,
                    };
                    let default = if self.peek() == &Token::BlockingAssign {
                        self.advance();
                        Some(self.parse_expr(0)?)
                    } else {
                        None
                    };
            params.push(ParamDecl {
                name: name,
                        dtype: dtype.clone(),
                        range: range.clone(),
                        default,
                        is_localparam,
                        is_type_param: false,
                        type_default: None,
                    });
                    if self.peek() == &Token::Comma {
                        self.advance();
                    } else {
                        break;
                    }
                }
                self.skip_semi();
                if params.is_empty() {
                    Ok(None)
                } else if params.len() == 1 {
                    Ok(Some(ModuleItem::Param(params.into_iter().next().unwrap())))
                } else {
                    Ok(Some(ModuleItem::Generate(GenerateBlock {
                        items: vec![GenerateItem::Items(
                            params.into_iter().map(|p| ModuleItem::Param(p)).collect(),
                        )],
                    })))
                }
            }
            Token::Virtual => {
                // virtual <iface_type>[.<modport>] <vif_name>;
                self.advance(); // consume 'virtual'
                let iface_type = self.expect_ident()?;
                let mut modport = None;
                if self.peek() == &Token::Dot {
                    self.advance();
                    modport = Some(self.expect_ident()?);
                }
                let vif_name = self.expect_ident()?;
                self.skip_semi();
                Ok(Some(ModuleItem::VirtualInterface {
                    iface_type,
                    modport,
                    vif_name,
                }))
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
                    is_static: task.is_static,
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
                // Check for 'typedef class' (forward declaration)
                if matches!(self.peek_ahead(1), Token::Class | Token::Virtual) {
                    self.advance(); // consume 'typedef'
                    while self.peek() != &Token::Semi && self.peek() != &Token::Eof {
                        self.advance();
                    }
                    self.skip_semi();
                    return Ok(None);
                }
                let td = self.parse_typedef()?;
                self.typedef_names.push(td.name.clone());
                Ok(Some(ModuleItem::Typedef(td)))
            }
            Token::Import => {
                self.advance();
                // Check for DPI-C import or export
                if self.peek() == &Token::StringLit(Symbol::intern("DPI-C"))
                    || self.peek() == &Token::StringLit(Symbol::intern("DPI"))
                {
                    let result = self.parse_dpi_import()?;
                    return Ok(Some(ModuleItem::DpiImport(result)));
                }
                let pkg = self.expect_ident()?;
                self.expect(Token::Scope)?;
                let item = if self.peek() == &Token::Star {
                    self.advance();
                    Symbol::intern("*")
                } else {
                    self.expect_ident()?
                };
                // Register imported typedef names so subsequent declarations can use them
                if let Some(tdefs) = self.package_tdefs.get(&pkg) {
                    if item == "*" {
                        for name in tdefs {
                            if !self.typedef_names.contains(name) {
                                self.typedef_names.push(name.clone());
                            }
                        }
                    } else if tdefs.contains(&item) && !self.typedef_names.contains(&item) {
                        self.typedef_names.push(item.clone());
                    }
                }
                self.skip_semi();
                Ok(Some(ModuleItem::Import { package: pkg, item }))
            }
            Token::Covergroup => {
                let cg = self.parse_covergroup()?;
                Ok(Some(ModuleItem::Covergroup(cg)))
            }
            Token::Clocking => {
                let cb = self.parse_clocking_block()?;
                Ok(Some(ModuleItem::Clocking(cb)))
            }
            Token::Specify => {
                let sb = self.parse_specify_block()?;
                Ok(Some(ModuleItem::Specify(sb)))
            }
            Token::Export => {
                // export "DPI-C" function/task
                self.advance();
                if self.peek() == &Token::StringLit(Symbol::intern("DPI-C"))
                    || self.peek() == &Token::StringLit(Symbol::intern("DPI"))
                {
                    let result = self.parse_dpi_import()?;
                    Ok(Some(ModuleItem::DpiImport(result)))
                } else {
                    // Not a DPI export — skip to semicolon
                    self.skip_until_semi_or_end()?;
                    Ok(None)
                }
            }
            Token::Assert | Token::Assume | Token::Cover | Token::Expect => {
                self.skip_until_semi_or_end()?;
                Ok(None)
            }
            Token::Void | Token::Auto | Token::Static => {
                self.skip_until_semi_or_end()?;
                Ok(None)
            }
            Token::Class | Token::EndClass => {
                // Skip class/endclass tokens — don't use skip_until_semi_or_end
                // because class bodies contain semicolons; just advance one token
                // and let the module loop handle the rest
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn skip_until_semi_or_end(&mut self) -> Result<(), SimError> {
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
                Token::Begin => {
                    depth += 1;
                    self.advance();
                }
                Token::End => {
                    depth = depth.saturating_sub(1);
                    self.advance();
                }
                _ => {
                    self.advance();
                }
            }
        }
    }

    fn skip_to_stmt_boundary(&mut self) {
        loop {
            match self.peek() {
                Token::Semi => {
                    self.advance();
                    return;
                }
                Token::End
                | Token::Endcase
                | Token::EndFunction
                | Token::EndTask
                | Token::Endmodule
                | Token::Eof => {
                    return;
                }
                _ => {
                    self.advance();
                }
            }
        }
    }

    fn skip_attribute(&mut self) {
        // Called when peek = `(*` — caller hasn't advanced past `(` yet.
        // Advance past `(`, then track depth for nested `(*...*)`.
        let mut depth = 1u32;
        self.advance(); // consume the initial `(`
        loop {
            match self.peek() {
                Token::Eof => return,
                _ => {
                    if self.peek() == &Token::Star && self.peek_ahead(1) == &Token::RParen {
                        self.advance(); // `*`
                        self.advance(); // `)`
                        depth -= 1;
                        if depth == 0 {
                            return;
                        }
                    } else if self.peek() == &Token::LParen && self.peek_ahead(1) == &Token::Star {
                        depth += 1;
                        self.advance(); // `(`
                    } else {
                        self.advance();
                    }
                }
            }
        }
    }

}
