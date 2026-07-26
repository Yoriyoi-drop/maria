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

use crate::parser::util::*;
use crate::ast::types::const_eval_simple;
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

    fn parse_module(&mut self) -> Result<Module, SimError> {
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
                return Err(SimError::parse(format!(
                    "line {}: expected module name",
                    self.peek_line()
                )))
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

        loop {
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
                            eprintln!("warning: skipping module item: {}", e);
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
            name: name,
            ports,
            params,
            decls,
            items,
        })
    }

    fn parse_interface_fast(&mut self) -> Result<(), SimError> {
        self.advance(); // consume 'interface'
        match self.peek() {
            Token::Ident(_) => {
                self.advance();
            }
            _ => return Err(SimError::parse("expected interface name")),
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

    fn parse_program_fast(&mut self) -> Result<(), SimError> {
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

    fn skip_balanced_paren(&mut self) -> Result<(), SimError> {
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
                Token::Eof => return Err(SimError::parse("unexpected EOF in balanced paren")),
                _ => {
                    self.advance();
                }
            }
        }
        Ok(())
    }

    fn parse_interface(&mut self) -> Result<Interface, SimError> {
        self.advance(); // consume 'interface'
        self.typedef_names.clear();

        let name = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => {
                return Err(SimError::parse(format!(
                    "line {}: expected interface name",
                    self.peek_line()
                )))
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
                return Err(SimError::parse(format!(
                    "line {}: expected endinterface",
                    self.peek_line()
                )));
            }
        }
        if self.peek() == &Token::Colon {
            self.advance();
            if matches!(self.peek(), Token::Ident(_)) {
                self.advance();
            }
        }

        Ok(Interface {
            name: name,
            params,
            decls,
            modports,
        })
    }

    fn parse_modport(&mut self) -> Result<Modport, SimError> {
        self.advance(); // consume 'modport'
        let name = match self.peek() {
            Token::Ident(s) => {
                let n = s.clone();
                self.advance();
                n
            }
            _ => {
                return Err(SimError::parse(format!(
                    "line {}: expected modport name",
                    self.peek_line()
                )))
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
                    return Err(SimError::parse(format!(
                        "line {}: expected direction in modport",
                        self.peek_line()
                    )))
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
                        return Err(SimError::parse(format!(
                            "line {}: expected signal name in modport",
                            self.peek_line()
                        )))
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
        Ok(Modport { name: name, items })
    }

    fn parse_param_list(&mut self, params: &mut Vec<ParamDecl>) -> Result<(), SimError> {
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
                        type_ident = Some(s.clone());
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

            let mut dtype = None;
            if self.peek() == &Token::Signed {
                self.advance();
                dtype = Some(DataType::Signed(Box::new(DataType::Int)));
            }

            // Skip unpacked array dimension after name: name [N]
            if self.peek() == &Token::LBrack && self.peek_ahead(1) != &Token::Colon {
                self.advance(); // [
                let _ = self.parse_expr(0);
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

    fn parse_type_expr(&mut self) -> Result<DataType, SimError> {
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
            _ => return Err(SimError::parse(format!("expected type"))),
        };
        self.advance();
        if self.peek() == &Token::Signed {
            self.advance();
            Ok(DataType::Signed(Box::new(dt)))
        } else {
            Ok(dt)
        }
    }

    fn parse_port_list(&mut self, ports: &mut Vec<Port>) -> Result<(), SimError> {
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
                            return Err(SimError::parse(format!(
                                "line {}: expected port name",
                                self.peek_line()
                            )))
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

    fn parse_range(&mut self) -> Result<Option<ExprRange>, SimError> {
        self.expect(Token::LBrack)?;
        let msb = self.parse_expr(0)?;
        self.expect(Token::Colon)?;
        let lsb = self.parse_expr(0)?;
        self.expect(Token::RBrack)?;
        Ok(Some(ExprRange { msb, lsb }))
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

    fn parse_scoped_type_name(&mut self) -> Option<DataType> {
        // Check if the next tokens are Ident(::Ident)? — a user-defined type name
        // that should be treated as the type of a declaration (e.g., wire pkg::type varname)
        if let Token::Ident(s) = self.peek() {
            let s = s.clone();
            let ahead = self.peek_ahead(1).clone();
            if ahead == Token::Scope {
                let pkg = s;
                self.advance(); // consume package name
                self.advance(); // consume ::
                if let Token::Ident(t) = self.peek() {
                    let type_name = t.clone();
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

    fn parse_decl(&mut self) -> Result<Decl, SimError> {
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
                while self.peek() == &Token::LBrack && self.peek_ahead(1) == &Token::Colon {
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
                while self.peek() == &Token::LBrack && self.peek_ahead(1) == &Token::Colon {
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
                while self.peek() == &Token::LBrack && self.peek_ahead(1) == &Token::Colon {
                    if let Some(er) = self.parse_range()? {
                        extra_packed.push((er, None));
                    }
                }
                let names = self.parse_decl_names(decl_expr_range, extra_packed)?;
                self.skip_semi();
                return Ok(Decl { dtype, kind: DeclKind::Logic, names });
            }
            _ => return Err(SimError::parse(format!("line {}: expected wire/reg/logic/int/byte/shortint/longint/enum/struct/union/wand/wor/tri", self.peek_line()))),
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
        while self.peek() == &Token::LBrack && self.peek_ahead(1) == &Token::Colon {
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

    fn parse_decl_names(
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
                    let (var_expr_range, array_range) = if decl_expr_range.is_some() {
                        let ar = if self.peek() == &Token::LBrack {
                            if self.peek_ahead(1) == &Token::RBrack {
                                self.advance();
                                self.advance();
                                is_dynamic = true;
                                None
                            } else if self.peek_ahead(1) == &Token::Dollar
                                && self.peek_ahead(2) == &Token::RBrack
                            {
                                self.advance();
                                self.advance();
                                self.advance();
                                is_queue = true;
                                None
                            } else if self.peek_ahead(1) == &Token::Int {
                                // int-key associative array
                                self.advance(); // [
                                self.advance(); // int
                                self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::Int);
                                None
                            } else if self.peek_ahead(1) == &Token::String {
                                // string-key associative array
                                self.advance(); // [
                                self.advance(); // string
                                self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::String);
                                None
                            } else if self.peek_ahead(1) == &Token::Bit {
                                // bit-key associative array
                                self.advance();
                                self.advance();
                                self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::Bit);
                                None
                            } else if self.peek_ahead(1) == &Token::Logic {
                                // logic-key associative array
                                self.advance();
                                self.advance();
                                self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::Logic);
                                None
                            } else if self.peek_ahead(1) == &Token::Byte {
                                // byte-key associative array
                                self.advance();
                                self.advance();
                                self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::Byte);
                                None
                            } else if self.peek_ahead(1) == &Token::Shortint {
                                // shortint-key associative array
                                self.advance();
                                self.advance();
                                self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::Shortint);
                                None
                            } else if self.peek_ahead(1) == &Token::Longint {
                                // longint-key associative array
                                self.advance();
                                self.advance();
                                self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::Longint);
                                None
                            } else if self.peek_ahead(1) == &Token::Star
                                && self.peek_ahead(2) == &Token::RBrack
                            {
                                // wildcard [*] associative array
                                self.advance();
                                self.advance();
                                self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::Int);
                                None
                            } else if self.peek_ahead(1) == &Token::Colon
                                || self.peek_ahead(2) == &Token::Colon
                            {
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
                                self.advance(); // [
                                self.parse_expr(0)?;
                                self.expect(Token::RBrack)?;
                                None
                            }
                        } else {
                            None
                        };
                        (decl_expr_range.clone(), ar)
                    } else {
                        if self.peek() == &Token::LBrack {
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
                            } else if self.peek_ahead(1) != &Token::Colon {
                                self.advance(); // [
                                self.parse_expr(0)?;
                                self.expect(Token::RBrack)?;
                                (None, None)
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
                                (ver, ar)
                            }
                        } else {
                            (None, None)
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

    fn parse_enum_members(&mut self) -> Result<Vec<(Symbol, Option<Expr>)>, SimError> {
        self.expect(Token::LBrace)?;
        let mut members = Vec::new();
        loop {
            match self.peek() {
                Token::Ident(name) => {
                    let name = name.clone();
                    self.advance();
                    let val = if matches!(self.peek(), Token::Eq | Token::BlockingAssign) {
                        self.advance();
                        Some(self.parse_expr(0)?)
                    } else {
                        None
                    };
                    members.push((name, val));
                }
                _ => {
                    return Err(SimError::parse(format!(
                        "line {}: expected identifier in enum",
                        self.peek_line()
                    )))
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

    fn parse_struct_body(&mut self) -> Result<Vec<StructMember>, SimError> {
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
                    let name = name.clone();
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
                    return Err(SimError::parse(format!(
                        "line {}: expected type in struct/union member",
                        self.peek_line()
                    )))
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
            let name = self.expect_ident()?;
            self.skip_semi();
            members.push(StructMember {
                name,
                dtype: Box::new(member_type),
                range,
            });
        }
    }

    fn parse_typedef(&mut self) -> Result<TypedefDecl, SimError> {
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
                    _ => None,
                };
                if base.is_some() && self.peek() == &Token::LBrack {
                    self.parse_range()?;
                }
                let members = self.parse_enum_members()?;
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, DataType::EnumType { base, members }, None)
                } else {
                    return Err(SimError::parse(format!(
                        "line {}: expected name after typedef enum",
                        self.peek_line()
                    )));
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
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(SimError::parse(format!(
                        "line {}: expected name after typedef bit",
                        self.peek_line()
                    )));
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
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(SimError::parse(format!(
                        "line {}: expected name after typedef byte",
                        self.peek_line()
                    )));
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
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(SimError::parse(format!(
                        "line {}: expected name after typedef shortint",
                        self.peek_line()
                    )));
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
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(SimError::parse(format!(
                        "line {}: expected name after typedef longint",
                        self.peek_line()
                    )));
                }
            }
            Token::Time => {
                self.advance();
                let range = if self.peek() == &Token::LBrack {
                    self.parse_range()?
                } else {
                    None
                };
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, DataType::Time, range)
                } else {
                    return Err(SimError::parse(format!(
                        "line {}: expected name after typedef time",
                        self.peek_line()
                    )));
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
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(SimError::parse(format!(
                        "line {}: expected name after typedef int",
                        self.peek_line()
                    )));
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
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(SimError::parse(format!(
                        "line {}: expected name after typedef integer",
                        self.peek_line()
                    )));
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
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(SimError::parse(format!(
                        "line {}: expected name after typedef logic",
                        self.peek_line()
                    )));
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
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(SimError::parse(format!(
                        "line {}: expected name after typedef reg",
                        self.peek_line()
                    )));
                }
            }
            Token::Struct => {
                self.advance();
                if matches!(self.peek(), Token::Ident(s) if s == "packed") {
                    self.advance();
                }
                let members = self.parse_struct_body()?;
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, DataType::StructType { members }, None)
                } else {
                    return Err(SimError::parse(format!(
                        "line {}: expected name after typedef struct",
                        self.peek_line()
                    )));
                }
            }
            Token::Union => {
                self.advance();
                if matches!(self.peek(), Token::Ident(s) if s == "packed") {
                    self.advance();
                }
                let members = self.parse_struct_body()?;
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, DataType::UnionType { members }, None)
                } else {
                    return Err(SimError::parse(format!(
                        "line {}: expected name after typedef union",
                        self.peek_line()
                    )));
                }
            }
            _ => {
                return Err(SimError::parse(format!(
                    "line {}: expected type after typedef",
                    self.peek_line()
                )))
            }
        };
        self.skip_semi();
        Ok(TypedefDecl { name: name, dtype, range })
    }

    fn parse_generate_block(&mut self) -> Result<GenerateBlock, SimError> {
        self.advance(); // consume 'generate'
        let mut items = Vec::new();
        loop {
            match self.peek() {
                Token::EndGenerate => {
                    self.advance();
                    return Ok(GenerateBlock { items });
                }
                Token::Eof => {
                    return Err(SimError::parse("line {}: unexpected EOF in generate block"));
                }
                _ => {
                    let item = self.parse_generate_item()?;
                    items.push(item);
                }
            }
        }
    }

    fn parse_generate_item(&mut self) -> Result<GenerateItem, SimError> {
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
                        return Err(SimError::parse(format!(
                            "line {}: expected genvar name",
                            self.peek_line()
                        )))
                    }
                };
                // Parse init: i = <expr>
                let _init = if self.peek() != &Token::Semi {
                    self.expect(Token::BlockingAssign)?;
                    let init_expr = self.parse_expr(0)?;
                    self.expect(Token::Semi)?;
                    Some(Stmt::BlockingAssign {
                        lhs: Expr::Ident(var),
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
                    var: var,
                    init: None,
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
                    None => self.parse_generate_item(),
                }
            }
        }
    }

    fn parse_generate_block_body(&mut self) -> Result<Vec<ModuleItem>, SimError> {
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

    fn parse_function(&mut self, virtual_flag: bool) -> Result<FunctionDecl, SimError> {
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
            Token::Ident(_) if matches!(self.peek_ahead(1), Token::Ident(_) | Token::LBrack) => {
                let tp_name = self.expect_ident()?;
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
                return Err(SimError::parse(format!(
                    "line {}: expected function name",
                    self.peek_line()
                )))
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
                    return Err(SimError::parse(format!(
                        "line {}: expected method name after ::",
                        self.peek_line()
                    )));
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
                } else if let Token::Ident(name) = self.peek() {                        if self.type_param_names.contains(name) {
                        self.advance();
                    } else if matches!(self.peek_ahead(1), Token::Ident(_) | Token::LBrack) {
                        // User-defined type name followed by port name or range
                        self.advance();
                    }
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
                Token::Auto | Token::Static => {
                    // automatic/static variable declaration in function body
                    self.advance();
                    // Try to parse as declaration
                    if let Ok(decl) = self.parse_decl() {
                        decls.push(decl);
                    } else {
                        return Err(SimError::parse(format!(
                            "line {}: expected declaration after automatic/static",
                            self.peek_line()
                        )));
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
                return Err(SimError::parse(format!(
                    "line {}: expected endfunction",
                    self.peek_line()
                )));
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

    fn parse_task(&mut self, virtual_flag: bool) -> Result<TaskDecl, SimError> {
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
                    return Err(SimError::parse(format!(
                        "line {}: expected method name after ::",
                        self.peek_line()
                    )));
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
                return Err(SimError::parse(format!(
                    "line {}: expected endtask",
                    self.peek_line()
                )));
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

    fn parse_always(&mut self) -> Result<AlwaysBlock, SimError> {
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

    fn parse_initial(&mut self) -> Result<InitialBlock, SimError> {
        self.advance();
        let stmts = self.parse_stmt_block()?;
        Ok(InitialBlock { stmts })
    }

    fn parse_final(&mut self) -> Result<InitialBlock, SimError> {
        self.advance();
        let stmts = self.parse_stmt_block()?;
        Ok(InitialBlock { stmts })
    }

    fn parse_sensitivity_events(&mut self) -> Result<Vec<SensitivityEvent>, SimError> {
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

    fn parse_sensitivity_list(&mut self) -> Result<SensitivityList, SimError> {
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

    fn parse_assign(&mut self) -> Result<ContinuousAssign, SimError> {
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

    fn parse_delay(&mut self) -> Result<Delay, SimError> {
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

    fn parse_instance(&mut self) -> Result<ModuleInstance, SimError> {
        let name_tok = self.peek().clone();
        let module_name = match &name_tok {
            Token::Ident(s) => {
                self.advance();
                                *s
            }
            _ => {
                return Err(SimError::parse(format!(
                    "line {}: expected module name",
                    self.peek_line()
                )))
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
                                return Err(SimError::parse(format!(
                                    "line {}: expected parameter name",
                                    self.peek_line()
                                )))
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
                return Err(SimError::parse(format!(
                    "line {}: expected instance name",
                    self.peek_line()
                )))
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
                                return Err(SimError::parse(format!(
                                    "line {}: expected port name",
                                    self.peek_line()
                                )))
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

    fn parse_gate_primitive(&mut self) -> Result<GatePrimitive, SimError> {
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
                return Err(SimError::parse(format!(
                    "line {}: expected gate type",
                    self.peek_line()
                )))
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
                    return Err(SimError::parse(format!(
                        "line {}: expected gate instance name",
                        self.peek_line()
                    )))
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

    fn parse_stmt_block(&mut self) -> Result<Vec<Stmt>, SimError> {
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
            let stmts = match self.parse_stmt() {
                Ok(s) => vec![s],
                Err(e) => {
                    eprintln!("warning: skipping statement: {}", e);
                    self.skip_to_stmt_boundary();
                    vec![]
                }
            };
            Ok(stmts)
        }
    }

    fn parse_immediate_assertion(&mut self) -> Result<Stmt, SimError> {
        let kind = match self.peek() {
            Token::Assert => {
                self.advance();
                "assert"
            }
            Token::Assume => {
                self.advance();
                "assume"
            }
            Token::Cover => {
                self.advance();
                "cover"
            }
            Token::Expect => {
                self.advance();
                "expect"
            }
            _ => return Err(SimError::parse("expected assert/assume/cover/expect")),
        };

        // Check for concurrent assertion: assert property (...)
        if self.peek() == &Token::Property {
            self.advance();
            self.expect(Token::LParen)?;
            // Parse optional clocking: @(posedge clk)
            let clock_event = if self.peek() == &Token::At {
                self.advance();
                self.expect(Token::LParen)?;
                let ce = if self.peek() == &Token::PosEdge {
                    self.advance();
                    let sig = self.expect_ident()?;
                    Some(ClockEvent::Posedge(sig))
                } else if self.peek() == &Token::NegEdge {
                    self.advance();
                    let sig = self.expect_ident()?;
                    Some(ClockEvent::Negedge(sig))
                } else {
                    let sig = self.expect_ident()?;
                    Some(ClockEvent::Edge(sig))
                };
                self.expect(Token::RParen)?;
                ce
            } else {
                None
            };
            // Parse optional disable iff (expr)
            let disable_iff = if self.peek() == &Token::Disable {
                self.advance();
                match self.peek() {
                    Token::Ident(s) if s == "iff" => {
                        self.advance();
                    }
                    _ => return Err(SimError::parse("expected 'iff' after 'disable'")),
                }
                self.expect(Token::LParen)?;
                let expr = self.parse_expr(0)?;
                self.expect(Token::RParen)?;
                Some(Box::new(expr))
            } else {
                None
            };
            let expr = self.parse_expr(0)?;
            self.expect(Token::RParen)?;
            let fail_stmt = if self.peek() == &Token::Else {
                self.advance();
                Some(Box::new(self.parse_stmt()?))
            } else {
                None
            };
            self.skip_semi();
            let cond = Expr::TernaryOp {
                cond: Box::new(expr),
                true_expr: Box::new(Expr::Value(Value::Decimal(1))),
                false_expr: Box::new(Expr::Value(Value::Decimal(0))),
            };
            return match kind {
                "assert" => Ok(Stmt::Assert {
                    cond,
                    pass_stmt: None,
                    fail_stmt,
                    clock_event,
                    disable_iff,
                }),
                "assume" => Ok(Stmt::Assume {
                    cond,
                    pass_stmt: None,
                    fail_stmt,
                    clock_event,
                    disable_iff,
                }),
                "cover" => Ok(Stmt::Cover {
                    cond,
                    pass_stmt: None,
                    clock_event,
                    disable_iff,
                }),
                _ => unreachable!(),
            };
        }

        // Immediate assertion: assert (expr) [pass_stmt] [else fail_stmt]
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
            "assert" => Ok(Stmt::Assert {
                cond,
                pass_stmt,
                fail_stmt,
                clock_event: None,
                disable_iff: None,
            }),
            "assume" => Ok(Stmt::Assume {
                cond,
                pass_stmt,
                fail_stmt,
                clock_event: None,
                disable_iff: None,
            }),
            "cover" => Ok(Stmt::Cover {
                cond,
                pass_stmt,
                clock_event: None,
                disable_iff: None,
            }),
            "expect" => Ok(Stmt::Expect {
                cond,
                pass_stmt,
                fail_stmt,
            }),
            _ => unreachable!(),
        }
    }

    fn parse_clocking_event(&mut self) -> Result<Expr, SimError> {
        self.expect(Token::At)?;
        self.expect(Token::LParen)?;
        if self.peek() == &Token::PosEdge || self.peek() == &Token::NegEdge {
            self.advance();
        }
        let signal = self.parse_expr(0)?;
        self.expect(Token::RParen)?;
        Ok(signal)
    }

    fn parse_wait_order(&mut self) -> Result<Stmt, SimError> {
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
        let fail_stmt = if self.peek() == &Token::Else {
            self.advance(); // consume 'else'
            Some(Box::new(self.parse_stmt()?))
        } else {
            None
        };
        self.skip_semi();
        Ok(Stmt::WaitOrder { events, fail_stmt })
    }

    fn parse_covergroup(&mut self) -> Result<CovergroupDecl, SimError> {
        self.advance(); // consume 'covergroup'
        let name = self.expect_ident()?;
        let clocking_event = if self.peek() == &Token::At {
            Some(self.parse_clocking_event()?)
        } else {
            None
        };
        // Handle optional 'with function sample(...)' for covergroups
        if let Token::Ident(s) = self.peek() {
            if *s == Symbol::intern("with") {
                self.advance(); // consume 'with'
                // Skip 'function sample(type param, ...)' until ';'
                let mut depth = 0;
                loop {
                    match self.peek() {
                        Token::Semi if depth == 0 => {
                            self.advance();
                            break;
                        }
                        Token::LParen => {
                            depth += 1;
                            self.advance();
                        }
                        Token::RParen if depth > 0 => {
                            depth -= 1;
                            self.advance();
                        }
                        Token::Eof => break,
                        _ => {
                            self.advance();
                        }
                    }
                }
            } else {
                self.skip_semi();
            }
        } else {
            self.skip_semi();
        }
        let mut coverpoints = Vec::new();
        let mut crosses = Vec::new();
        loop {
            match self.peek() {
                Token::EndGroup | Token::Eof => {
                    self.advance();
                    // Handle optional 'endgroup : name'
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) {
                            self.advance();
                        }
                    }
                    break;
                }
                Token::Ident(_) => {
                    let ident = self.expect_ident()?;
                    if self.peek() == &Token::Colon {
                        self.advance(); // consume :
                        match self.peek() {
                            Token::Coverpoint => {
                                self.advance(); // consume coverpoint
                                let expr = self.parse_expr(0)?;
                                let mut bins = Vec::new();
                                if self.peek() == &Token::LBrace {
                                    self.advance();
                                    loop {
                                        match self.peek() {
                                            Token::RBrace => {
                                                self.advance();
                                                break;
                                            }
                                            Token::Bins
                                            | Token::IllegalBins
                                            | Token::IgnoreBins => {
                                                let bin_type = match self.peek() {
                                                    Token::IllegalBins => BinType::Illegal,
                                                    Token::IgnoreBins => BinType::Ignore,
                                                    _ => BinType::Normal,
                                                };
                                                self.advance();
                                                let bin_name = self.expect_ident()?;
                                                self.expect(Token::BlockingAssign)?;
                                                self.expect(Token::LBrace)?;
                                                let mut range_list = Vec::new();
                                                loop {
                                                    if self.peek() == &Token::LBrack {
                                                        self.advance();
                                                        let low = self.parse_expr(0)?;
                                                        self.expect(Token::Colon)?;
                                                        let high = self.parse_expr(0)?;
                                                        self.expect(Token::RBrack)?;
                                                        range_list.push(low);
                                                        range_list.push(high);
                                                    } else {
                                                        range_list.push(self.parse_expr(0)?);
                                                    }
                                                    if self.peek() == &Token::Comma {
                                                        let ahead = self.peek_ahead(1).clone();
                                                        // Check if next token starts a new port declaration (direction or scoped type)
                                                        let is_new_port = ahead == Token::Input
                                                            || ahead == Token::Output
                                                            || ahead == Token::Inout
                                                            || (matches!(&ahead, Token::Ident(_))
                                                                && matches!(
                                                                    self.peek_ahead(2),
                                                                    Token::Scope
                                                                ));
                                                        if !is_new_port {
                                                            self.advance();
                                                        } else {
                                                            break;
                                                        }
                                                    } else {
                                                        break;
                                                    }
                                                }
                                                self.expect(Token::RBrace)?;
                                                self.skip_semi();
                                                bins.push(BinDef {
                                                    name: bin_name,
                                                    range_list,
                                                    bin_type,
                                                });
                                            }
                                            _ => break,
                                        }
                                    }
                                }
                                self.skip_semi();
                                coverpoints.push(CoverpointDef {
                                    name: ident,
                                    expr,
                                    bins,
                                });
                            }
                            Token::Cross => {
                                self.advance(); // consume cross
                                let mut cps = Vec::new();
                                loop {
                                    cps.push(self.expect_ident()?);
                                    if self.peek() == &Token::Comma {
                                        self.advance();
                                    } else {
                                        break;
                                    }
                                }
                                self.skip_semi();
                                crosses.push(CrossDef {
                                    name: ident,
                                    coverpoints: cps,
                                });
                            }
                            _ => {
                                return Err(SimError::parse(format!(
                                    "line {}: unexpected token after ':' in covergroup body",
                                    self.peek_line()
                                )));
                            }
                        }
                    } else {
                        return Err(SimError::parse(format!(
                            "line {}: unexpected token after identifier in covergroup body",
                            self.peek_line()
                        )));
                    }
                }
                Token::Option_ => {
                    self.advance();
                    self.skip_until_semi_or_end()?;
                }
                _ => {
                    return Err(SimError::parse(format!(
                        "line {}: unexpected token in covergroup body: {}",
                        self.peek_line(),
                        self.peek()
                    )));
                }
            }
        }
        Ok(CovergroupDecl {
            name,
            clocking_event,
            coverpoints,
            crosses,
        })
    }

    fn parse_dpi_import(&mut self) -> Result<DpiImport, SimError> {
        // Check if this is a DPI-C export instead of import
        let _saved = self.pos;
        // We already consumed the string "DPI-C" or "DPI"
        // Now check for 'context' and then 'function' or 'task'
        // For export: import "DPI-C" context function ...  vs export "DPI-C" function ...

        self.advance(); // consume "DPI-C" string literal
        let is_task = if self.peek() == &Token::Task {
            self.advance();
            true
        } else if self.peek() == &Token::Function {
            self.advance();
            false
        } else {
            return Err(SimError::parse(format!(
                "line {}: expected 'function' or 'task' after import \"DPI-C\"",
                self.peek_line()
            )));
        };
        if matches!(self.peek(), Token::Auto | Token::Static) {
            self.advance();
        }
        let return_type = if is_task {
            None
        } else if self.peek() == &Token::Void {
            self.advance();
            None
        } else if let Some(dt) = self.try_parse_dpi_type() {
            self.skip_dpi_range();
            Some(Box::new(dt))
        } else {
            None
        };
        let name = self.expect_ident()?;
        self.expect(Token::LParen)?;
        let mut args = Vec::new();
        if self.peek() != &Token::RParen {
            loop {
                let direction = if self.peek() == &Token::Input {
                    self.advance();
                    PortDirection::Input
                } else if self.peek() == &Token::Output {
                    self.advance();
                    PortDirection::Output
                } else if self.peek() == &Token::Inout {
                    self.advance();
                    PortDirection::Inout
                } else {
                    PortDirection::Input // default direction per SV spec
                };
                let dtype = self.try_parse_dpi_type().unwrap_or(DataType::Logic);
                self.skip_dpi_range();
                let arg_name = self.expect_ident()?;
                args.push(DpiArg {
                    direction,
                    dtype,
                    name: arg_name,
                });
                if self.peek() == &Token::Comma {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        self.expect(Token::RParen)?;
        self.skip_semi();
        Ok(DpiImport {
            name,
            return_type,
            args,
            is_task,
        })
    }

    fn skip_dpi_range(&mut self) {
        if self.peek() == &Token::LBrack {
            self.advance();
            let mut depth = 1;
            while depth > 0 && self.peek() != &Token::Eof {
                match self.peek() {
                    Token::LBrack => depth += 1,
                    Token::RBrack => {
                        depth -= 1;
                        if depth == 0 {
                            self.advance();
                            break;
                        }
                    }
                    _ => {}
                }
                self.advance();
            }
        }
    }

    fn try_parse_dpi_type(&mut self) -> Option<DataType> {
        let dt = match self.peek() {
            Token::Byte => {
                self.advance();
                DataType::Byte
            }
            Token::Shortint => {
                self.advance();
                DataType::Shortint
            }
            Token::Int => {
                self.advance();
                DataType::Int
            }
            Token::Longint => {
                self.advance();
                DataType::Longint
            }
            Token::Integer => {
                self.advance();
                DataType::Integer
            }
            Token::Real => {
                self.advance();
                DataType::Real
            }
            Token::RealTime => {
                self.advance();
                DataType::Realtime
            }
            Token::Bit => {
                self.advance();
                DataType::Bit
            }
            Token::Logic => {
                self.advance();
                DataType::Logic
            }
            Token::String => {
                self.advance();
                DataType::String
            }
            Token::Ident(s) if s == "chandle" => {
                self.advance();
                DataType::Longint
            }
            _ => return None,
        };
        Some(dt)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, SimError> {
        // Skip (* ... *) attribute annotations
        if self.peek() == &Token::LParen && self.peek_ahead(1) == &Token::Star {
            self.skip_attribute();
            return self.parse_stmt();
        }
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
                            Stmt::Case {
                                expr,
                                items,
                                default,
                            } => {
                                if qualifier == Token::Unique {
                                    Ok(Stmt::UniqueCase {
                                        expr,
                                        items,
                                        default,
                                    })
                                } else {
                                    Ok(Stmt::PriorityCase {
                                        expr,
                                        items,
                                        default,
                                    })
                                }
                            }
                            Stmt::CaseX {
                                expr,
                                items,
                                default,
                            } => {
                                if qualifier == Token::Unique {
                                    Ok(Stmt::UniqueCase {
                                        expr,
                                        items,
                                        default,
                                    })
                                } else {
                                    Ok(Stmt::PriorityCase {
                                        expr,
                                        items,
                                        default,
                                    })
                                }
                            }
                            Stmt::CaseZ {
                                expr,
                                items,
                                default,
                            } => {
                                if qualifier == Token::Unique {
                                    Ok(Stmt::UniqueCase {
                                        expr,
                                        items,
                                        default,
                                    })
                                } else {
                                    Ok(Stmt::PriorityCase {
                                        expr,
                                        items,
                                        default,
                                    })
                                }
                            }
                            _ => Ok(stmt),
                        }
                    }
                    Token::If => {
                        let stmt = self.parse_if_stmt()?;
                        match stmt {
                            Stmt::IfElse {
                                cond,
                                true_branch,
                                false_branch,
                            } => {
                                if qualifier == Token::Unique {
                                    Ok(Stmt::UniqueIf {
                                        cond,
                                        true_branch,
                                        false_branch,
                                    })
                                } else {
                                    Ok(Stmt::PriorityIf {
                                        cond,
                                        true_branch,
                                        false_branch,
                                    })
                                }
                            }
                            _ => Ok(stmt),
                        }
                    }
                    _ => Err(SimError::parse(format!(
                        "line {}: expected case or if after unique/priority/unique0",
                        self.peek_line()
                    ))),
                }
            }
            Token::Begin => {
                self.advance();
                let mut block_name = String::new();
                if self.peek() == &Token::Colon {
                    self.advance();
                    if let Token::Ident(name) = self.peek() {
                        block_name = name.as_str().to_string();
                        self.advance();
                    }
                }
                let mut stmts = Vec::new();
                loop {
                    if self.peek() == &Token::End || self.peek() == &Token::Eof {
                        self.advance();
                        break;
                    }
                    match self.parse_stmt() {
                        Ok(s) => stmts.push(s),
                        Err(e) => {
                            eprintln!("warning: skipping statement: {}", e);
                            self.skip_to_stmt_boundary();
                        }
                    }
                }
                if block_name.is_empty() {
                    Ok(Stmt::Block { stmts })
                } else {
                    Ok(Stmt::NamedBlock {
                        name: Symbol::intern(&block_name),
                        stmts,
                        decls: vec![],
                    })
                }
            }
            Token::If => self.parse_if_stmt(),
            Token::Case | Token::CaseX | Token::CaseZ => self.parse_case_stmt(),
            Token::For => self.parse_for_stmt(),
            Token::Foreach => self.parse_foreach_stmt(),
            Token::While => self.parse_while_stmt(),
            Token::Forever => self.parse_forever_stmt(),
            Token::Repeat => self.parse_repeat_stmt(),
            Token::Fork => self.parse_fork_join(),
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
                    Token::Ident(s) => {
                        self.advance();
                        *s
                    }
                    _ => {
                        return Err(SimError::parse(format!(
                            "line {}: expected identifier after disable",
                            self.peek_line()
                        )))
                    }
                };
                self.skip_semi();
                Ok(Stmt::Disable { name: name })
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
                    Ok(Stmt::Wait {
                        cond,
                        stmt: Some(Box::new(stmt)),
                    })
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
                Ok(Stmt::Delay {
                    delay,
                    stmt: Box::new(stmt),
                })
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
                    Ok(Stmt::EventControl {
                        events,
                        stmt: Some(Box::new(stmt)),
                    })
                }
            }
            Token::Arrow => {
                self.advance();
                let tok = self.peek().clone();
                let name = match tok {
                    Token::Ident(s) => {
                        self.advance();
                        s
                    }
                    _ => {
                        return Err(SimError::parse(format!(
                            "line {}: expected event name after ->",
                            self.peek_line()
                        )))
                    }
                };
                self.skip_semi();
                Ok(Stmt::EventTrigger { name: name })
            }
            Token::Ident(ref s) if s == "randcase" => {
                self.advance();
                let mut items = Vec::new();
                loop {
                    if self.peek() == &Token::Endcase || self.peek() == &Token::Eof {
                        if self.peek() == &Token::Endcase {
                            self.advance();
                        }
                        break;
                    }
                    let weight = self.parse_expr(0)?;
                    self.expect(Token::Colon)?;
                    let stmt = self.parse_stmt()?;
                    let w = const_eval_simple(&weight).unwrap_or(1) as u64;
                    items.push(RandCaseItem {
                        weight: w,
                        stmt: Box::new(stmt),
                    });
                }
                Ok(Stmt::RandCase { items })
            }
            Token::Ident(ref s) if s == "randsequence" => {
                self.advance();
                let mut productions = Vec::new();
                loop {
                    let is_endseq = matches!(self.peek(), Token::Ident(s) if s == "endsequence");
                    if is_endseq || self.peek() == &Token::Eof {
                        if matches!(self.peek(), Token::Ident(s) if s == "endsequence") {
                            self.advance();
                        }
                        break;
                    }
                    let prod_name = self.expect_ident()?;
                    self.expect(Token::Colon)?;
                    let mut items = Vec::new();
                    loop {
                        let stmt = self.parse_stmt()?;
                        let weight = if self.peek() == &Token::BlockingAssign {
                            self.advance();
                            let w_expr = self.parse_expr(0)?;
                            Some(const_eval_simple(&w_expr).unwrap_or(1) as u64)
                        } else {
                            None
                        };
                        items.push(RandSeqItem {
                            value: Box::new(stmt),
                            weight,
                        });
                        if self.peek() == &Token::Pipe {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.skip_semi();
                    productions.push(RandSeqProduction {
                        name: prod_name,
                        items,
                    });
                }
                Ok(Stmt::RandSequence { productions })
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
                                self.expect(Token::RParen)?;
                                lhs = Expr::MethodCall {
                                    obj: Box::new(lhs),
                                    method: member,
                                    args,
                                    with_clause: None,
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
                    Token::Increment => {
                        self.advance();
                        self.skip_semi();
                        let rhs = Expr::BinaryOp {
                            op: BinaryOp::Add,
                            lhs: Box::new(lhs.clone()),
                            rhs: Box::new(Expr::Value(Value::Decimal(1))),
                        };
                        Ok(Stmt::BlockingAssign {
                            lhs,
                            rhs,
                            delay: None,
                        })
                    }
                    Token::Decrement => {
                        self.advance();
                        self.skip_semi();
                        let rhs = Expr::BinaryOp {
                            op: BinaryOp::Sub,
                            lhs: Box::new(lhs.clone()),
                            rhs: Box::new(Expr::Value(Value::Decimal(1))),
                        };
                        Ok(Stmt::BlockingAssign {
                            lhs,
                            rhs,
                            delay: None,
                        })
                    }
                    Token::BlockingAssign => {
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        self.skip_semi();
                        Ok(Stmt::BlockingAssign {
                            lhs,
                            rhs,
                            delay: None,
                        })
                    }
                    Token::NonBlockingAssign => {
                        if is_valid_lvalue(&lhs) {
                            self.advance();
                            let rhs = self.parse_expr(0)?;
                            self.skip_semi();
                            Ok(Stmt::NonBlockingAssign {
                                lhs,
                                rhs,
                                delay: None,
                            })
                        } else {
                            self.advance();
                            let rhs = self.parse_expr(8)?;
                            self.skip_semi();
                            Ok(Stmt::Expr {
                                expr: Expr::BinaryOp {
                                    op: BinaryOp::Le,
                                    lhs: Box::new(lhs),
                                    rhs: Box::new(rhs),
                                },
                            })
                        }
                    }
                    Token::PlusAssign => {
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        self.skip_semi();
                        let lhs_copy = lhs.clone();
                        Ok(Stmt::BlockingAssign {
                            lhs,
                            rhs: Expr::BinaryOp {
                                op: BinaryOp::Add,
                                lhs: Box::new(lhs_copy),
                                rhs: Box::new(rhs),
                            },
                            delay: None,
                        })
                    }
                    Token::MinusAssign => {
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        self.skip_semi();
                        let lhs_copy = lhs.clone();
                        Ok(Stmt::BlockingAssign {
                            lhs,
                            rhs: Expr::BinaryOp {
                                op: BinaryOp::Sub,
                                lhs: Box::new(lhs_copy),
                                rhs: Box::new(rhs),
                            },
                            delay: None,
                        })
                    }
                    Token::XorAssign => {
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        self.skip_semi();
                        let lhs_copy = lhs.clone();
                        Ok(Stmt::BlockingAssign {
                            lhs,
                            rhs: Expr::BinaryOp {
                                op: BinaryOp::BitXor,
                                lhs: Box::new(lhs_copy),
                                rhs: Box::new(rhs),
                            },
                            delay: None,
                        })
                    }
                    _ => {
                        self.skip_semi();
                        Ok(Stmt::Expr { expr: lhs })
                    }
                }
            }
        }
    }

    fn parse_if_stmt(&mut self) -> Result<Stmt, SimError> {
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

    fn parse_case_stmt(&mut self) -> Result<Stmt, SimError> {
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
            Ok(Stmt::CaseInside {
                expr,
                items,
                default,
            })
        } else if is_casex {
            Ok(Stmt::CaseX {
                expr,
                items,
                default,
            })
        } else if is_casez {
            Ok(Stmt::CaseZ {
                expr,
                items,
                default,
            })
        } else {
            Ok(Stmt::Case {
                expr,
                items,
                default,
            })
        }
    }

    fn parse_for_stmt(&mut self) -> Result<Stmt, SimError> {
        self.advance();
        self.expect(Token::LParen)?;
        let init = if self.peek() != &Token::Semi {
            // Handle variable declaration in for loop init: for (int k = 0; ...)
            if matches!(
                self.peek(),
                Token::Int | Token::Integer | Token::Bit | Token::Logic | Token::Reg
            ) {
                self.advance(); // skip type keyword
                if self.peek() == &Token::Signed {
                    self.advance();
                }
                if self.peek() == &Token::Unsigned {
                    self.advance();
                }
                let var = self.expect_ident()?;
                let init_val = if self.peek() == &Token::BlockingAssign {
                    self.advance();
                    Some(self.parse_expr(0)?)
                } else {
                    None
                };
                let stmt = if let Some(val) = init_val {
                    Stmt::BlockingAssign {
                        lhs: Expr::Ident(var),
                        rhs: val,
                        delay: None,
                    }
                } else {
                    Stmt::Null
                };
                Some(Box::new(stmt))
            } else {
                let expr = self.parse_expr(0)?;
                let init_stmt = match self.peek() {
                    Token::BlockingAssign => {
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        Stmt::BlockingAssign {
                            lhs: expr,
                            rhs,
                            delay: None,
                        }
                    }
                    _ => Stmt::Null,
                };
                Some(Box::new(init_stmt))
            }
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
            // Handle postfix increment/decrement: k++ or k--
            if self.peek() == &Token::Increment {
                self.advance();
                if let Expr::Ident(var) = expr {
                    let step_stmt = Stmt::BlockingAssign {
                        lhs: Expr::Ident(var),
                        rhs: Expr::BinaryOp {
                            op: BinaryOp::Add,
                            lhs: Box::new(Expr::Ident(var)),
                            rhs: Box::new(Expr::Value(Value::Decimal(1))),
                        },
                        delay: None,
                    };
                    Some(Box::new(step_stmt))
                } else {
                    None
                }
            } else if self.peek() == &Token::Decrement {
                self.advance();
                if let Expr::Ident(var) = expr {
                    let step_stmt = Stmt::BlockingAssign {
                        lhs: Expr::Ident(var),
                        rhs: Expr::BinaryOp {
                            op: BinaryOp::Sub,
                            lhs: Box::new(Expr::Ident(var)),
                            rhs: Box::new(Expr::Value(Value::Decimal(1))),
                        },
                        delay: None,
                    };
                    Some(Box::new(step_stmt))
                } else {
                    None
                }
            } else {
                let step_stmt = match self.peek() {
                    Token::BlockingAssign => {
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        Stmt::BlockingAssign {
                            lhs: expr,
                            rhs,
                            delay: None,
                        }
                    }
                    _ => Stmt::Null,
                };
                Some(Box::new(step_stmt))
            }
        } else {
            None
        };
        self.expect(Token::RParen)?;
        let stmts = self.parse_stmt_block()?;
        Ok(Stmt::LoopFor {
            init,
            cond,
            step,
            stmts,
        })
    }

    fn parse_foreach_stmt(&mut self) -> Result<Stmt, SimError> {
        self.advance();
        self.expect(Token::LParen)?;
        let array_var = self.expect_ident()?;
        self.expect(Token::LBrack)?;
        let mut index_vars = Vec::new();
        loop {
            index_vars.push(self.expect_ident()?);
            if self.peek() == &Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        self.expect(Token::RBrack)?;
        self.expect(Token::RParen)?;
        let stmts = self.parse_stmt_block()?;
        Ok(Stmt::ForeachLoop {
            array_var,
            index_vars,
            stmts,
        })
    }

    fn parse_while_stmt(&mut self) -> Result<Stmt, SimError> {
        self.advance();
        self.expect(Token::LParen)?;
        let cond = self.parse_expr(0)?;
        self.expect(Token::RParen)?;
        let stmts = self.parse_stmt_block()?;
        Ok(Stmt::LoopWhile { cond, stmts })
    }

    fn parse_forever_stmt(&mut self) -> Result<Stmt, SimError> {
        self.advance();
        let stmts = self.parse_stmt_block()?;
        Ok(Stmt::LoopForever { stmts })
    }

    fn parse_repeat_stmt(&mut self) -> Result<Stmt, SimError> {
        self.advance();
        self.expect(Token::LParen)?;
        let count = self.parse_expr(0)?;
        self.expect(Token::RParen)?;
        let stmts = self.parse_stmt_block()?;
        Ok(Stmt::Repeat { count, stmts })
    }

    fn parse_fork_join(&mut self) -> Result<Stmt, SimError> {
        self.advance(); // consume 'fork'
        let mut processes = Vec::new();
        loop {
            match self.peek() {
                Token::Join => {
                    self.advance();
                    return Ok(Stmt::Fork {
                        processes,
                        join_type: JoinType::Join,
                    });
                }
                Token::JoinAny => {
                    self.advance();
                    return Ok(Stmt::Fork {
                        processes,
                        join_type: JoinType::JoinAny,
                    });
                }
                Token::JoinNone => {
                    self.advance();
                    return Ok(Stmt::Fork {
                        processes,
                        join_type: JoinType::JoinNone,
                    });
                }
                Token::Eof => {
                    return Err(SimError::parse(format!(
                        "line {}: unexpected EOF in fork block",
                        self.peek_line()
                    )))
                }
                _ => {
                    let stmt = self.parse_stmt()?;
                    processes.push(stmt);
                }
            }
        }
    }

    fn parse_syscall(&mut self) -> Result<Stmt, SimError> {
        self.advance();
        let name_tok = self.peek().clone();
        let name = match &name_tok {
            Token::Ident(s) => {
                self.advance();
                *s
            }
            _ => {
                return Err(SimError::parse(format!(
                    "line {}: expected system call name after $",
                    self.peek_line()
                )))
            }
        };

        match name.as_str() {
            "finish" | "stop" => {
                if self.peek() == &Token::LParen {
                    self.advance();
                    // Optional argument (e.g. $finish(0), $finish(1))
                    if self.peek() != &Token::RParen {
                        self.parse_expr(0)?;
                    }
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
                            || matches!(
                                self.peek(),
                                Token::Input | Token::Output | Token::Inout | Token::Dot
                            )
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

}
