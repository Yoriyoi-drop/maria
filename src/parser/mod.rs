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
use crate::diagnostics::diagnostic::{DiagCode, DiagLevel, Diagnostic, SourceSnippet};
use crate::error::SimError;
use crate::intern::Symbol;
use crate::parser::lexer::*;

pub struct Parser {
    tokens: Vec<(Token, usize, usize)>,
    pos: std::cell::Cell<usize>,
    source_file: String,
    source_lines: Vec<String>,
    class_names: std::collections::HashSet<Symbol>,
    typedef_names: std::collections::HashSet<Symbol>,
    /// Nama class GLOBAL (lintas file) dari discovery pass CompileSession.
    /// Disimpan terpisah dari `class_names` (yang di-clear di parse_design)
    /// agar tetap tersedia saat module/interface body di-parse — tanpanya
    /// `ClassType var;` dari file lain (class UVM dsb.) salah di-parse sebagai
    /// instance module (`type inst;`).
    global_class_names: std::collections::HashSet<Symbol>,
    /// Nama typedef GLOBAL (lintas file) — simetris dengan global_class_names.
    global_typedef_names: std::collections::HashSet<Symbol>,
    /// Type parameter names di scope module (dari `parameter type T = ...`),
    /// agar `T x;` diparse sebagai deklarasi, bukan instance.
    module_type_params: std::collections::HashSet<Symbol>,
    package_tdefs: std::collections::HashMap<Symbol, Vec<Symbol>>,
    type_param_names: Vec<Symbol>,
    file_line_map: Vec<(usize, String)>,
    recursion_depth: usize,
    /// Safety counter: total tokens consumed. Reset per `parse_design()` call.
    /// If this exceeds MAX_PARSE_STEPS, parsing aborts to prevent infinite loops.
    parse_steps: usize,
    /// Safety counter: consecutive peek() calls without advance().
    /// Catches loops that check peek() without calling advance().
    /// Uses Cell for interior mutability (peek() takes &self).
    peek_count: std::cell::Cell<usize>,
    pub errors: Vec<Diagnostic>,
}

impl Parser {
    /// Batas advance per token (budget amat luas untuk backtracking normal,
    /// yang memanggil advance berkali-kali per token). Dihitung dinamis dari
    /// ukuran input agar desain besar (ribuan module / jutaan token) tidak
    /// kena limit statis; anti infinite-loop tetap efektif karena budget per
    /// token yang wajar jauh di bawah ini.
    fn step_limit(&self) -> usize {
        self.tokens.len().saturating_mul(256).max(10_000_000)
    }

    /// Batas panggilan peek per parse_design. peek() dipanggil beberapa kali
    /// per token (lookahead); 64× token adalah budget luas. Di-reset setiap
    /// `parse_design()` agar counter tidak akumulatif lintas pemanggilan.
    fn peek_limit(&self) -> usize {
        self.tokens.len().saturating_mul(64).max(1_000_000)
    }

