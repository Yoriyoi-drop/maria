use crate::ast::*;
use crate::ast::types::const_eval_simple;
use crate::parser::lexer::*;
use super::util::*;
use crate::error::SimError;

pub struct Parser {
    tokens: Vec<(Token, usize, usize)>,
    pos: usize,
    source_file: String,
    class_names: Vec<String>,
    typedef_names: Vec<String>,
    package_tdefs: std::collections::HashMap<String, Vec<String>>,
    type_param_names: Vec<String>,
}

impl Parser {
    pub fn new(tokens: Vec<(Token, usize, usize)>, source_file: &str) -> Self {
        Self { tokens, pos: 0, source_file: source_file.to_string(), class_names: vec!["process".to_string(), "uvm_object".to_string(), "uvm_component".to_string(), "uvm_sequence_item".to_string(), "uvm_sequence".to_string(), "uvm_sequencer".to_string(), "uvm_driver".to_string(), "uvm_monitor".to_string(), "uvm_scoreboard".to_string(), "uvm_analysis_port".to_string(), "uvm_analysis_imp".to_string(), "uvm_test".to_string(), "uvm_config_db".to_string(), "uvm_report_object".to_string(), "uvm_factory".to_string(), "uvm_resource_db".to_string()], typedef_names: Vec::new(), package_tdefs: std::collections::HashMap::new(), type_param_names: Vec::new() }
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
        SimError::parse(format!("{}:{}:{}: {}", self.source_file, self.peek_line(), self.peek_col(), msg.into()))
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

    fn expect_ident(&mut self) -> Result<String, SimError> {
        let tok = self.peek().clone();
        match &tok {
            Token::Ident(s) => { self.advance(); Ok(s.clone()) }
            Token::New => { self.advance(); Ok("new".to_string()) }
            Token::This => { self.advance(); Ok("this".to_string()) }
            _ => Err(SimError::parse(format!("line {}: expected identifier, found {}", self.peek_line(), self.peek()))),
        }
    }

    pub fn parse_design(&mut self) -> Result<Design, SimError> {
        self.class_names.clear();
        self.class_names.push("process".to_string());
        self.class_names.push("uvm_object".to_string());
        self.class_names.push("uvm_component".to_string());
        self.class_names.push("uvm_sequence_item".to_string());
        self.class_names.push("uvm_sequence".to_string());
        self.class_names.push("uvm_sequencer".to_string());
        self.class_names.push("uvm_driver".to_string());
        self.class_names.push("uvm_monitor".to_string());
        self.class_names.push("uvm_scoreboard".to_string());
        self.class_names.push("uvm_analysis_port".to_string());
        self.class_names.push("uvm_analysis_imp".to_string());
        self.class_names.push("uvm_test".to_string());
        self.class_names.push("uvm_config_db".to_string());
        self.class_names.push("uvm_report_object".to_string());
        self.class_names.push("uvm_factory".to_string());
        self.class_names.push("uvm_resource_db".to_string());
        let mut modules = Vec::new();
        let mut classes = Vec::new();
        let mut packages = Vec::new();
        let mut interfaces = Vec::new();
        let mut unit_imports = Vec::new();
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
                    self.class_names.push(name.clone());
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
                if self.peek() == &Token::Semi { self.advance(); }
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
                    if self.peek() == &Token::LParen { let _ = self.skip_balanced_paren(); }
                }
                if let Token::Ident(name) = self.peek() {
                    self.class_names.push(name.clone());
                }
                self.pos = start;
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
                if self.peek() == &Token::Semi { self.advance(); }
            } else if self.peek() == &Token::Clocking {
                // Skip clocking block in first pass
                self.advance(); // consume 'clocking'
                while self.peek() != &Token::EndClocking && self.peek() != &Token::Eof {
                    self.advance();
                }
                if self.peek() == &Token::EndClocking { self.advance(); }
            } else if self.peek() == &Token::Config {
                // Skip config in first pass
                self.advance(); // consume 'config'
                while self.peek() != &Token::EndConfig && self.peek() != &Token::Eof {
                    self.advance();
                }
                if self.peek() == &Token::EndConfig { self.advance(); }
            } else if self.peek() == &Token::Primitive {
                // Skip UDP in first pass
                self.advance(); // consume 'primitive'
                while self.peek() != &Token::EndPrimitive && self.peek() != &Token::Eof {
                    self.advance();
                }
                if self.peek() == &Token::EndPrimitive { self.advance(); }
            } else if self.peek() == &Token::Sequence {
                self.advance(); // consume 'sequence'
                while self.peek() != &Token::EndSequence && self.peek() != &Token::Eof {
                    self.advance();
                }
                if self.peek() == &Token::EndSequence { self.advance(); }
            } else {
                // Gracefully skip unknown top-level constructs
                eprintln!("warning: skipping top-level construct at line {}: {}", self.peek_line(), self.peek());
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
                        "*".to_string()
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
                Token::Config => {
                    let cfg = self.parse_config_decl()?;
                    configs.push(cfg);
                }
                Token::Primitive => {
                    let udp = self.parse_udp_declaration()?;
                    udp_defs.push(udp);
                }
                _ => {
                    if matches!(self.peek(), Token::Wire | Token::Wand | Token::Wor |
                        Token::Tri | Token::TriAnd | Token::TriOr | Token::Tri0 | Token::Tri1 |
                        Token::Supply0 | Token::Supply1 | Token::Reg | Token::Logic |
                        Token::Int | Token::Integer | Token::Bit | Token::Byte |
                        Token::Shortint | Token::Longint | Token::Time |
                        Token::Real | Token::RealTime | Token::String |
                        Token::Enum | Token::Struct | Token::Union) {
                        return Err(SimError::parse(format!("line {}: declaration outside of module", self.peek_line())));
                    }
                    let line = self.peek_line();
                    // Gracefully skip unknown constructs at top level
                    self.advance();
                    eprintln!("warning: skipping top-level construct at line {}: {}", line, self.peek());
                }
            }
        }
        Ok(Design { modules, classes, packages, interfaces, binds, clocking_blocks, configs, udp_defs, top_module: None, unit_imports, timescale: None })
    }

    fn parse_clocking_block(&mut self) -> Result<ClockingBlock, SimError> {
        self.advance(); // consume 'clocking'
        let name = self.expect_ident()?;

        // Parse clock event: @(posedge clk) or @(negedge clk) or @(clk)
        self.expect(Token::At)?;
        self.expect(Token::LParen)?;
        let clock_event = if self.peek() == &Token::PosEdge {
            self.advance();
            let sig = self.expect_ident()?;
            ClockEvent::Posedge(sig)
        } else if self.peek() == &Token::NegEdge {
            self.advance();
            let sig = self.expect_ident()?;
            ClockEvent::Negedge(sig)
        } else {
            let sig = self.expect_ident()?;
            ClockEvent::Edge(sig)
        };
        self.expect(Token::RParen)?;
        self.skip_semi();

        let mut default_input_skew = None;
        let mut default_output_skew = None;
        let mut items = Vec::new();

        loop {
            match self.peek() {
                Token::EndClocking => {
                    self.advance();
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) {
                            self.advance();
                        }
                    }
                    break;
                }
                Token::Default => {
                    // default input/output #skew;
                    self.advance();
                    if self.peek() == &Token::Input {
                        self.advance();
                        if self.peek() == &Token::Hash {
                            self.advance();
                            if let Token::Number { value, .. } = self.peek().clone() {
                                self.advance();
                                default_input_skew = value.parse::<u64>().ok();
                            }
                        }
                        self.skip_semi();
                    } else if self.peek() == &Token::Output {
                        self.advance();
                        if self.peek() == &Token::Hash {
                            self.advance();
                            if let Token::Number { value, .. } = self.peek().clone() {
                                self.advance();
                                default_output_skew = value.parse::<u64>().ok();
                            }
                        }
                        self.skip_semi();
                    } else {
                        self.skip_semi();
                    }
                }
                Token::Input => {
                    self.advance();
                    let mut signals = Vec::new();
                    loop {
                        if self.peek() == &Token::Semi || self.peek() == &Token::Eof {
                            break;
                        }
                        if self.peek() == &Token::Hash {
                            // skew override
                            self.advance();
                            if let Token::Number { value, .. } = self.peek().clone() {
                                self.advance();
                                let _skew = value.parse::<u64>().ok();
                            }
                        }
                        if let Token::Ident(s) = self.peek().clone() {
                            self.advance();
                            signals.push(s);
                        } else {
                            break;
                        }
                        if self.peek() == &Token::Comma {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.skip_semi();
                    items.push(ClockingItem::Input { signals, skew: None });
                }
                Token::Output => {
                    self.advance();
                    let mut signals = Vec::new();
                    loop {
                        if self.peek() == &Token::Semi || self.peek() == &Token::Eof {
                            break;
                        }
                        if self.peek() == &Token::Hash {
                            self.advance();
                            if let Token::Number { value, .. } = self.peek().clone() {
                                self.advance();
                                let _skew = value.parse::<u64>().ok();
                            }
                        }
                        if let Token::Ident(s) = self.peek().clone() {
                            self.advance();
                            signals.push(s);
                        } else {
                            break;
                        }
                        if self.peek() == &Token::Comma {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.skip_semi();
                    items.push(ClockingItem::Output { signals, skew: None });
                }
                Token::Inout => {
                    self.advance();
                    let mut signals = Vec::new();
                    loop {
                        if self.peek() == &Token::Semi || self.peek() == &Token::Eof {
                            break;
                        }
                        if let Token::Ident(s) = self.peek().clone() {
                            self.advance();
                            signals.push(s);
                        } else {
                            break;
                        }
                        if self.peek() == &Token::Comma {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.skip_semi();
                    items.push(ClockingItem::InputOutput { signals });
                }
                _ => {
                    self.advance();
                }
            }
        }

        Ok(ClockingBlock {
            name,
            clock_event,
            default_input_skew,
            default_output_skew,
            items,
        })
    }

    fn parse_specify_item(&mut self) -> Result<Option<SpecifyItem>, SimError> {
        // Check for $setup, $hold, $setuphold system function calls
        if self.peek() == &Token::Dollar {
            // Read the system function name
            let saved = self.pos;
            self.advance(); // consume $
            if let Token::Ident(fname) = self.peek().clone() {
                self.advance();
                match fname.as_str() {
                    "$setup" | "$hold" | "$setuphold" => {
                        let is_setup = fname == "$setup";
                        let _is_hold = fname == "$hold";
                        let is_setuphold = fname == "$setuphold";
                        self.expect(Token::LParen)?;
                        let data = self.parse_expr(0)?;
                        self.expect(Token::Comma)?;
                        let ref_event = self.parse_expr(0)?;
                        let (setup_limit, hold_limit) = if is_setuphold {
                            self.expect(Token::Comma)?;
                            let sl = self.parse_expr(0)?;
                            self.expect(Token::Comma)?;
                            let hl = self.parse_expr(0)?;
                            (Some(sl), Some(hl))
                        } else {
                            self.expect(Token::Comma)?;
                            let limit = self.parse_expr(0)?;
                            if is_setup { (Some(limit), None) } else { (None, Some(limit)) }
                        };
                        self.expect(Token::RParen)?;
                        if self.peek() == &Token::Semi {
                            self.advance(); // consume optional ;
                        }
                        return if is_setuphold {
                            Ok(Some(SpecifyItem::SetupHoldCheck {
                                ref_event,
                                data,
                                setup_limit: setup_limit.unwrap(),
                                hold_limit: hold_limit.unwrap(),
                            }))
                        } else if is_setup {
                            Ok(Some(SpecifyItem::SetupCheck { data, ref_event, limit: setup_limit.unwrap() }))
                        } else {
                            Ok(Some(SpecifyItem::HoldCheck { ref_event, data, limit: hold_limit.unwrap() }))
                        };
                    }
                    _ => {}
                }
            }
            // Not recognized, reset position
            self.pos = saved;
        }

        // specparam name = value;
        if self.peek() == &Token::SpecParam {
            self.advance();
            let name = self.expect_ident()?;
            self.expect(Token::BlockingAssign)?;
            let value = self.parse_expr(0)?;
            self.skip_semi();
            return Ok(Some(SpecifyItem::SpecParam { name, value }));
        }

        // Simple path delay: (src => dst) = (rise, fall);
        if self.peek() == &Token::LParen {
            let saved = self.pos;
            self.advance();
            if let Token::Ident(src) = self.peek().clone() {
                self.advance();
                if self.peek() == &Token::Arrow {
                    self.advance();
                    if let Token::Ident(dst) = self.peek().clone() {
                        self.advance();
                        if self.peek() == &Token::RParen {
                            self.advance();
                            self.expect(Token::BlockingAssign)?;
                            self.expect(Token::LParen)?;
                            let rise = self.parse_expr(0)?;
                            let fall = if self.peek() == &Token::Comma {
                                self.advance();
                                Some(self.parse_expr(0)?)
                            } else { None };
                            self.expect(Token::RParen)?;
                            self.skip_semi();
                            return Ok(Some(SpecifyItem::PathDelay { src: src.clone(), dst: dst.clone(), rise: Some(rise), fall }));
                        }
                    }
                }
            }
            self.pos = saved;
        }

        // Skip empty lines or unrecognized items
        Ok(None)
    }

    fn parse_specify_block(&mut self) -> Result<SpecifyBlock, SimError> {
        self.advance(); // consume 'specify'
        let mut items = Vec::new();
        loop {
            if self.peek() == &Token::EndSpecify || self.peek() == &Token::Eof {
                break;
            }
            if let Some(item) = self.parse_specify_item()? {
                items.push(item);
            } else {
                // Unknown item — skip token
                self.advance();
            }
        }
        self.expect(Token::EndSpecify)?;
        Ok(SpecifyBlock { items })
    }

    fn parse_udp_symbol(&mut self) -> Result<UdpSymbol, SimError> {
        let tok = self.peek().clone();
        match &tok {
            Token::Number { value, base, .. } if *base == Some(2) || *base == Some(10) || *base == Some(16) || *base == Some(8) => {
                // Could be sized like 1'b0, but in table it's just '0', '1', 'x'
                let trimmed = if let Some(b) = base {
                    let prefix = format!("'{}", *b as char);
                    if let Some(idx) = value.find(&prefix) {
                        value[idx + prefix.len()..].to_string()
                    } else {
                        value.clone()
                    }
                } else {
                    value.clone()
                };
                self.advance();
                match trimmed.as_str() {
                    "0" => Ok(UdpSymbol::Zero),
                    "1" => Ok(UdpSymbol::One),
                    "x" | "X" => Ok(UdpSymbol::X),
                    "?" => Ok(UdpSymbol::DontCare),
                    "-" => Ok(UdpSymbol::NoChange),
                    _ if trimmed.starts_with('(') => {
                        let end = trimmed.find(')').unwrap_or(trimmed.len() - 1);
                        let edge = trimmed[1..end].to_string();
                        Ok(UdpSymbol::Edge(edge))
                    }
                    _ => Err(SimError::parse(format!("line {}: invalid UDP table symbol '{}'", self.peek_line(), trimmed))),
                }
            }
            Token::Number { value, .. } if value == "0" || value == "1" => {
                self.advance();
                match value.as_str() {
                    "0" => Ok(UdpSymbol::Zero),
                    "1" => Ok(UdpSymbol::One),
                    _ => Ok(UdpSymbol::X),
                }
            }
            Token::FillLit(_) => {
                self.advance();
                Ok(UdpSymbol::X)
            }
            Token::Minus => {
                self.advance();
                Ok(UdpSymbol::NoChange)
            }
            Token::Question => {
                self.advance();
                Ok(UdpSymbol::DontCare)
            }
            Token::LParen => {
                // Edge transition: (01), (0x), etc.
                self.advance();
                let mut edge_str = String::new();
                // Read content until )
                while self.peek() != &Token::Eof && self.peek() != &Token::RParen {
                    match self.peek() {
                        Token::Number { value, .. } => {
                            edge_str.push_str(value);
                            self.advance();
                        }
                        Token::Question => {
                            edge_str.push('?');
                            self.advance();
                        }
                        Token::FillLit(_) => {
                            edge_str.push('x');
                            self.advance();
                        }
                        Token::Ident(s) => {
                            edge_str.push_str(&s);
                            self.advance();
                        }
                        _ => break,
                    }
                }
                self.expect(Token::RParen)?;
                Ok(UdpSymbol::Edge(edge_str))
            }
            Token::Ident(s) if s == "x" || s == "X" => {
                self.advance();
                Ok(UdpSymbol::X)
            }
            Token::Ident(s) if s == "r" || s == "R" => {
                self.advance();
                Ok(UdpSymbol::Edge("01".to_string()))
            }
            Token::Ident(s) if s == "f" || s == "F" => {
                self.advance();
                Ok(UdpSymbol::Edge("10".to_string()))
            }
            Token::Ident(s) if s == "p" || s == "P" => {
                self.advance();
                Ok(UdpSymbol::Edge("p".to_string()))
            }
            Token::Ident(s) if s == "n" || s == "N" => {
                self.advance();
                Ok(UdpSymbol::Edge("n".to_string()))
            }
            Token::Ident(s) if s == "*" || s == "Star" => {
                self.advance();
                Ok(UdpSymbol::Edge("??".to_string()))
            }
            _ => Err(SimError::parse(format!("line {}: unexpected token in UDP table: {}", self.peek_line(), tok))),
        }
    }

    fn parse_udp_table(&mut self, is_sequential: bool) -> Result<Vec<UdpTableEntry>, SimError> {
        self.expect(Token::Table)?;
        self.skip_semi();

        let mut entries = Vec::new();
        loop {
            if self.peek() == &Token::EndTable {
                self.advance();
                break;
            }
            if self.peek() == &Token::Eof {
                return Err(SimError::parse("unexpected EOF in UDP table"));
            }
            // Parse one line
            let mut inputs = Vec::new();
            loop {
                if self.peek() == &Token::Colon {
                    self.advance();
                    break;
                }
                let sym = self.parse_udp_symbol()?;
                inputs.push(sym);
            }
            if is_sequential {
                // Sequential UDP: inputs : current_state : output ;
                let current_state = self.parse_udp_symbol()?;
                inputs.push(current_state);
                self.expect(Token::Colon)?;
            }
            let output = self.parse_udp_symbol()?;
            self.skip_semi();
            entries.push(UdpTableEntry { inputs, output });
        }
        Ok(entries)
    }

    fn parse_udp_declaration(&mut self) -> Result<UdpDef, SimError> {
        self.expect(Token::Primitive)?;
        let name = self.expect_ident()?;

        // Parse port list: (output [reg] port1, input port2, input port3, ...)
        self.expect(Token::LParen)?;
        let mut ports = Vec::new();
        let mut is_sequential = false;

        loop {
            if self.peek() == &Token::RParen {
                self.advance();
                break;
            }
            let direction = if self.peek() == &Token::Output {
                self.advance();
                // Check for 'output reg name' (sequential UDP)
                if self.peek() == &Token::Reg {
                    self.advance();
                    let name = self.expect_ident()?;
                    ports.push(UdpPort { direction: PortDirection::Output, name, is_reg: true });
                    is_sequential = true;
                    if self.peek() == &Token::Comma { self.advance(); }
                    continue;
                }
                PortDirection::Output
            } else if self.peek() == &Token::Input {
                self.advance();
                // Check for 'input reg name'
                if self.peek() == &Token::Reg {
                    self.advance();
                }
                let name = self.expect_ident()?;
                ports.push(UdpPort { direction: PortDirection::Input, name, is_reg: false });
                if self.peek() == &Token::Comma { self.advance(); }
                continue;
            } else if self.peek() == &Token::Inout {
                self.advance();
                let name = self.expect_ident()?;
                ports.push(UdpPort { direction: PortDirection::Inout, name, is_reg: false });
                if self.peek() == &Token::Comma { self.advance(); }
                continue;
            } else if self.peek() == &Token::Reg {
                // bare reg without direction (non-standard)
                self.advance();
                is_sequential = true;
                let name = self.expect_ident()?;
                ports.push(UdpPort { direction: PortDirection::Output, name, is_reg: true });
                if self.peek() == &Token::Comma { self.advance(); }
                continue;
            } else {
                return Err(SimError::parse(format!("line {}: expected direction (input/output) in UDP port list", self.peek_line())));
            };

            let name = self.expect_ident()?;
            ports.push(UdpPort { direction, name, is_reg: false });

            if self.peek() == &Token::Comma {
                self.advance();
            }
        }
        self.skip_semi();

        // Check for optional initial statement (sequential UDP)
        let mut initial_output = None;
        if self.peek() == &Token::Initial {
            self.advance();
            // Expect output port name
            if matches!(self.peek(), Token::Ident(_)) || self.peek() == &Token::Output {
                self.advance();
            }
            // expect =
            self.expect(Token::BlockingAssign)?;
            let sym = self.parse_udp_symbol()?;
            initial_output = Some(sym);
            self.skip_semi();
        }

        let table = self.parse_udp_table(is_sequential)?;
        self.expect(Token::EndPrimitive)?;
        self.skip_semi();

        Ok(UdpDef {
            name,
            ports,
            table,
            is_sequential,
            initial_output,
        })
    }

    fn parse_config_decl(&mut self) -> Result<ConfigDecl, SimError> {
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
                            default_liblist = Some(name);
                        }
                    }
                    self.skip_semi();
                }
                Token::Instance => {
                    self.advance();
                    let mut instance_path = String::new();
                    if let Token::Ident(s) = self.peek().clone() {
                        self.advance();
                        instance_path = s;
                    }
                    // Handle hierarchical paths: top.sub1
                    while self.peek() == &Token::Dot {
                        self.advance();
                        if let Token::Ident(s) = self.peek().clone() {
                            self.advance();
                            instance_path.push('.');
                            instance_path.push_str(&s);
                        }
                    }
                    if self.peek() == &Token::Liblist {
                        self.advance();
                        if let Token::Ident(lib) = self.peek().clone() {
                            self.advance();
                            rules.push(ConfigRule::InstanceLiblist {
                                instance: instance_path,
                                liblist: lib,
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
                        cell_name = s;
                    }
                    if self.peek() == &Token::Liblist {
                        self.advance();
                        if let Token::Ident(lib) = self.peek().clone() {
                            self.advance();
                            rules.push(ConfigRule::CellLiblist {
                                cell: cell_name,
                                liblist: lib,
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
                            rules.push(ConfigRule::UseLiblist { liblist: lib });
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

    fn parse_package_decl(&mut self) -> Result<PackageDecl, SimError> {
        self.advance(); // consume 'package'
        let name = self.expect_ident()?;
        self.skip_semi();
        let mut items = Vec::new();
        loop {
            match self.peek() {
                Token::EndPackage => {
                    self.advance();
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) {
                            self.advance();
                        }
                    }
                    break;
                }
                Token::Eof => return Err(SimError::parse("unexpected EOF in package")),
                _ => {
                    match self.peek() {
                        Token::Param | Token::Parameter | Token::LocalParam => {
                            let is_localparam = self.peek() == &Token::LocalParam;
                            self.advance();

                            // Handle 'parameter type X = type_expr'
                            if self.peek() == &Token::Type {
                                self.advance();
                                let pname = self.expect_ident()?;
                                let type_default = if self.peek() == &Token::BlockingAssign {
                                    self.advance();
                                    Some(self.parse_type_expr()?)
                                } else { None };
                                self.skip_semi();
                                items.push(PackageItem::Param(ParamDecl {
                                    name: pname, dtype: None, range: None,
                                    default: None, is_localparam, is_type_param: true, type_default,
                                }));
                                continue;
                            }

                            // Parse optional built-in type keyword
                            let mut dtype = None;
                            match self.peek() {
                                Token::Integer => { self.advance(); dtype = Some(DataType::Integer); }
                                Token::Int => { self.advance(); dtype = Some(DataType::Int); }
                                Token::Reg => { self.advance(); dtype = Some(DataType::Logic); }
                                Token::Logic => { self.advance(); dtype = Some(DataType::Logic); }
                                Token::Bit => { self.advance(); dtype = Some(DataType::Bit); }
                                Token::Byte => { self.advance(); dtype = Some(DataType::Byte); }
                                Token::Shortint => { self.advance(); dtype = Some(DataType::Shortint); }
                                Token::Longint => { self.advance(); dtype = Some(DataType::Longint); }
                                Token::Time => { self.advance(); dtype = Some(DataType::Time); }
                                _ => {}
                            }

                            // Handle signed/unsigned
                            if self.peek() == &Token::Signed {
                                self.advance();
                                let inner = dtype.take().unwrap_or(DataType::Int);
                                dtype = Some(DataType::Signed(Box::new(inner)));
                            }
                            if self.peek() == &Token::Unsigned {
                                self.advance();
                            }

                            // Handle user-defined type (ident followed by ident or [)
                            let mut type_ident = None;
                            if dtype.is_none() {
                                if let Token::Ident(s) = self.peek() {
                                    let ahead = self.peek_ahead(1).clone();
                                    if matches!(ahead, Token::Ident(_) | Token::LBrack | Token::Signed | Token::Unsigned) {
                                        type_ident = Some(s.clone());
                                        self.advance();
                                    }
                                }
                            }

                            // Parse optional range [msb:lsb]
                            let mut range = None;
                            if self.peek() == &Token::LBrack {
                                self.advance();
                                let msb = self.parse_expr(0)?;
                                self.expect(Token::Colon)?;
                                let lsb = self.parse_expr(0)?;
                                self.expect(Token::RBrack)?;
                                range = Some((msb, lsb));
                                // Skip additional packed dimensions: [a:b][c:d]
                                while self.peek() == &Token::LBrack {
                                    self.advance();
                                    self.parse_expr(0)?;
                                    self.expect(Token::Colon)?;
                                    self.parse_expr(0)?;
                                    self.expect(Token::RBrack)?;
                                }
                            }

                            // Parse parameter name(s)
                            loop {
                                let pk = self.peek().clone();
                                let pname = match &pk {
                                    Token::Ident(s) => { self.advance(); s.clone() }
                                    _ => break,
                                };
                                // Skip unpacked array dimension after name: name [N]
                                if self.peek() == &Token::LBrack && self.peek_ahead(1) != &Token::Colon {
                                    self.advance();
                                    self.parse_expr(0)?;
                                    self.expect(Token::RBrack)?;
                                }
                                let default = if self.peek() == &Token::BlockingAssign {
                                    self.advance();
                                    Some(self.parse_expr(0)?)
                                } else { None };
                                let resolved_dtype = if let Some(t) = &type_ident {
                                    Some(DataType::UserDefined(t.clone()))
                                } else {
                                    dtype.clone()
                                };
                                items.push(PackageItem::Param(ParamDecl {
                                    name: pname, dtype: resolved_dtype,
                                    range: range.clone(), default,
                                    is_localparam, is_type_param: false, type_default: None,
                                }));
                                if self.peek() == &Token::Comma {
                                    self.advance();
                                } else {
                                    break;
                                }
                            }
                            self.skip_semi();
                        }
                        Token::Function => {
                            items.push(PackageItem::Function(self.parse_function(false)?));
                        }
                        Token::Task => {
                            items.push(PackageItem::Task(self.parse_task(false)?));
                        }
                        Token::Typedef => {
                            // Check for 'typedef class' (forward declaration)
                            if matches!(self.peek_ahead(1), Token::Class | Token::Virtual) {
                                self.advance(); // consume 'typedef'
                                while self.peek() != &Token::Semi && self.peek() != &Token::Eof {
                                    self.advance();
                                }
                                self.skip_semi();
                            } else {
                                let td = self.parse_typedef()?;
                                self.typedef_names.push(td.name.clone());
                                self.package_tdefs.entry(name.clone()).or_default().push(td.name.clone());
                                items.push(PackageItem::Typedef(td));
                            }
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
                            // Register imported typedef names
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
                            items.push(PackageItem::Import { package: pkg, item });
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

    fn parse_class(&mut self) -> Result<ClassDecl, SimError> {
        self.advance(); // consume 'class'
        let mut type_params = Vec::new();
        if self.peek() == &Token::Hash {
            self.advance();
            self.expect(Token::LParen)?;
            loop {
                if self.peek() == &Token::RParen { break; }
                self.expect(Token::Type)?;
                let tp_name = self.expect_ident()?;
                let default_type = if self.peek() == &Token::BlockingAssign {
                    self.advance();
                    Some(self.parse_type_expr()?)
                } else { None };
                type_params.push(TypeParam { name: tp_name, default_type });
                if self.peek() == &Token::Comma { self.advance(); } else { break; }
            }
            self.expect(Token::RParen)?;
        }
        let name = self.expect_ident()?;
        let extends = if self.peek() == &Token::Extends {
            self.advance();
            let base_name = self.expect_ident()?;
            // Handle parameterized base class: extends Base #(.PARAM(value), ...)
            if self.peek() == &Token::Hash {
                self.advance();
                if self.peek() == &Token::LParen {
                    self.skip_balanced_paren()?;
                }
            }
            Some(base_name)
        } else {
            None
        };
        self.expect(Token::Semi)?;
        self.type_param_names = type_params.iter().map(|tp| tp.name.clone()).collect();
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
                        _ => return Err(SimError::parse(format!("line {}: expected function/task after virtual", self.peek_line()))),
                    }
                }
                Token::Task => {
                    members.push(ClassMember::Task(self.parse_task(false)?));
                }
                Token::Input | Token::Output | Token::Inout | Token::Reg | Token::Logic | Token::Wire | Token::Int | Token::Integer | Token::Signed | Token::Bit | Token::Byte | Token::Shortint | Token::Longint | Token::Time | Token::String | Token::Mailbox | Token::Semaphore | Token::Real | Token::RealTime | Token::Enum | Token::Struct | Token::Union | Token::Wand | Token::Wor | Token::Tri | Token::Tri0 | Token::Tri1 | Token::TriAnd | Token::TriOr | Token::Supply0 | Token::Supply1 => {
                    let mut decl = self.parse_decl()?;
                    for n in &mut decl.names { n.is_rand = false; }
                    members.push(ClassMember::Decl(decl));
                }
                Token::Rand | Token::RandC => {
                    self.advance();
                    let mut decl = self.parse_decl()?;
                    for n in &mut decl.names { n.is_rand = true; }
                    members.push(ClassMember::Decl(decl));
                }
                Token::Ident(name) if self.type_param_names.contains(name) => {
                    let tp_name = name.clone();
                    self.advance();
                    let decl_expr_range = if self.peek() == &Token::LBrack { self.parse_range()? } else { None };
                    let mut extra_packed: Vec<(ExprRange, Option<Range>)> = Vec::new();
                    while self.peek() == &Token::LBrack && self.peek_ahead(1) == &Token::Colon {
                        if let Some(er) = self.parse_range()? {
                            extra_packed.push((er, None));
                        }
                    }
                    let names = self.parse_decl_names(decl_expr_range, extra_packed)?;
                    self.skip_semi();
                    members.push(ClassMember::Decl(crate::ast::types::Decl { dtype: DataType::UserDefined(tp_name), kind: crate::ast::types::DeclKind::Logic, names }));
                }
                Token::Constraint => {
                    self.advance();
                    let cname = self.expect_ident()?;
                    self.expect(Token::LBrace)?;
                    let mut body = Vec::new();
                    while self.peek() != &Token::RBrace {
                        // Check for solve...before directive
                        if let Token::Ident(ref s) = self.peek() {
                            if s == "solve" {
                                self.advance();
                                let mut vars = Vec::new();
                                // Parse: solve x before y;
                                let first_var = self.expect_ident()?;
                                vars.push(first_var);
                                if let Token::Ident(ref s2) = self.peek() {
                                    if s2 == "before" {
                                        self.advance();
                                        loop {
                                            let v = self.expect_ident()?;
                                            vars.push(v);
                                            if self.peek() == &Token::Comma {
                                                self.advance();
                                            } else {
                                                break;
                                            }
                                        }
                                    }
                                }
                                self.skip_semi();
                                body.push(ConstraintItem::SolveBefore { vars });
                                continue;
                            }
                        }
                        let expr = self.parse_expr(0)?;
                        self.skip_semi();
                        body.push(ConstraintItem::Expr(expr));
                    }
                    self.advance(); // consume '}'
                    members.push(ClassMember::Constraint { name: cname, body });
                }
                _ => {
                    // Skip unknown tokens to avoid getting stuck
                    self.advance();
                }
            }
        }
        self.type_param_names.clear();
        Ok(ClassDecl { name, extends, type_params, members })
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
            _ => return Err(SimError::parse(format!("line {}: expected module name", self.peek_line()))),
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
                "*".to_string()
            } else {
                self.expect_ident()?
            };
            self.skip_semi();
            items.push(ModuleItem::Import { package: pkg, item: item.clone() });
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
            Token::EndProgram => { self.advance(); }
            Token::EndInterface => { self.advance(); }
            _ => { self.expect(Token::Endmodule)?; }
        }
        if self.peek() == &Token::Colon {
            self.advance();
            if matches!(self.peek(), Token::Ident(_)) {
                self.advance();
            }
        }

        Ok(Module { name, ports, params, decls, items })
    }

    fn parse_interface_fast(&mut self) -> Result<(), SimError> {
        self.advance(); // consume 'interface'
        match self.peek() {
            Token::Ident(_) => { self.advance(); }
            _ => return Err(SimError::parse("expected interface name")),
        }
        self.skip_semi();
        loop {
            match self.peek() {
                Token::EndInterface | Token::Eof => { self.advance(); break; }
                _ => {
                    match self.peek() {
                        Token::ModPort => {
                            self.advance(); // consume 'modport'
                            loop {
                                match self.peek() {
                                    Token::Ident(_) => { self.advance(); }
                                    _ => {}
                                }
                                self.skip_until_semi_or_end()?;
                                break;
                            }
                        }
                        Token::Param | Token::Parameter | Token::LocalParam
                        | Token::Function | Token::Task => {
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
        if let Token::Ident(_) = self.peek() { self.advance(); }
        if self.peek() == &Token::Hash {
            self.advance(); // #
            if self.peek() == &Token::LParen { self.skip_balanced_paren()?; }
        }
        if self.peek() == &Token::LParen { self.skip_balanced_paren()?; }
        self.skip_semi();
        loop {
            match self.peek() {
                Token::EndProgram | Token::Eof => { self.advance(); break; }
                _ => { self.advance(); }
            }
        }
        Ok(())
    }

    fn skip_balanced_paren(&mut self) -> Result<(), SimError> {
        let mut depth = 0;
        loop {
            match self.peek() {
                Token::LParen => { depth += 1; self.advance(); }
                Token::RParen => {
                    depth -= 1;
                    self.advance();
                    if depth == 0 { break; }
                }
                Token::Eof => return Err(SimError::parse("unexpected EOF in balanced paren")),
                _ => { self.advance(); }
            }
        }
        Ok(())
    }

    fn parse_interface(&mut self) -> Result<Interface, SimError> {
        self.advance(); // consume 'interface'
        self.typedef_names.clear();

        let name = match self.peek() {
            Token::Ident(s) => { let n = s.clone(); self.advance(); n }
            _ => return Err(SimError::parse(format!("line {}: expected interface name", self.peek_line()))),
        };
        self.skip_semi();

        let params = Vec::new();
        let mut decls = Vec::new();
        let mut modports = Vec::new();

        loop {
            match self.peek() {
                Token::EndInterface | Token::Eof => { break; }
                _ => {
                    match self.peek() {
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
                    }
                }
            }
        }
        match self.peek() {
            Token::EndInterface => { self.advance(); }
            _ => { return Err(SimError::parse(format!("line {}: expected endinterface", self.peek_line()))); }
        }
        if self.peek() == &Token::Colon {
            self.advance();
            if matches!(self.peek(), Token::Ident(_)) {
                self.advance();
            }
        }

        Ok(Interface { name, params, decls, modports })
    }

    fn parse_modport(&mut self) -> Result<Modport, SimError> {
        self.advance(); // consume 'modport'
        let name = match self.peek() {
            Token::Ident(s) => { let n = s.clone(); self.advance(); n }
            _ => return Err(SimError::parse(format!("line {}: expected modport name", self.peek_line()))),
        };
        self.expect(Token::LParen)?;
        let mut items = Vec::new();
        loop {
            let dir = match self.peek() {
                Token::Input => { self.advance(); PortDirection::Input }
                Token::Output => { self.advance(); PortDirection::Output }
                Token::Inout => { self.advance(); PortDirection::Inout }
                _ => return Err(SimError::parse(format!("line {}: expected direction in modport", self.peek_line()))),
            };
            // Collect all signals under this direction, comma-separated
            loop {
                let sig_name = match self.peek() {
                    Token::Ident(s) => { let n = s.clone(); self.advance(); n }
                    _ => return Err(SimError::parse(format!("line {}: expected signal name in modport", self.peek_line()))),
                };
                items.push(ModportItem { name: sig_name, direction: dir.clone() });
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
                Token::RParen => { self.advance(); break; }
                _ => continue,
            }
        }
        self.skip_semi();
        Ok(Modport { name, items })
    }

    fn parse_param_list(&mut self, params: &mut Vec<ParamDecl>) -> Result<(), SimError> {
        let mut is_localparam = false;
        loop {
            match self.peek() {
                Token::Param | Token::Parameter => { is_localparam = false; self.advance(); }
                Token::LocalParam => { is_localparam = true; self.advance(); }
                _ => {}
            }

            // Skip optional type keyword (integer, int, reg, logic, bit, string)
            let mut type_ident = None;
            match self.peek() {
                Token::Integer | Token::Int | Token::Reg | Token::Logic | Token::Bit
                | Token::String => { self.advance(); }
                Token::Ident(_) if matches!(self.peek_ahead(1), Token::Ident(_) | Token::LBrack | Token::Scope) => {
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
            let resolved_dtype = type_ident.as_ref().map(|t| DataType::UserDefined(t.clone())).or(dtype);

            params.push(ParamDecl { name, dtype: resolved_dtype, range, default, is_localparam, is_type_param, type_default });

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
                        Token::Ident(_) => { self.advance(); }
                        _ => return Err(SimError::parse(format!("line {}: expected port name", self.peek_line()))),
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
                        Token::Input => { self.advance(); PortDirection::Input }
                        Token::Output => { self.advance(); PortDirection::Output }
                        Token::Inout => { self.advance(); PortDirection::Inout }
                        _ => PortDirection::Input,
                    };

                    if matches!(self.peek(), Token::Wire | Token::Reg | Token::Logic
                        | Token::Bit | Token::Byte | Token::Shortint | Token::Longint | Token::Time
                        | Token::Int | Token::Integer) {
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
                            dtype_name = Some(name);
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
                                // Skip unpacked array dimensions after port name: data_i [N]
                                while self.peek() == &Token::LBrack
                                    && self.peek_ahead(1) != &Token::Colon
                                {
                                    self.advance(); // [
                                    self.parse_expr(0)?;
                                    self.expect(Token::RBrack)?;
                                }
                                ports.push(Port {
                                    name: name.clone(),
                                    direction: dir.clone(),
                                    range: range.clone(),
                                    expr_range: expr_range.clone(),
                                    dtype_name: dtype_name.clone(),
                                });
                            }
                            _ => break,
                        }

                        if self.peek() == &Token::Comma {
                            let ahead = self.peek_ahead(1).clone();
                            let is_new_port = ahead == Token::Input || ahead == Token::Output
                                || ahead == Token::Inout
                                || (matches!(&ahead, Token::Ident(_)) && matches!(self.peek_ahead(2), Token::Scope));
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
        if matches!(self.peek(), Token::BlockingAssign | Token::NonBlockingAssign) {
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
                if matches!(self.peek(), Token::Wire | Token::Reg | Token::Logic | Token::Int | Token::Integer
                    | Token::Bit | Token::Byte | Token::Shortint | Token::Longint | Token::Time
                    | Token::String | Token::Real | Token::RealTime | Token::Enum | Token::Struct | Token::Union) {
                    Ok(Some(ModuleItem::Decl(self.parse_decl()?)))
                } else if let Token::Ident(_) = self.peek() {
                    // Implicit var with type inference (treated as logic)
                    let vname = self.expect_ident()?;
                    let names = vec![DeclVar {
                        name: vname, range: None, expr_range: None, array_range: None,
                        extra_packed_dims: vec![], is_dynamic: false, is_queue: false,
                        is_associative: false, assoc_key_type: None, is_rand: false, is_const: false, expr: None,
                    }];
                    self.skip_semi();
                    Ok(Some(ModuleItem::Decl(Decl { dtype: DataType::Logic, kind: DeclKind::Logic, names })))
                } else {
                    Ok(None)
                }
            }
            Token::Wire | Token::Reg | Token::Logic | Token::Int | Token::Integer
                | Token::Bit | Token::Byte | Token::Shortint | Token::Longint | Token::Time
                | Token::String | Token::Real | Token::RealTime
                | Token::Mailbox | Token::Semaphore
                | Token::Enum | Token::Struct | Token::Union
                | Token::Wand | Token::Wor | Token::Tri
                | Token::Tri0 | Token::Tri1 | Token::TriAnd | Token::TriOr
                | Token::Supply0 | Token::Supply1 => {
                let decl = self.parse_decl()?;
                Ok(Some(ModuleItem::Decl(decl)))
            }
            Token::Ident(name) => {
                if self.class_names.contains(name) || self.typedef_names.contains(name) {
                    let dtype = DataType::UserDefined(name.clone());
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
                                name: vname, range: None, expr_range: None, array_range: None, 
 extra_packed_dims: vec![],is_dynamic: false, is_queue: false, is_associative: false, assoc_key_type: None, is_rand: false, is_const: false, expr: None,
                            });
                        } else {
                            if self.peek() == &Token::BlockingAssign {
                                // = new() or = expr — skip and continue to skip_semi
                                self.skip_semi();
                                return Ok(Some(ModuleItem::Decl(Decl { dtype, kind: DeclKind::Logic, names })));
                            }
                            break;
                        }
                        if self.peek() == &Token::Comma { self.advance(); } else { break; }
                    }
                    self.skip_semi();
                    Ok(Some(ModuleItem::Decl(Decl { dtype, kind: DeclKind::Logic, names })))
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
                } else {
                    let line = self.peek_line();
                    Err(SimError::parse(format!("line {}: unexpected token in module body: {}", line, self.peek())))
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
                    Token::Integer => { self.advance(); dtype = Some(DataType::Integer); }
                    Token::Int => { self.advance(); dtype = Some(DataType::Int); }
                    Token::Reg => { self.advance(); dtype = Some(DataType::Logic); }
                    Token::Logic => { self.advance(); dtype = Some(DataType::Logic); }
                    Token::Bit => { self.advance(); dtype = Some(DataType::Bit); }
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
                        Token::Ident(s) => { self.advance(); s.clone() }
                        _ => break,
                    };
                    let default = if self.peek() == &Token::BlockingAssign {
                        self.advance();
                        Some(self.parse_expr(0)?)
                    } else {
                        None
                    };
                    params.push(ParamDecl { name, dtype: dtype.clone(), range: range.clone(), default, is_localparam, is_type_param: false, type_default: None });
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
                        items: vec![GenerateItem::Items(params.into_iter().map(|p| ModuleItem::Param(p)).collect())],
                    })))
                }
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
                // Check for DPI-C import
                if self.peek() == &Token::StringLit("DPI-C".to_string())
                    || self.peek() == &Token::StringLit("DPI".to_string()) {
                    let result = self.parse_dpi_import()?;
                    return Ok(Some(ModuleItem::DpiImport(result)));
                }
                let pkg = self.expect_ident()?;
                self.expect(Token::Scope)?;
                let item = if self.peek() == &Token::Star {
                    self.advance();
                    "*".to_string()
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
            Token::Assert | Token::Assume | Token::Cover | Token::Expect => {
                self.skip_until_semi_or_end()?;
                Ok(None)
            }
            Token::Void | Token::Auto | Token::Static => {
                self.skip_until_semi_or_end()?;
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
                Token::Begin => { depth += 1; self.advance(); }
                Token::End => { depth = depth.saturating_sub(1); self.advance(); }
                _ => { self.advance(); }
            }
        }
    }

    fn skip_to_stmt_boundary(&mut self) {
        loop {
            match self.peek() {
                Token::Semi => { self.advance(); return; }
                Token::End | Token::Endcase | Token::EndFunction
                | Token::EndTask | Token::Endmodule | Token::Eof => { return; }
                _ => { self.advance(); }
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
                        if depth == 0 { return; }
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
                    Some(DataType::UserDefined(format!("{}::{}", pkg, type_name)))
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
                return Ok(Decl { dtype: DataType::UserDefined("__mailbox".to_string()), kind: DeclKind::Reg, names });
            }
            Token::Semaphore => {
                self.advance();
                let names = self.parse_decl_names(None, vec![])?;
                self.skip_semi();
                return Ok(Decl { dtype: DataType::UserDefined("__semaphore".to_string()), kind: DeclKind::Reg, names });
            }
            Token::Ident(_) => {
                let name = self.expect_ident()?;
                let mut dtype = DataType::UserDefined(name);
                // Handle scoped type: pkg::type
                if self.peek() == &Token::Scope {
                    self.advance();
                    let type_name = self.expect_ident()?;
                    dtype = DataType::UserDefined(format!("{}::{}", match &dtype { DataType::UserDefined(s) => s, _ => "", }, type_name));
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

        Ok(Decl { dtype: effective_dtype, kind, names })
    }

    fn parse_decl_names(&mut self, decl_expr_range: Option<ExprRange>, extra_packed_dims: Vec<(ExprRange, Option<Range>)>) -> Result<Vec<DeclVar>, SimError> {
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
                                self.advance(); self.advance();
                                is_dynamic = true;
                                None
                            } else if self.peek_ahead(1) == &Token::Dollar && self.peek_ahead(2) == &Token::RBrack {
                                self.advance(); self.advance(); self.advance();
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
                                self.advance(); self.advance(); self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::Bit);
                                None
                            } else if self.peek_ahead(1) == &Token::Logic {
                                // logic-key associative array
                                self.advance(); self.advance(); self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::Logic);
                                None
                            } else if self.peek_ahead(1) == &Token::Byte {
                                // byte-key associative array
                                self.advance(); self.advance(); self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::Byte);
                                None
                            } else if self.peek_ahead(1) == &Token::Shortint {
                                // shortint-key associative array
                                self.advance(); self.advance(); self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::Shortint);
                                None
                            } else if self.peek_ahead(1) == &Token::Longint {
                                // longint-key associative array
                                self.advance(); self.advance(); self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::Longint);
                                None
                            } else if self.peek_ahead(1) == &Token::Star && self.peek_ahead(2) == &Token::RBrack {
                                // wildcard [*] associative array
                                self.advance(); self.advance(); self.expect(Token::RBrack)?;
                                is_associative = true;
                                assoc_key_type = Some(DataType::Int);
                                None
                            } else if self.peek_ahead(1) == &Token::Colon
                                || self.peek_ahead(2) == &Token::Colon {
                                let er = self.parse_range()?;
                                er.as_ref().and_then(|er| {
                                    if let (Ok(m), Ok(l)) = (const_eval_simple(&er.msb), const_eval_simple(&er.lsb)) {
                                        Some(Range { msb: m as usize, lsb: l as usize })
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
                                self.advance(); self.advance();
                                is_dynamic = true;
                                (None, None)
                            } else if self.peek_ahead(1) == &Token::Dollar && self.peek_ahead(2) == &Token::RBrack {
                                self.advance(); self.advance(); self.advance();
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
                            }
                        } else {
                            (None, None)
                        }
                    };
                    let var_range = var_expr_range.as_ref().and_then(|er| {
                        if let (Ok(m), Ok(l)) = (const_eval_simple(&er.msb), const_eval_simple(&er.lsb)) {
                            Some(Range { msb: m as usize, lsb: l as usize })
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
                        name: name.clone(),
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

    fn parse_enum_members(&mut self) -> Result<Vec<(String, Option<Expr>)>, SimError> {
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
                _ => return Err(SimError::parse(format!("line {}: expected identifier in enum", self.peek_line()))),
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
                Token::Logic => { self.advance(); DataType::Logic }
                Token::Int => { self.advance(); DataType::Int }
                Token::Integer => { self.advance(); DataType::Integer }
                Token::Bit => { self.advance(); DataType::Bit }
                Token::Byte => { self.advance(); DataType::Byte }
                Token::Shortint => { self.advance(); DataType::Shortint }
                Token::Longint => { self.advance(); DataType::Longint }
                Token::Time => { self.advance(); DataType::Time }
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
                        Token::Time => { self.advance(); DataType::Time }
                        _ => DataType::Logic,
                    };
                    DataType::Signed(Box::new(inner))
                }
                Token::Struct => {
                    self.advance();
                    if matches!(self.peek(), Token::Ident(s) if s == "packed") { self.advance(); }
                    DataType::StructType { members: self.parse_struct_body()? }
                }
                Token::Ident(name) => {
                    let name = name.clone();
                    self.advance();
                    // Handle scoped type: pkg::type
                    if self.peek() == &Token::Scope {
                        self.advance();
                        let type_name = self.expect_ident()?;
                        DataType::UserDefined(format!("{}::{}", name, type_name))
                    } else {
                        DataType::UserDefined(name)
                    }
                }
                _ => return Err(SimError::parse(format!("line {}: expected type in struct/union member", self.peek_line()))),
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

    fn parse_typedef(&mut self) -> Result<TypedefDecl, SimError> {
        self.advance(); // consume typedef
        let (name, dtype, range) = match self.peek() {
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
                        if self.peek() == &Token::Unsigned { self.advance(); }
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
                    return Err(SimError::parse(format!("line {}: expected name after typedef enum", self.peek_line())));
                }
            }
            Token::Bit => {
                self.advance();
                let mut dtype = DataType::Bit;
                if self.peek() == &Token::Signed { self.advance(); dtype = DataType::Signed(Box::new(dtype)); }
                if self.peek() == &Token::Unsigned { self.advance(); }
                let range = if self.peek() == &Token::LBrack { self.parse_range()? } else { None };
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(SimError::parse(format!("line {}: expected name after typedef bit", self.peek_line())));
                }
            }
            Token::Byte => {
                self.advance();
                let mut dtype = DataType::Byte;
                if self.peek() == &Token::Signed { self.advance(); dtype = DataType::Signed(Box::new(dtype)); }
                if self.peek() == &Token::Unsigned { self.advance(); }
                let range = if self.peek() == &Token::LBrack { self.parse_range()? } else { None };
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(SimError::parse(format!("line {}: expected name after typedef byte", self.peek_line())));
                }
            }
            Token::Shortint => {
                self.advance();
                let mut dtype = DataType::Shortint;
                if self.peek() == &Token::Signed { self.advance(); dtype = DataType::Signed(Box::new(dtype)); }
                if self.peek() == &Token::Unsigned { self.advance(); }
                let range = if self.peek() == &Token::LBrack { self.parse_range()? } else { None };
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(SimError::parse(format!("line {}: expected name after typedef shortint", self.peek_line())));
                }
            }
            Token::Longint => {
                self.advance();
                let mut dtype = DataType::Longint;
                if self.peek() == &Token::Signed { self.advance(); dtype = DataType::Signed(Box::new(dtype)); }
                if self.peek() == &Token::Unsigned { self.advance(); }
                let range = if self.peek() == &Token::LBrack { self.parse_range()? } else { None };
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(SimError::parse(format!("line {}: expected name after typedef longint", self.peek_line())));
                }
            }
            Token::Time => {
                self.advance();
                let range = if self.peek() == &Token::LBrack { self.parse_range()? } else { None };
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, DataType::Time, range)
                } else {
                    return Err(SimError::parse(format!("line {}: expected name after typedef time", self.peek_line())));
                }
            }
            Token::Int => {
                self.advance();
                let mut dtype = DataType::Int;
                if self.peek() == &Token::Signed { self.advance(); dtype = DataType::Signed(Box::new(dtype)); }
                if self.peek() == &Token::Unsigned { self.advance(); }
                let range = if self.peek() == &Token::LBrack { self.parse_range()? } else { None };
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(SimError::parse(format!("line {}: expected name after typedef int", self.peek_line())));
                }
            }
            Token::Integer => {
                self.advance();
                let mut dtype = DataType::Integer;
                if self.peek() == &Token::Signed { self.advance(); dtype = DataType::Signed(Box::new(dtype)); }
                if self.peek() == &Token::Unsigned { self.advance(); }
                let range = if self.peek() == &Token::LBrack { self.parse_range()? } else { None };
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(SimError::parse(format!("line {}: expected name after typedef integer", self.peek_line())));
                }
            }
            Token::Logic => {
                self.advance();
                let mut dtype = DataType::Logic;
                if self.peek() == &Token::Signed { self.advance(); dtype = DataType::Signed(Box::new(dtype)); }
                if self.peek() == &Token::Unsigned { self.advance(); }
                let range = if self.peek() == &Token::LBrack { self.parse_range()? } else { None };
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(SimError::parse(format!("line {}: expected name after typedef logic", self.peek_line())));
                }
            }
            Token::Reg => {
                self.advance();
                let dtype = DataType::Logic;
                let range = if self.peek() == &Token::LBrack { self.parse_range()? } else { None };
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, dtype, range)
                } else {
                    return Err(SimError::parse(format!("line {}: expected name after typedef reg", self.peek_line())));
                }
            }
            Token::Struct => {
                self.advance();
                if matches!(self.peek(), Token::Ident(s) if s == "packed") { self.advance(); }
                let members = self.parse_struct_body()?;
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, DataType::StructType { members }, None)
                } else {
                    return Err(SimError::parse(format!("line {}: expected name after typedef struct", self.peek_line())));
                }
            }
            Token::Union => {
                self.advance();
                if matches!(self.peek(), Token::Ident(s) if s == "packed") { self.advance(); }
                let members = self.parse_struct_body()?;
                if let Token::Ident(name) = self.peek() {
                    let name = name.clone();
                    self.advance();
                    (name, DataType::UnionType { members }, None)
                } else {
                    return Err(SimError::parse(format!("line {}: expected name after typedef union", self.peek_line())));
                }
            }
            _ => return Err(SimError::parse(format!("line {}: expected type after typedef", self.peek_line()))),
        };
        self.skip_semi();
        Ok(TypedefDecl { name, dtype, range })
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
                Ok(GenerateItem::If { cond, true_items, false_items })
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
                    Token::Ident(n) => { self.advance(); n.clone() }
                    _ => return Err(SimError::parse(format!("line {}: expected genvar name", self.peek_line()))),
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
                Ok(GenerateItem::Case { case_type, expr, items, default })
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
        // Skip optional 'automatic'/'static' qualifier
        if matches!(self.peek(), Token::Auto | Token::Static) {
            self.advance();
        }
        // Parse optional return type
        let return_type = match self.peek() {
            Token::Void => { self.advance(); Some(Box::new(DataType::Void)) }
            Token::Int => { self.advance(); Some(Box::new(DataType::Int)) }
            Token::Integer => { self.advance(); Some(Box::new(DataType::Integer)) }
            Token::String => { self.advance(); Some(Box::new(DataType::String)) }
            Token::Byte => { self.advance(); Some(Box::new(DataType::Byte)) }
            Token::Shortint => { self.advance(); Some(Box::new(DataType::Shortint)) }
            Token::Longint => { self.advance(); Some(Box::new(DataType::Longint)) }
            Token::Time => { self.advance(); Some(Box::new(DataType::Time)) }
            Token::Bit => { self.advance(); Some(Box::new(DataType::Bit)) }
            Token::Logic => { self.advance(); Some(Box::new(DataType::Logic)) }
            Token::Signed => { self.advance(); Some(Box::new(DataType::Signed(Box::new(DataType::Logic)))) }
            Token::Ident(name) if self.type_param_names.contains(name) => {
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
        if self.peek() == &Token::Unsigned { self.advance(); }
        let range = if self.peek() == &Token::LBrack {
            self.parse_range()?
        } else {
            None
        };
        let name_tok = self.peek().clone();
        let name = match &name_tok {
            Token::Ident(n) => { self.advance(); n.clone() }
            Token::New => { self.advance(); "new".to_string() }
            _ => return Err(SimError::parse(format!("line {}: expected function name", self.peek_line()))),
        };
        // Parse ANSI-style port list in parens (e.g., function new(int level, string name))
        let mut ports = Vec::new();
        let mut decls = Vec::new();
        if self.peek() == &Token::LParen {
            self.advance();
            while self.peek() != &Token::RParen && self.peek() != &Token::Eof {
                // Track whether we saw int/integer for 32-bit default width
                let is_int = matches!(self.peek(), Token::Int | Token::Integer);
                // Skip type keywords and direction keywords
                if matches!(self.peek(),
                    Token::Int | Token::Integer | Token::String | Token::Void |
                    Token::Reg | Token::Logic | Token::Wire | Token::Signed | Token::Unsigned |
                    Token::Input | Token::Output | Token::Inout)
                {
                    self.advance();
                } else if let Token::Ident(name) = self.peek() {
                    if self.type_param_names.contains(name) {
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
                Token::Auto | Token::Static => {
                    // automatic/static variable declaration in function body
                    self.advance();
                    // Try to parse as declaration
                    if let Ok(decl) = self.parse_decl() {
                        decls.push(decl);
                    } else {
                        return Err(SimError::parse(format!("line {}: expected declaration after automatic/static", self.peek_line())));
                    }
                }
                Token::Begin => {
                    let stmts = self.parse_stmt_block()?;
                    self.expect(Token::EndFunction)?;
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) { self.advance(); }
                    }
                    return Ok(FunctionDecl { name, range, return_type, ports, decls, stmts, virtual_flag });
                }
                Token::EndFunction => {
                    self.advance(); // consume 'endfunction'
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) { self.advance(); }
                    }
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
        match self.peek() {
            Token::EndFunction => { self.advance(); }
            _ => { return Err(SimError::parse(format!("line {}: expected endfunction", self.peek_line()))); }
        }
        if self.peek() == &Token::Colon {
            self.advance();
            if matches!(self.peek(), Token::Ident(_)) { self.advance(); }
        }
        Ok(FunctionDecl { name, range, return_type, ports, decls, stmts, virtual_flag })
    }

    fn parse_task(&mut self, virtual_flag: bool) -> Result<TaskDecl, SimError> {
        self.advance(); // consume 'task'
        // Skip optional 'automatic'/'static' qualifier
        if matches!(self.peek(), Token::Auto | Token::Static) {
            self.advance();
        }
        let name = self.expect_ident()?;
        let mut ports = Vec::new();
        let mut decls = Vec::new();
        // Parse ANSI-style port list in parens (e.g., task set_val(input [7:0] x))
        if self.peek() == &Token::LParen {
            self.advance();
            while self.peek() != &Token::RParen && self.peek() != &Token::Eof {
                let is_int = matches!(self.peek(), Token::Int | Token::Integer);
                if matches!(self.peek(),
                    Token::Int | Token::Integer | Token::String | Token::Void |
                    Token::Reg | Token::Logic | Token::Wire | Token::Signed |
                    Token::Input | Token::Output | Token::Inout)
                {
                    self.advance();
                }
                let range: Option<Range> = if self.peek() == &Token::LBrack {
                    if let Ok(Some(er)) = self.parse_range() {
                        if let (Ok(m), Ok(l)) = (const_eval_simple(&er.msb), const_eval_simple(&er.lsb)) {
                            Some(Range { msb: m as usize, lsb: l as usize })
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
                Token::Input | Token::Output | Token::Inout => {
                    match self.peek() {
                        Token::Input => { self.advance(); }
                        Token::Output => { self.advance(); }
                        _ => { self.advance(); }
                    }
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
                    decls.push(self.parse_decl()?);
                }
                Token::Begin => {
                    let stmts = self.parse_stmt_block()?;
                    self.expect(Token::EndTask)?;
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) { self.advance(); }
                    }
                    return Ok(TaskDecl { name, ports, decls, stmts, virtual_flag });
                }
                Token::EndTask => {
                    self.advance(); // consume 'endtask'
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) { self.advance(); }
                    }
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
        match self.peek() {
            Token::EndTask => { self.advance(); }
            _ => { return Err(SimError::parse(format!("line {}: expected endtask", self.peek_line()))); }
        }
        if self.peek() == &Token::Colon {
            self.advance();
            if matches!(self.peek(), Token::Ident(_)) { self.advance(); }
        }
        Ok(TaskDecl { name, ports, decls, stmts, virtual_flag })
    }

    fn parse_always(&mut self) -> Result<AlwaysBlock, SimError> {
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
            return Ok(SensitivityList { events: vec![SensitivityEvent::Wildcard] });
        }
        if self.peek() == &Token::LParen && self.peek_ahead(1) == &Token::Star {
            self.advance(); // (
            self.advance(); // *
            // Check for @( * ) — closing paren may or may not be present
            if self.peek() == &Token::RParen {
                self.advance();
            }
            return Ok(SensitivityList { events: vec![SensitivityEvent::Wildcard] });
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
        Ok(Delay { rise, fall, turnoff })
    }

    fn parse_instance(&mut self) -> Result<ModuleInstance, SimError> {
        let name_tok = self.peek().clone();
        let module_name = match &name_tok {
            Token::Ident(s) => {
                self.advance();
                s.clone()
            }
            _ => return Err(SimError::parse(format!("line {}: expected module name", self.peek_line()))),
        };

        let mut param_assigns = std::collections::HashMap::new();
        let mut type_param_assigns = std::collections::HashMap::new();

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
                            _ => return Err(SimError::parse(format!("line {}: expected parameter name", self.peek_line()))),
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
            _ => return Err(SimError::parse(format!("line {}: expected instance name", self.peek_line()))),
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
                            Token::Ident(s) => { self.advance(); s.clone() }
                            _ => return Err(SimError::parse(format!("line {}: expected port name", self.peek_line()))),
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

        if self.peek() != &Token::Semi {
            // If we have trailing tokens (e.g., = new() from misidentified class type), skip them
            self.skip_until_semi_or_end()?;
        } else {
            self.advance();
        }

        Ok(ModuleInstance { module_name, instance_name, range, param_assigns, type_param_assigns, port_conns })
    }

    fn is_type_token(&self) -> bool {
        matches!(self.peek(), Token::Bit | Token::Logic | Token::Int | Token::Integer
            | Token::Byte | Token::Shortint | Token::Longint | Token::Time | Token::Reg
            | Token::Real | Token::RealTime | Token::String
            | Token::Struct | Token::Union | Token::Enum)
    }

    fn parse_dist_item(&mut self) -> Result<DistItem, SimError> {
        // dist item: expr := weight  or  expr :/ weight  or  [lo:hi] := weight  or  [lo:hi] :/ weight
        if self.peek() == &Token::LBrack && self.peek_ahead(1) != &Token::RBrack && self.peek_ahead(1) != &Token::Colon {
            // Range item: [lo:hi] := weight or [lo:hi] :/ weight
            self.advance(); // [
            let lo = self.parse_expr(0)?;
            self.expect(Token::Colon)?;
            let hi = self.parse_expr(0)?;
            self.expect(Token::RBrack)?;
            if self.peek() == &Token::Equiv { // :=
                self.advance();
                let val = self.parse_expr(0)?;
                let weight = const_eval_simple(&val).unwrap_or(0) as u64;
                Ok(DistItem::Range(Box::new(lo), Box::new(hi), DistWeight::Item(weight)))
            } else if matches!(self.peek(), Token::Colon) && self.peek_ahead(1) == &Token::Slash {
                // :/
                self.advance(); // :
                self.advance(); // /
                let val = self.parse_expr(0)?;
                let weight = const_eval_simple(&val).unwrap_or(0) as u64;
                Ok(DistItem::Range(Box::new(lo), Box::new(hi), DistWeight::Range(weight)))
            } else {
                return Err(SimError::parse(format!("line {}: expected := or :/ after dist range", self.peek_line())));
            }
        } else {
            // Single value: expr := weight or expr :/ weight
            let expr = self.parse_expr(0)?;
            if self.peek() == &Token::Equiv { // :=
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
                return Err(SimError::parse(format!("line {}: expected := or :/ after dist item", self.peek_line())));
            }
        }
    }

    fn parse_gate_primitive(&mut self) -> Result<GatePrimitive, SimError> {
        let gate_type = match self.peek() {
            Token::And => { self.advance(); GateType::And }
            Token::Or => { self.advance(); GateType::Or }
            Token::Nand => { self.advance(); GateType::Nand }
            Token::Nor => { self.advance(); GateType::Nor }
            Token::Xor => { self.advance(); GateType::Xor }
            Token::Xnor => { self.advance(); GateType::Xnor }
            Token::Buf => { self.advance(); GateType::Buf }
            Token::NotGate => { self.advance(); GateType::Not }
            _ => return Err(SimError::parse(format!("line {}: expected gate type", self.peek_line()))),
        };
        let instance_name = if self.peek() == &Token::LParen {
            None
        } else {
            let name = match self.peek().clone() {
                Token::Ident(s) => { self.advance(); Some(s) }
                _ => return Err(SimError::parse(format!("line {}: expected gate instance name", self.peek_line()))),
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
            Token::Assert => { self.advance(); "assert" }
            Token::Assume => { self.advance(); "assume" }
            Token::Cover => { self.advance(); "cover" }
            Token::Expect => { self.advance(); "expect" }
            _ => return Err(SimError::parse("expected assert/assume/cover/expect")),
        };

        // Check for concurrent assertion: assert property (...)
        if self.peek() == &Token::Property {
            self.advance();
            self.expect(Token::LParen)?;
            // Parse optional clocking: @(posedge clk)
            if self.peek() == &Token::At {
                self.parse_clocking_event()?;
            }
            // Parse optional disable iff (expr)
            if self.peek() == &Token::Disable {
                self.advance();
                match self.peek() {
                    Token::Ident(s) if s == "iff" => { self.advance(); }
                    _ => return Err(SimError::parse("expected 'iff' after 'disable'")),
                }
                self.expect(Token::LParen)?;
                self.parse_expr(0)?;
                self.expect(Token::RParen)?;
            }
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
                "assert" => Ok(Stmt::Assert { cond, pass_stmt: None, fail_stmt }),
                "assume" => Ok(Stmt::Assume { cond, pass_stmt: None, fail_stmt }),
                "cover" => Ok(Stmt::Cover { cond, pass_stmt: None }),
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
            "assert" => Ok(Stmt::Assert { cond, pass_stmt, fail_stmt }),
            "assume" => Ok(Stmt::Assume { cond, pass_stmt, fail_stmt }),
            "cover" => Ok(Stmt::Cover { cond, pass_stmt }),
            "expect" => Ok(Stmt::Expect { cond, pass_stmt, fail_stmt }),
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
        self.skip_semi();
        let mut coverpoints = Vec::new();
        let mut crosses = Vec::new();
        loop {
            match self.peek() {
                Token::EndGroup | Token::Eof => {
                    self.advance();
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
                                            Token::Bins | Token::IllegalBins | Token::IgnoreBins => {
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
                            let is_new_port = ahead == Token::Input || ahead == Token::Output
                                || ahead == Token::Inout
                                || (matches!(&ahead, Token::Ident(_)) && matches!(self.peek_ahead(2), Token::Scope));
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
                                            bins.push(BinDef { name: bin_name, range_list, bin_type });
                                        }
                                        _ => break,
                                    }
                                    }
                                }
                                self.skip_semi();
                                coverpoints.push(CoverpointDef { name: ident, expr, bins });
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
                                crosses.push(CrossDef { name: ident, coverpoints: cps });
                            }
                            _ => {
                                return Err(SimError::parse(format!("line {}: unexpected token after ':' in covergroup body", self.peek_line())));
                            }
                        }
                    } else {
                        return Err(SimError::parse(format!("line {}: unexpected token after identifier in covergroup body", self.peek_line())));
                    }
                }
                Token::Option_ => {
                    self.advance();
                    self.skip_until_semi_or_end()?;
                }
                _ => {
                    return Err(SimError::parse(format!("line {}: unexpected token in covergroup body: {}", self.peek_line(), self.peek())));
                }
            }
        }
        Ok(CovergroupDecl { name, clocking_event, coverpoints, crosses })
    }

    fn parse_dpi_import(&mut self) -> Result<DpiImport, SimError> {
        self.advance(); // consume "DPI-C" string literal
        let is_task = if self.peek() == &Token::Task {
            self.advance();
            true
        } else if self.peek() == &Token::Function {
            self.advance();
            false
        } else {
            return Err(SimError::parse(format!("line {}: expected 'function' or 'task' after import \"DPI-C\"", self.peek_line())));
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
                args.push(DpiArg { direction, dtype, name: arg_name });
                if self.peek() == &Token::Comma {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        self.expect(Token::RParen)?;
        self.skip_semi();
        Ok(DpiImport { name, return_type, args, is_task })
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
            Token::Byte => { self.advance(); DataType::Byte }
            Token::Shortint => { self.advance(); DataType::Shortint }
            Token::Int => { self.advance(); DataType::Int }
            Token::Longint => { self.advance(); DataType::Longint }
            Token::Integer => { self.advance(); DataType::Integer }
            Token::Real => { self.advance(); DataType::Real }
            Token::RealTime => { self.advance(); DataType::Realtime }
            Token::Bit => { self.advance(); DataType::Bit }
            Token::Logic => { self.advance(); DataType::Logic }
            Token::String => { self.advance(); DataType::String }
            Token::Ident(s) if s == "chandle" => { self.advance(); DataType::Longint }
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
                    _ => Err(SimError::parse(format!("line {}: expected case or if after unique/priority/unique0", self.peek_line()))),
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
                    Token::Ident(s) => { self.advance(); s.clone() }
                    _ => return Err(SimError::parse(format!("line {}: expected identifier after disable", self.peek_line()))),
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
                    _ => return Err(SimError::parse(format!("line {}: expected event name after ->", self.peek_line()))),
                };
                self.skip_semi();
                Ok(Stmt::EventTrigger { name })
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
                            let is_new_port = ahead == Token::Input || ahead == Token::Output
                                || ahead == Token::Inout
                                || (matches!(&ahead, Token::Ident(_)) && matches!(self.peek_ahead(2), Token::Scope));
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
                        Ok(Stmt::BlockingAssign { lhs, rhs, delay: None })
                    }
                    Token::Decrement => {
                        self.advance();
                        self.skip_semi();
                        let rhs = Expr::BinaryOp {
                            op: BinaryOp::Sub,
                            lhs: Box::new(lhs.clone()),
                            rhs: Box::new(Expr::Value(Value::Decimal(1))),
                        };
                        Ok(Stmt::BlockingAssign { lhs, rhs, delay: None })
                    }
                    Token::BlockingAssign => {
                        self.advance();
                        let rhs = self.parse_expr(0)?;
                        self.skip_semi();
                        Ok(Stmt::BlockingAssign { lhs, rhs, delay: None })
                    }
                    Token::NonBlockingAssign => {
                        if is_valid_lvalue(&lhs) {
                            self.advance();
                            let rhs = self.parse_expr(0)?;
                            self.skip_semi();
                            Ok(Stmt::NonBlockingAssign { lhs, rhs, delay: None })
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
                            rhs: Expr::BinaryOp { op: BinaryOp::Add, lhs: Box::new(lhs_copy), rhs: Box::new(rhs) },
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
                            rhs: Expr::BinaryOp { op: BinaryOp::Sub, lhs: Box::new(lhs_copy), rhs: Box::new(rhs) },
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
                            rhs: Expr::BinaryOp { op: BinaryOp::BitXor, lhs: Box::new(lhs_copy), rhs: Box::new(rhs) },
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
            Ok(Stmt::CaseInside { expr, items, default })
        } else if is_casex {
            Ok(Stmt::CaseX { expr, items, default })
        } else if is_casez {
            Ok(Stmt::CaseZ { expr, items, default })
        } else {
            Ok(Stmt::Case { expr, items, default })
        }
    }

    fn parse_for_stmt(&mut self) -> Result<Stmt, SimError> {
        self.advance();
        self.expect(Token::LParen)?;
        let init = if self.peek() != &Token::Semi {
            // Handle variable declaration in for loop init: for (int k = 0; ...)
            if matches!(self.peek(), Token::Int | Token::Integer | Token::Bit | Token::Logic | Token::Reg) {
                self.advance(); // skip type keyword
                if self.peek() == &Token::Signed { self.advance(); }
                if self.peek() == &Token::Unsigned { self.advance(); }
                let var = self.expect_ident()?;
                let init_val = if self.peek() == &Token::BlockingAssign {
                    self.advance();
                    Some(self.parse_expr(0)?)
                } else { None };
                let stmt = if let Some(val) = init_val {
                    Stmt::BlockingAssign { lhs: Expr::Ident(var), rhs: val, delay: None }
                } else { Stmt::Null };
                Some(Box::new(stmt))
            } else {
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
                        lhs: Expr::Ident(var.clone()),
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
                        lhs: Expr::Ident(var.clone()),
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
                        Stmt::BlockingAssign { lhs: expr, rhs, delay: None }
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
        Ok(Stmt::LoopFor { init, cond, step, stmts })
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
        Ok(Stmt::ForeachLoop { array_var, index_vars, stmts })
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
                Token::Join => { self.advance(); return Ok(Stmt::Fork { processes, join_type: JoinType::Join }); }
                Token::JoinAny => { self.advance(); return Ok(Stmt::Fork { processes, join_type: JoinType::JoinAny }); }
                Token::JoinNone => { self.advance(); return Ok(Stmt::Fork { processes, join_type: JoinType::JoinNone }); }
                Token::Eof => return Err(SimError::parse(format!("line {}: unexpected EOF in fork block", self.peek_line()))),
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
                s.clone()
            }
            _ => return Err(SimError::parse(format!("line {}: expected system call name after $", self.peek_line()))),
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

    fn parse_expr(&mut self, min_prec: usize) -> Result<Expr, SimError> {
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
                    // expr inside { list }
                    // Same precedence as relational (7)
                    if 7 < min_prec { break; }
                    self.advance();
                    self.expect(Token::LBrace)?;
                    let mut range_list = Vec::new();
                    if self.peek() != &Token::RBrace {
                        loop {
                            range_list.push(self.parse_expr(0)?);
                            if self.peek() == &Token::Comma {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(Token::RBrace)?;
                    lhs = Expr::Inside {
                        expr: Box::new(lhs),
                        range_list,
                    };
                    continue;
                }
                Token::Ident(ref s) if s == "dist" => {
                    // expr dist { items }
                    // Same precedence as inside (7)
                    if 7 < min_prec { break; }
                    self.advance();
                    self.expect(Token::LBrace)?;
                    let mut items = Vec::new();
                    if self.peek() != &Token::RBrace {
                        loop {
                            let dist_item = self.parse_dist_item()?;
                            items.push(dist_item);
                            if self.peek() == &Token::Comma {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(Token::RBrace)?;
                    lhs = Expr::Dist {
                        expr: Box::new(lhs),
                        items,
                    };
                    continue;
                }
                Token::Ident(ref s) if s == "with" => {
                    // expr.method(args) with (expr)
                    // Same precedence as method call
                    if 6 < min_prec { break; }
                    self.advance();
                    self.expect(Token::LParen)?;
                    let with_expr = self.parse_expr(0)?;
                    self.expect(Token::RParen)?;
                    let old_lhs = std::mem::replace(&mut lhs, Expr::Value(Value::Decimal(0)));
                    match old_lhs {
                        Expr::MethodCall { obj, method, args, with_clause: None } => {
                            lhs = Expr::MethodCall {
                                obj,
                                method,
                                args,
                                with_clause: Some(Box::new(with_expr)),
                            };
                        }
                        _ => {
                            return Err(self.err("'with' clause can only follow a method call"));
                        }
                    }
                    continue;
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
                            with_clause: None,
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

    fn parse_primary_expr(&mut self) -> Result<Expr, SimError> {
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
                    Token::Signed => { self.advance(); "signed".to_string() }
                    Token::Unsigned => { self.advance(); "unsigned".to_string() }
                    _ => return Err(SimError::parse(format!("line {}: expected system function name", self.peek_line()))),
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
                                if self.peek() == &Token::Comma {
                                    self.advance();
                                } else {
                                    break;
                                }
                            }
                        }
                        self.expect(Token::RParen)?;
                        return Ok(Expr::FuncCall { name: format!("{}::{}", name, item), args });
                    }
                    return Ok(Expr::ScopedIdent { package: name.clone(), item });
                }
                // Class#(Type)::method resolution (parameterized class)
                if self.peek() == &Token::Hash {
                    self.advance();
                    self.expect(Token::LParen)?;
                    let mut type_specs = Vec::new();
                    loop {
                        if self.peek() == &Token::RParen { break; }
                        let dt = self.parse_type_expr()?;
                        type_specs.push(dt);
                        if self.peek() == &Token::Comma { self.advance(); } else { break; }
                    }
                    self.expect(Token::RParen)?;
                    let type_str: Vec<String> = type_specs.iter().map(|dt| dt.to_string()).collect();
                    let suffix = type_str.join(",");
                    let class_prefix = if suffix.is_empty() { name.clone() } else { format!("{}#{}", name, suffix) };
                    if self.peek() == &Token::Scope {
                        self.advance();
                        let item = self.expect_ident()?;
                        if self.peek() == &Token::LParen {
                        // Type cast: scoped_type'(expr) like prim_mubi_pkg::mubi4_t'(expr)
                        if self.peek() == &Token::Quote && self.peek_ahead(1) == &Token::LParen {
                            self.advance(); // consume '
                            self.advance(); // consume (
                            let expr = self.parse_expr(0)?;
                            self.expect(Token::RParen)?;
                            return Ok(Expr::Cast {
                                dtype: format!("{}::{}", class_prefix, item),
                                expr: Box::new(expr),
                            });
                        }

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
                            return Ok(Expr::FuncCall { name: format!("{}::{}", class_prefix, item), args });
                        }
                        return Ok(Expr::ScopedIdent { package: class_prefix, item });
                    }
                    return Ok(Expr::Ident(class_prefix));
                }
                // Type cast: type_name'(expr)
                if self.peek() == &Token::Quote {
                    self.advance();
                    self.expect(Token::LParen)?;
                    let expr = self.parse_expr(0)?;
                    self.expect(Token::RParen)?;
                    return Ok(Expr::Cast {
                        dtype: name.clone(),
                        expr: Box::new(expr),
                    });
                }
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
            Token::Number { value, base, width, is_signed } => {
                self.advance();
                // Check for width cast: 22'(expr)
                if self.peek() == &Token::Quote && self.peek_ahead(1) == &Token::LParen {
                    self.advance(); // consume '
                    self.advance(); // consume (
                    let expr = self.parse_expr(0)?;
                    self.expect(Token::RParen)?;
                    let n = value.parse::<i64>().unwrap_or(0);
                    return Ok(Expr::Cast {
                        dtype: format!("{}", n),
                        expr: Box::new(expr),
                    });
                }
                let val = if let Some(base) = base {
                    match base {
                        2 => Expr::Value(Value::Binary {
                            bits: value.clone(),
                            width: *width,
                            is_signed: *is_signed,
                        }),
                        8 => Expr::Value(Value::Octal {
                            bits: value.clone(),
                            width: *width,
                            is_signed: *is_signed,
                        }),
                        10 => {
                            let n = value.parse::<i64>().unwrap_or(0);
                            Expr::Value(Value::Decimal(n))
                        }
                        16 => Expr::Value(Value::Hex {
                            bits: value.clone(),
                            width: *width,
                            is_signed: *is_signed,
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
                if self.peek() == &Token::LBrack {
                    // new[size] or new[size](init) — dynamic array allocation
                    self.advance();
                    let size = self.parse_expr(0)?;
                    self.expect(Token::RBrack)?;
                    let _init = if self.peek() == &Token::LParen {
                        self.advance();
                        let val = self.parse_expr(0)?;
                        self.expect(Token::RParen)?;
                        Some(Box::new(val))
                    } else {
                        None
                    };
                    Ok(Expr::FuncCall { name: "new".to_string(), args: vec![size] })
                } else {
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
                let expr = self.parse_expr(12)?;
                Ok(Expr::UnaryOp { op, expr: Box::new(expr) })
            }
            Token::Not => {
                self.advance();
                let expr = self.parse_expr(12)?;
                Ok(Expr::UnaryOp {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                })
            }
            Token::LBrace => {
                self.advance();
                // Check for streaming operator: {<<N{expr}} or {>>N{expr}}
                if matches!(self.peek(), Token::Shl | Token::Shr | Token::Sshl | Token::Sshr) {
                    let op = if matches!(self.peek(), Token::Shl | Token::Sshl) {
                        String::from("<<")
                    } else {
                        String::from(">>")
                    };
                    self.advance();
                    let slice_size = if !matches!(self.peek(), Token::LBrace) {
                        Some(Box::new(self.parse_expr(0)?))
                    } else {
                        None
                    };
                    self.expect(Token::LBrace)?;
                    let mut slices = Vec::new();
                    loop {
                        if self.peek() == &Token::RBrace { break; }
                        let item = self.parse_expr(0)?;
                        slices.push(item);
                        if self.peek() == &Token::Comma {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.expect(Token::RBrace)?;
                    self.expect(Token::RBrace)?;
                    return Ok(Expr::StreamingConcat { op, slice_size, slices });
                }
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
            Token::Quote => {
                self.advance();
                if self.peek() == &Token::LBrace {
                    // '{ ... } pattern — consume braces
                    self.advance();
                    let mut depth = 1usize;
                    while depth > 0 && self.peek() != &Token::Eof {
                        match self.peek() {
                            Token::LBrace => { depth += 1; self.advance(); }
                            Token::RBrace => { depth -= 1; if depth > 0 { self.advance(); } }
                            _ => { self.advance(); }
                        }
                    }
                    if self.peek() == &Token::RBrace { self.advance(); }
                }
                Ok(Expr::FillLit(crate::ir::LogicVal::Zero))
            }
            // Prefix ++/-- (treated as (expr+1) at elaboration time)
            Token::Increment => {
                self.advance();
                let expr = self.parse_expr(12)?;
                Ok(Expr::BinaryOp {
                    op: BinaryOp::Add,
                    lhs: Box::new(expr),
                    rhs: Box::new(Expr::Value(Value::Decimal(1))),
                })
            }
            Token::Decrement => {
                self.advance();
                let expr = self.parse_expr(12)?;
                Ok(Expr::BinaryOp {
                    op: BinaryOp::Sub,
                    lhs: Box::new(expr),
                    rhs: Box::new(Expr::Value(Value::Decimal(1))),
                })
            }
            // Type cast: int'(expr), logic'(expr), bit'(expr), void'(expr), etc.
            Token::Void | Token::Int | Token::Integer | Token::Logic | Token::Bit
                | Token::Byte | Token::Shortint | Token::Longint | Token::Time | Token::Signed
                | Token::Real | Token::RealTime => {
                self.advance(); // consume the type keyword
                let type_name = match &tok {
                    Token::Void => "void",
                    Token::Int => "int",
                    Token::Integer => "integer",
                    Token::Logic => "logic",
                    Token::Bit => "bit",
                    Token::Byte => "byte",
                    Token::Shortint => "shortint",
                    Token::Longint => "longint",
                    Token::Time => "time",
                    Token::Signed => "signed",
                    Token::Real => "real",
                    Token::RealTime => "realtime",
                    _ => unreachable!(),
                };
                if self.peek() == &Token::Quote {
                    self.advance();
                    self.expect(Token::LParen)?;
                    let expr = self.parse_expr(0)?;
                    self.expect(Token::RParen)?;
                    Ok(Expr::Cast {
                        dtype: type_name.to_string(),
                        expr: Box::new(expr),
                    })
                } else {
                    // Just return as identifier
                    Ok(Expr::Ident(type_name.to_string()))
                }
            }
            _ => Err(SimError::parse(format!("line {}: expected expression, found {}", self.peek_line(), tok))),
        }
    }
}