    pub fn new(tokens: Vec<(Token, usize, usize)>, source_file: &str) -> Self {
        Self {
            tokens,
            pos: std::cell::Cell::new(0),
            source_file: source_file.to_string(),
            source_lines: Vec::new(),
            class_names: {
                let mut s = std::collections::HashSet::new();
                s.insert(Symbol::intern("process"));
                s.insert(Symbol::intern("uvm_object"));
                s.insert(Symbol::intern("uvm_component"));
                s.insert(Symbol::intern("uvm_sequence_item"));
                s.insert(Symbol::intern("uvm_sequence"));
                s.insert(Symbol::intern("uvm_sequencer"));
                s.insert(Symbol::intern("uvm_driver"));
                s.insert(Symbol::intern("uvm_monitor"));
                s.insert(Symbol::intern("uvm_scoreboard"));
                s.insert(Symbol::intern("uvm_analysis_port"));
                s.insert(Symbol::intern("uvm_analysis_imp"));
                s.insert(Symbol::intern("uvm_test"));
                s.insert(Symbol::intern("uvm_config_db"));
                s.insert(Symbol::intern("uvm_report_object"));
                s.insert(Symbol::intern("uvm_factory"));
                s.insert(Symbol::intern("uvm_resource_db"));
                s
            },
            typedef_names: std::collections::HashSet::new(),
            global_class_names: std::collections::HashSet::new(),
            global_typedef_names: std::collections::HashSet::new(),
            module_type_params: std::collections::HashSet::new(),
            package_tdefs: std::collections::HashMap::new(),
            type_param_names: Vec::new(),
            file_line_map: Vec::new(),
            recursion_depth: 0,
            parse_steps: 0,
            peek_count: std::cell::Cell::new(0),
            errors: Vec::new(),
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

    /// Tambahkan nama class & typedef GLOBAL (lintas file) yang ditemukan lewat
    /// discovery pass di CompileSession. Parsing per-file hanya tahu nama di
    /// file-nya sendiri; tanpa ini, `class_name var;` dari file lain akan
    /// disalahartikan sebagai instansiasi module (`type inst;`).
    pub fn with_global_type_names(
        mut self,
        classes: &std::collections::HashSet<Symbol>,
        typedefs: &std::collections::HashSet<Symbol>,
    ) -> Self {
        // Simpan salinan global di field terpisah: `parse_design()` me-reset
        // class_names (dan parse_module/parse_interface me-reset typedef_names),
        // jadi kalau hanya di-extend ke set aktif, nama global akan hilang
        // sebelum body module/interface diparse → `ClassType var;` lintas file
        // salah di-parse sebagai instance module.
        self.global_class_names = classes.iter().copied().collect();
        self.global_typedef_names = typedefs.iter().copied().collect();
        self.class_names.extend(classes.iter().copied());
        self.typedef_names.extend(typedefs.iter().copied());
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
        let pc = self.peek_count.get() + 1;
        self.peek_count.set(pc);
        // Guard: if peek() called excessively without advance(), force EOF
        // This catches infinite loops that peek() without calling advance()
        if pc > self.peek_limit() {
            self.pos.set(self.tokens.len());
            return &Token::Eof;
        }
        if self.pos.get() >= self.tokens.len() {
            return &Token::Eof;
        }
        &self.tokens[self.pos.get()].0
    }

    fn peek_line(&self) -> usize {
        if self.pos.get() >= self.tokens.len() {
            return 0;
        }
        self.tokens[self.pos.get()].1
    }

    fn peek_col(&self) -> usize {
        if self.pos.get() >= self.tokens.len() {
            return 0;
        }
        self.tokens[self.pos.get()].2
    }

    fn err(&self, msg: impl Into<String>) -> SimError {
        let msg_str = msg.into();
        let cumulative_line = self.peek_line();
        let col = self.peek_col();
        let (display_file, display_line) = self.resolve_source_file(cumulative_line);

        // Tentukan DiagCode berdasarkan pesan error
        let code = if msg_str.contains("unexpected token") || msg_str.contains("Unexpected") {
            DiagCode::UnexpectedToken
        } else if msg_str.contains("expected") && msg_str.contains("';'") {
            DiagCode::ExpectedSemi
        } else if msg_str.contains("expected") {
            DiagCode::ExpectedToken
        } else if msg_str.contains("unclosed") || msg_str.contains("unterminated") || msg_str.contains("unexpected EOF") {
            DiagCode::UnclosedBlock
        } else {
            DiagCode::InvalidSyntax
        };

        // Buat source snippet jika ada source line
        let source_line = if cumulative_line > 0 && cumulative_line <= self.source_lines.len() {
            Some(self.source_lines[cumulative_line - 1].clone())
        } else {
            None
        };

        let msg_for_diag = msg_str.clone();
        let mut diag = Diagnostic::error(code, msg_for_diag);

        if let Some(sl) = source_line {
            let snippet = SourceSnippet::new(&display_file, display_line, col, sl.trim_end());
            diag = diag.with_source_snippet(snippet);
        }

        // Jika ada source lines, gunakan Diagnostic variant
        if !self.source_lines.is_empty() {
            SimError::from_parse_diagnostic(diag)
        } else {
            // Fallback: tetap pakai flat string
            SimError::parse(format!(
                "{}:{}:{}: {}",
                display_file, display_line, col, msg_str
            ))
        }
    }

    fn push_warning_at(&mut self, msg: impl Into<String>, line: usize, col: usize) {
        let msg: String = msg.into();
        let mut diag = Diagnostic::new(DiagLevel::Warning, DiagCode::InvalidSyntax, msg)
            .with_code_context();
        if line > 0 && line <= self.source_lines.len() {
            let source_line = &self.source_lines[line - 1];
            let snippet = SourceSnippet::new(&self.source_file, line, col, source_line.trim_end());
            diag = diag.with_source_snippet(snippet);
        }
        self.errors.push(diag);
    }

    fn peek_ahead(&self, n: usize) -> &Token {
        if self.tokens.is_empty() {
            return &Token::Eof;
        }
        let idx = (self.pos.get() + n).min(self.tokens.len() - 1);
        &self.tokens[idx].0
    }

    fn advance(&mut self) {
        self.pos.set(self.pos.get() + 1);
        self.parse_steps += 1;
        // Safety guard: if step limit exceeded, force EOF to break
        // out of any sub-parser infinite loop (parse_class, parse_module, etc.)
        if self.parse_steps > self.step_limit() {
            self.pos.set(self.tokens.len());
        }
    }

    fn expect(&mut self, expected: Token) -> Result<(), SimError> {
        if self.peek() == &expected {
            self.advance();
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
            _ => Err(self.err(format!(
                "expected identifier, found {}",
                self.peek()
            ))),
        }
    }

    pub fn parse_design(&mut self) -> Result<Design, SimError> {
        self.class_names.clear();
        self.class_names.insert(Symbol::intern("process"));
        self.class_names.insert(Symbol::intern("uvm_object"));
        self.class_names.insert(Symbol::intern("uvm_component"));
        self.class_names.insert(Symbol::intern("uvm_sequence_item"));
        self.class_names.insert(Symbol::intern("uvm_sequence"));
        self.class_names.insert(Symbol::intern("uvm_sequencer"));
        self.class_names.insert(Symbol::intern("uvm_driver"));
        self.class_names.insert(Symbol::intern("uvm_monitor"));
        self.class_names.insert(Symbol::intern("uvm_scoreboard"));
        self.class_names.insert(Symbol::intern("uvm_analysis_port"));
        self.class_names.insert(Symbol::intern("uvm_analysis_imp"));
        self.class_names.insert(Symbol::intern("uvm_test"));
        self.class_names.insert(Symbol::intern("uvm_config_db"));
        self.class_names.insert(Symbol::intern("uvm_report_object"));
        self.class_names.insert(Symbol::intern("uvm_factory"));
        self.class_names.insert(Symbol::intern("uvm_resource_db"));
        // Re-seed nama class & typedef GLOBAL (lintas file) yang di-clear di
        // atas. Tanpa ini, `ClassType var;` dari file lain (mis. class UVM di
        // file terpisah) salah di-parse sebagai instance module.
        self.class_names.extend(self.global_class_names.iter().copied());
        self.typedef_names.extend(self.global_typedef_names.iter().copied());
        let mut modules = Vec::with_capacity(64);
        let mut classes = Vec::with_capacity(32);
        let mut packages = Vec::with_capacity(16);
        let mut interfaces = Vec::with_capacity(16);
        let mut unit_imports = Vec::new();
        let mut unit_funcs: Vec<FunctionDecl> = Vec::new();
        let mut unit_tasks: Vec<TaskDecl> = Vec::new();
        let mut unit_typedefs: Vec<TypedefDecl> = Vec::new();
        let mut unit_params: Vec<ParamDecl> = Vec::new();
        let mut binds = Vec::new();
        let mut clocking_blocks = Vec::new();
        let mut configs = Vec::new();
        let mut udp_defs = Vec::new();
        // Reset safety counters for this parse_design() call
        self.parse_steps = 0;
        self.peek_count.set(0);
        // First pass: collect all class names — with error recovery
        // If parsing fails, error is saved and we skip to next construct
let saved_pos = self.pos.get();
        let mut _last_pos = self.pos.get();
        let mut _stuck = 0u32;
        while self.peek() != &Token::Eof {
            // Step limit guard: prevent infinite loops
            if self.parse_steps > self.step_limit() {
                return Err(self.err("parser exceeded maximum step limit (possible infinite loop)"));
            }
            // Stuck detection: if pos hasn't changed for too many iterations, abort
            if self.pos.get() == _last_pos {
                _stuck += 1;
                if _stuck > 10_000 {
                    let line = self.peek_line();
                    let col = self.peek_col();
                    self.push_warning_at("parser stuck (no progress) during class discovery pass — skipping".to_string(), line, col);
                    break;
                }
            } else {
                _stuck = 0;
                _last_pos = self.pos.get();
            }
            if self.peek() == &Token::Class {
                match self.parse_class_fast() {
                    Ok(_) => {}
                    Err(e) => { self.errors.push(e.to_diagnostic()); self.skip_to_next_top_level(); continue; }
                }
            } else if self.peek() == &Token::Module {
                match self.parse_module_fast() {
                    Ok(_) => {}
                    Err(e) => { self.errors.push(e.to_diagnostic()); self.skip_to_next_top_level(); continue; }
                }
            } else if self.peek() == &Token::Interface {
                // skip interface in first pass (no class deps needed)
                match self.parse_interface_fast() {
                    Ok(_) => {}
                    Err(e) => { self.errors.push(e.to_diagnostic()); self.skip_to_next_top_level(); continue; }
                }
            } else if self.peek() == &Token::Program {
                // skip program in first pass
                match self.parse_program_fast() {
                    Ok(_) => {}
                    Err(e) => { self.errors.push(e.to_diagnostic()); self.skip_to_next_top_level(); continue; }
                }
            } else if self.peek() == &Token::Package {
                // Skip package in first pass — full parse only in pass 2
                self.advance(); // consume 'package'
                let _ = self.expect_ident();
                // Skip to endpackage or EOF
                while self.peek() != &Token::EndPackage && self.peek() != &Token::Eof {
                    self.advance();
                }
                if self.peek() == &Token::EndPackage {
                    self.advance();
                    // Consume optional 'endpackage : name'
                    if self.peek() == &Token::Colon {
                        self.advance();
                        if matches!(self.peek(), Token::Ident(_)) {
                            self.advance();
                        }
                    }
                }
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
                self.advance(); // consume virtual
                match self.parse_class_fast() {
                    Ok(_) => {}
                    Err(e) => { self.errors.push(e.to_diagnostic()); self.skip_to_next_top_level(); continue; }
                }
            } else if self.peek() == &Token::Covergroup {
                // Skip covergroup in first pass — collect name
                let cg = match self.parse_covergroup() {
                    Ok(cg) => cg,
                    Err(e) => { self.errors.push(e.to_diagnostic()); self.skip_to_next_top_level(); continue; }
                };
                self.class_names.insert(cg.name);
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
            } else if self.peek() == &Token::New {
                // 'new' at top level — likely from partially parsed class body
                self.advance(); // consume 'new'
                // Skip balanced parens for new( ... )
                if self.peek() == &Token::LParen {
                    let _ = self.skip_balanced_paren_light();
                }
            } else {
                let line = self.peek_line();
                let col = self.peek_col();
                let tok = format!("{}", self.peek());
                let (_, _) = self.resolve_source_file(line);
                let summary = if tok.len() > 40 { format!("{}...", &tok[..40]) } else { tok };

                self.push_warning_at(format!("skipping top-level construct: {}", summary), line, col);
                self.skip_to_next_top_level();
            }
        }
// First pass done in {:?} — class_names={}, typedef_names={}
        self.pos.set(saved_pos);
        modules.clear();
        classes.clear();
        // Second pass: full parse with class names known — with error recovery
        // Jika parsing modul/class gagal, error disimpan dan lanjut ke konstruk berikutnya
let mut _last_pos = self.pos.get();
        let mut _stuck = 0u32;
        let mut _n_module = 0u32;
        let mut _n_interface = 0u32;
        let mut _n_class = 0u32;
        let mut _n_package = 0u32;
        while self.peek() != &Token::Eof {
            // Step limit guard: prevent infinite loops
            if self.parse_steps > self.step_limit() {
                return Err(self.err("parser exceeded maximum step limit (possible infinite loop)"));
            }
            // Stuck detection: if pos hasn't changed for too many iterations, abort
            if self.pos.get() == _last_pos {
                _stuck += 1;
                if _stuck > 10_000 {
                    let line = self.peek_line();
                    let col = self.peek_col();
                    self.push_warning_at("parser stuck (no progress) during second pass — skipping".to_string(), line, col);
                    break;
                }
            } else {
                _stuck = 0;
                _last_pos = self.pos.get();
            }
             let had_error = match self.peek() {
                Token::Module => {
                    _n_module += 1;
                    match self.parse_module() {
                        Ok(m) => { modules.push(m); false }
                        Err(e) => { self.errors.push(e.to_diagnostic()); true }
                    }
                },
                Token::Interface => {
                    _n_interface += 1;
                    match self.parse_interface() {
                        Ok(iface) => { interfaces.push(iface); false }
                        Err(e) => { self.errors.push(e.to_diagnostic()); true }
                    }
                },
                Token::Class => {
                    _n_class += 1;
                    match self.parse_class() {
                        Ok(c) => { classes.push(c); false }
                        Err(e) => { self.errors.push(e.to_diagnostic()); true }
                    }
                },
                Token::Package => {
                    _n_package += 1;
                    match self.parse_package_decl() {
                        Ok(p) => { packages.push(p); false }
                        Err(e) => { self.errors.push(e.to_diagnostic()); true }
                    }
                },
                Token::Program => match self.parse_module() {
                    Ok(m) => { modules.push(m); false }
                    Err(e) => { self.errors.push(e.to_diagnostic()); true }
                },
                Token::Import => {
                    self.advance();
                    let pkg = match self.expect_ident() {
                        Ok(p) => p,
                        Err(e) => { self.errors.push(e.to_diagnostic()); self.skip_to_next_top_level(); continue; }
                    };
                    if self.peek() != &Token::Scope {
                        let err = self.err("expected '::' after package name");
                        self.errors.push(err.to_diagnostic());
                        self.skip_to_next_top_level();
                        continue;
                    }
                    self.advance(); // consume ::
                    let item = if self.peek() == &Token::Star {
                        self.advance();
                        Symbol::intern("*")
                    } else {
                        match self.expect_ident() {
                            Ok(id) => id,
                            Err(e) => { self.errors.push(e.to_diagnostic()); self.skip_to_next_top_level(); continue; }
                        }
                    };
                    self.skip_semi();
                    unit_imports.push((pkg, item));
                    false
                }
                Token::LParen if self.peek_ahead(1) == &Token::Star => {
                    self.skip_attribute();
                    false
                }
                Token::Virtual if self.peek_ahead(1) == &Token::Class => {
                    self.advance(); // consume 'virtual' so parse_class() sees 'class'
                    match self.parse_class() {
                        Ok(c) => { classes.push(c); false }
                        Err(e) => { self.errors.push(e.to_diagnostic()); true }
                    }
                }
                Token::Covergroup => {
                    match self.parse_covergroup() {
                        Ok(cg) => {
                            if let Some(m) = modules.first_mut() {
                                m.items.push(ModuleItem::Covergroup(cg));
                            }
                            false
                        }
                        Err(e) => { self.errors.push(e.to_diagnostic()); true }
                    }
                }
                Token::Bind => {
                    self.advance(); // consume 'bind'
                    let target = match self.expect_ident() {
                        Ok(t) => t,
                        Err(e) => { self.errors.push(e.to_diagnostic()); self.skip_to_next_top_level(); continue; }
                    };
                    match self.parse_instance() {
                        Ok(instance) => {
                            binds.push(BindDecl { target, instance });
                            false
                        }
                        Err(e) => { self.errors.push(e.to_diagnostic()); true }
                    }
                }
                Token::Clocking => {
                    match self.parse_clocking_block() {
                        Ok(cb) => { clocking_blocks.push(cb); false }
                        Err(e) => { self.errors.push(e.to_diagnostic()); true }
                    }
                }
                Token::Export => {
                    self.advance();
                    if self.peek() == &Token::StringLit(Symbol::intern("DPI-C"))
                        || self.peek() == &Token::StringLit(Symbol::intern("DPI"))
                    {
                        match self.parse_dpi_import() {
                            Ok(_) => false,
                            Err(e) => { self.errors.push(e.to_diagnostic()); true }
                        }
                    } else {
                        match self.skip_until_semi_or_end() {
                            Ok(_) => false,
                            Err(e) => { self.errors.push(e.to_diagnostic()); true }
                        }
                    }
                }
                Token::Config => {
                    match self.parse_config_decl() {
                        Ok(cfg) => { configs.push(cfg); false }
                        Err(e) => { self.errors.push(e.to_diagnostic()); true }
                    }
                }
                Token::Primitive => {
                    match self.parse_udp_declaration() {
                        Ok(udp) => { udp_defs.push(udp); false }
                        Err(e) => { self.errors.push(e.to_diagnostic()); true }
                    }
                }
                Token::Function => {
                    match self.parse_function(false) {
                        Ok(func) => { unit_funcs.push(func); false }
                        Err(e) => { self.errors.push(e.to_diagnostic()); true }
                    }
                }
                Token::Task => {
                    match self.parse_task(false) {
                        Ok(task) => { unit_tasks.push(task); false }
                        Err(e) => { self.errors.push(e.to_diagnostic()); true }
                    }
                }
                Token::Typedef => {
                    match self.parse_typedef() {
                        Ok(td) => { unit_typedefs.push(td); false }
                        Err(e) => { self.errors.push(e.to_diagnostic()); true }
                    }
                }
                Token::Parameter | Token::LocalParam => {
                    let is_local = self.peek() == &Token::LocalParam;
                    self.advance();
                    let mut params = Vec::new();
                    match self.parse_param_list(&mut params) {
                        Ok(_) => {
                            for p in params {
                                if !is_local {
                                    unit_params.push(p);
                                }
                            }
                            false
                        }
                        Err(e) => { self.errors.push(e.to_diagnostic()); true }
                    }
                }
                Token::New => {
                    // 'new' at top level — skip past it and balanced parens
                    self.advance();
                    if self.peek() == &Token::LParen {
                        let _ = self.skip_balanced_paren_light();
                    }
                    false
                }
                Token::RBrace => {
                    // Stray '}' at top level — skip it
                    self.advance();
                    false
                }
                _ => {
                    // Error recovery: cek apakah ini deklarasi di luar module
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
                            | Token::Longint                        | Token::Time
                        | Token::Real
                        | Token::WReal
                        | Token::RealTime
                        | Token::String
                        | Token::Enum
                        | Token::Struct
                        | Token::Union
                    ) {
                        let err = self.err("declaration outside of module");
                        self.errors.push(err.to_diagnostic());
                        let _ = self.skip_until_semi_or_end();
                    } else {
                        let line = self.peek_line();
                        let col = self.peek_col();
                        let tok = self.peek().clone();
                        let tok_str = format!("{}", tok);
                        let (_, _) = self.resolve_source_file(line);
                        let summary = if tok_str.len() > 40 {
                            format!("{}...", &tok_str[..40])
                        } else {
                            tok_str
                        };

                        self.push_warning_at(format!("skipping top-level construct: {}", summary), line, col);
                        self.skip_to_next_top_level();
                    }
                    false
                }
            };
            // Skip past error boundary jika error terjadi
            if had_error {
                self.skip_to_next_top_level();
            }
        }
// Second pass done in {:?}
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
        self.push_depth()?;
        // Guard: if the token is `=`, skip to semi/end to avoid infinite loop
        if matches!(
            self.peek(),
            Token::BlockingAssign | Token::NonBlockingAssign
        ) {
            self.skip_until_semi_or_end()?;
            self.pop_depth();
            return Ok(None);
        }
        // Skip (* ... *) attribute annotations before module items
        if self.peek() == &Token::LParen && self.peek_ahead(1) == &Token::Star {
            self.skip_attribute();
            let result = self.parse_module_item();
            self.pop_depth();
            return result;
        }
        let result = self.parse_module_item_body();
        self.pop_depth();
        result
    }

    fn parse_module_item_body(&mut self) -> Result<Option<ModuleItem>, SimError> {
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
                        | Token::WReal
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
                        array_size_expr: None,
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
            | Token::WReal
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
                if std::env::var("MARIA_DEBUG_PARSE").is_ok()
                    && name.as_str() == "my_class"
                {
                    eprintln!(
                        "[DBG-PARSE] decision: name={} in_class={} in_typedef={} in_mod_type_param={} ahead1={}",
                        name,
                        self.class_names.contains(name),
                        self.typedef_names.contains(name),
                        self.module_type_params.contains(name),
                        format!("{}", self.peek_ahead(1))
                    );
                }
                // `chandle` adalah built-in SV type untuk C pointer (DPI).
                // Tidak ada Token::Chandle sehingga harus dicek di sini sebelum
                // mencoba parse sebagai instance.
                if *name == Symbol::intern("chandle") {
                    let decl = self.parse_decl()?;
                    return Ok(Some(ModuleItem::Decl(decl)));
                }
                if self.class_names.contains(name)
                    || self.typedef_names.contains(name)
                    || self.module_type_params.contains(name)
                {
                    let dtype = DataType::UserDefined(*name);
                    // User-defined type with array dimension: `Type [1:0] varname`
                    // atau `Type [N] nama, nama2`. parse_decl menangani unpacked
                    // array dgn benar; branch manual di bawah hanya untuk polos.
                    let ahead_is_lbrack = matches!(self.peek_ahead(1), Token::LBrack);
                    if ahead_is_lbrack {
                        let decl = self.parse_decl();
                        if let Ok(decl) = decl {
                            return Ok(Some(ModuleItem::Decl(decl)));
                        }
                    }
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
                            let vname = *n;
                            self.advance();
                            names.push(DeclVar {
                                name: vname,
                                range: None,
                                expr_range: None,
                                array_range: None,
                                array_size_expr: None,
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
                } else if self.peek_ahead(1) == &Token::Scope {
                    // Scoped type declaration: `pkg::type varname;` — parse_decl
                    // sudah mendukung `Ident :: Ident`. Contoh nyata OpenTitan:
                    // `tlul_pkg::tl_d2h_t tl_o_pre;` di body module.
                    let decl = self.parse_decl()?;
                    Ok(Some(ModuleItem::Decl(decl)))
                } else if matches!(self.peek_ahead(1), Token::Ident(_))
                    || self.peek_ahead(1) == &Token::Hash
                    || self.peek_ahead(1) == &Token::LParen
                    || self.peek_ahead(1) == &Token::LBrack
                {
                    // Check if Ident + [range] is a declaration (type [msb:lsb] name) or instance
                    if self.peek_ahead(1) == &Token::LBrack {
                        let decl = self.parse_decl();
                        if let Ok(decl) = decl { return Ok(Some(ModuleItem::Decl(decl))) }
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
                        array_size_expr: None,
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
                let line = self.peek_line();
                let col = self.peek_col();
                let tok = self.peek().clone();
                let tok_str = format!("{}", tok);
                let (_, _) = self.resolve_source_file(line);
                let summary = if tok_str.len() > 40 {
                    format!("{}...", &tok_str[..40])
                } else {
                    tok_str
                };
self.push_warning_at(format!("skipping unknown construct: {}", summary), line, col);
                self.skip_until_semi_or_end()?;
                Ok(None)
            }
            }
            Token::Sequence => {
                self.advance();
                while self.peek() != &Token::EndSequence && self.peek() != &Token::Eof {
                    self.advance();
                }
                if self.peek() == &Token::EndSequence {
                    self.advance();
                }
                Ok(None)
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
                // Handle 'parameter type T = type_expr'
                if self.peek() == &Token::Type {
                    self.advance();
                    let pname = self.expect_ident()?;
                    let type_default = if self.peek() == &Token::BlockingAssign {
                        self.advance();
                        Some(self.parse_type_expr()?)
                    } else {
                        None
                    };
                    // Daftarkan sebagai type param module agar `T x;` diparse
                    // sebagai deklarasi (UserDefined), bukan instance `T x(...)`.
                    self.module_type_params.insert(pname);
                    self.skip_semi();
                    return Ok(Some(ModuleItem::Param(ParamDecl {
                        name: pname,
                        dtype: None,
                        range: None,
                        default: None,
                        is_localparam,
                        is_type_param: true,
                        type_default,
                    })));
                }
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
                    Token::Byte => {
                        self.advance();
                        dtype = Some(DataType::Byte);
                    }
                    Token::Shortint => {
                        self.advance();
                        dtype = Some(DataType::Shortint);
                    }
                    Token::Longint => {
                        self.advance();
                        dtype = Some(DataType::Longint);
                    }
                    Token::Time => {
                        self.advance();
                        dtype = Some(DataType::Time);
                    }
                    _ => {}
                }
                // User-defined type: ident followed by name, range, or :: (e.g. pkg::type)
                if dtype.is_none() {
                    if let Token::Ident(s) = self.peek() {
                        let ahead = self.peek_ahead(1).clone();
                        if matches!(ahead, Token::Ident(_) | Token::LBrack | Token::Scope) {
                            let s_owned = *s;
                            self.advance();
                            let mut type_name = s_owned;
                            if self.peek() == &Token::Scope {
                                self.advance();
                                let t = self.expect_ident()?;
                                type_name = Symbol::intern(&format!("{}::{}", s_owned, t));
                            }
                            dtype = Some(DataType::UserDefined(type_name));
                        }
                    }
                }
                if self.peek() == &Token::Signed {
                    self.advance();
                    if dtype.is_none() {
                        dtype = Some(DataType::Signed(Box::new(DataType::Int)));
                    }
                }
                if self.peek() == &Token::Unsigned {
                    self.advance();
                    if dtype.is_none() {
                        dtype = Some(DataType::Int);
                    }
                }
                let mut range = None;
                // Packed dimensi bertingkat: `[NumCnt-1:0][Width-1:0] name = ...`.
                // Sebelumnya hanya 1 dim yang dikonsumsi → sisa `[Width-1:0]`
                // membuat `name` tak ter-parse → "signal 'name' not found".
                // Konsumsi semua dim berturut-turut; `range` = dim terakhir
                // (lebar elemen skalar, dipakai resolusi width param/array).
                while self.peek() == &Token::LBrack {
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
                            *s
                        }
                        _ => break,
                    };
                    // Skip unpacked array dimension(s) after name:
                    // name [N] atau name [msb:lsb] (multi-dimensi diperbolehkan)
                    while self.peek() == &Token::LBrack {
                        self.advance();
                        self.parse_expr(0)?;
                        if self.peek() == &Token::Colon {
                            self.advance();
                            self.parse_expr(0)?;
                        }
                        self.expect(Token::RBrack)?;
                    }
                    let default = if self.peek() == &Token::BlockingAssign {
                        self.advance();
                        Some(self.parse_expr(0)?)
                    } else {
                        None
                    };
            params.push(ParamDecl {
                name,
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
                            params.into_iter().map(ModuleItem::Param).collect(),
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
                self.typedef_names.insert(td.name);
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
                            self.typedef_names.insert(*name);
                        }
                    } else if tdefs.contains(&item) {
                        self.typedef_names.insert(item);
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
            Token::Class => {
                // Class inside module — skip entire class body to endclass
                self.skip_class_body();
                Ok(None)
            }
            Token::EndClass => {
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    /// Skip tokens until the next top-level construct (module/class/interface/package/etc.).
    /// First advances past the current token (so we don't get stuck on the same construct).
    /// 
    /// Penting: Function/Task ikut dilacak depth-nya agar error recovery tidak premature return
    /// saat berada di dalam body function/task.
    fn skip_to_next_top_level(&mut self) {
        let mut depth: i32 = 0;
        // Advance past current token first to avoid infinite loop
        self.advance();
        let mut _last_pos = self.pos.get();
        let mut _stuck = 0u32;
        loop {
            if self.pos.get() == _last_pos {
                _stuck += 1;
                if _stuck > 5_000 {
                    return; // emergency exit: stuck in skip_to_next_top_level
                }
            } else {
                _stuck = 0;
                _last_pos = self.pos.get();
            }
            match self.peek() {
                Token::Eof => return,
                Token::Module | Token::Class | Token::Interface | Token::Package | Token::Program
                    if depth == 0 => return,
                Token::Function | Token::Task | Token::Begin | Token::Case | Token::CaseX
                | Token::CaseZ | Token::Fork | Token::Specify | Token::Generate
                | Token::Covergroup => {
                    depth += 1;
                    self.advance();
                }
                Token::End | Token::Endcase | Token::Join | Token::EndFunction | Token::EndTask
                | Token::Endmodule | Token::EndClass | Token::EndInterface | Token::EndPackage
                | Token::EndPrimitive | Token::EndSpecify | Token::EndGenerate
                | Token::EndGroup => {
                    depth = depth.saturating_sub(1);
                    self.advance();
                }
                _ => {
                    self.advance();
                }
            }
        }
    }

    fn skip_until_semi_or_end(&mut self) -> Result<(), SimError> {
        let mut depth: i32 = 0;
        let mut _last_pos = self.pos.get();
        let mut _stuck = 0u32;
        loop {
            if self.pos.get() == _last_pos {
                _stuck += 1;
                if _stuck > 5_000 {
                    return Err(self.err("parser stuck (no progress) in skip_until_semi_or_end"));
                }
            } else {
                _stuck = 0;
                _last_pos = self.pos.get();
            }
            match self.peek() {
                Token::Semi if depth == 0 => {
                    self.advance();
                    return Ok(());
                }
                Token::Endmodule | Token::EndFunction | Token::EndTask | Token::EndClass | Token::EndInterface | Token::EndPackage | Token::EndProgram | Token::EndGenerate | Token::EndSpecify | Token::EndClocking | Token::EndConfig | Token::EndPrimitive | Token::EndTable | Token::EndGroup | Token::EndSequence | Token::EndEnum | Token::Eof => {
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

    /// Skip the body of a class declaration (from 'class' token to matching 'endclass').
    /// Assumes the current token is 'class'. Used when class appears inside a module body.
    fn skip_class_body(&mut self) {
        let mut depth = 0i32;
        self.advance(); // consume 'class'
        // Skip class header: name, #(params), extends, ';'
        let mut _last_pos = self.pos.get();
        let mut _stuck = 0u32;
        loop {
            if self.pos.get() == _last_pos {
                _stuck += 1;
                if _stuck > 5_000 {
                    self.push_warning_at("parser stuck (no progress) skipping class body — aborting".to_string(), self.peek_line(), self.peek_col());
                    return; // emergency exit: stuck in skip_class_body header
                }
            } else {
                _stuck = 0;
                _last_pos = self.pos.get();
            }
            match self.peek() {
                Token::Semi => { self.advance(); break; }
                Token::Hash => {
                    self.advance();
                    if self.peek() == &Token::LParen {
                        let _ = self.skip_balanced_paren_light();
                    }
                }
                Token::EndClass | Token::Eof => return,
                _ => { self.advance(); }
            }
        }
        // Skip class body until matching endclass
        let mut _last_pos2 = self.pos.get();
        let mut _stuck2 = 0u32;
        loop {
            if self.pos.get() == _last_pos2 {
                _stuck2 += 1;
                if _stuck2 > 5_000 {
                    self.push_warning_at("parser stuck (no progress) in class body — aborting".to_string(), self.peek_line(), self.peek_col());
                    return; // emergency exit: stuck in skip_class_body body
                }
            } else {
                _stuck2 = 0;
                _last_pos2 = self.pos.get();
            }
            match self.peek() {
                Token::Eof => return,
                Token::Class => {
                    depth += 1;
                    self.advance();
                }
                Token::EndClass => {
                    if depth == 0 {
                        self.advance();
                        // Skip optional ': name' after endclass
                        if self.peek() == &Token::Colon {
                            self.advance();
                            if matches!(self.peek(), Token::Ident(_)) {
                                self.advance();
                            }
                        }
                        return;
                    }
                    depth -= 1;
                    self.advance();
                }
                _ => { self.advance(); }
            }
        }
    }

    /// Lightweight balanced paren skipping (no error return — just consume tokens).
    fn skip_balanced_paren_light(&mut self) -> Result<(), SimError> {
        let mut depth = 1i32;
        self.advance(); // consume '('
        loop {
            match self.peek() {
                Token::LParen => { depth += 1; self.advance(); }
                Token::RParen => {
                    depth -= 1;
                    self.advance();
                    if depth == 0 { return Ok(()); }
                }
                Token::Eof => return Ok(()),
                _ => { self.advance(); }
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
                | Token::EndClass
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
        let mut _last_pos = self.pos.get();
        let mut _stuck = 0u32;
        loop {
            if self.pos.get() == _last_pos {
                _stuck += 1;
                if _stuck > 5_000 {
                    return; // emergency exit: stuck in skip_attribute
                }
            } else {
                _stuck = 0;
                _last_pos = self.pos.get();
            }
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
