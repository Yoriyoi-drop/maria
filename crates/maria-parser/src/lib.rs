// Module declarations: submodule files dari restrukturisasi parser/
pub mod class;
pub mod config;
pub mod decl;
pub mod expr;
pub mod instance;
pub mod lexer;
pub mod package;
pub mod preprocessor;
pub mod proc;
pub mod specify;
pub mod stmt;
pub mod udp;
pub mod util;
use crate::lexer::*;
use maria_ast::*;
use maria_core::diagnostics::diagnostic::{
    DiagCode, DiagLevel, Diagnostic, FixItHint, SourceSnippet,
};
use maria_core::error::SimError;
use maria_core::intern::Symbol;

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
    /// Offset cumulative line untuk konversi token line → file-relative line.
    /// Dihitung dari total baris combined source file sebelumnya. Dipakai saat
    /// file_line_map kosong (FastLexer tidak handle `line directive).
    line_base: usize,
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
            line_base: 0,
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

    /// Set cumulative line base offset untuk konversi token line → file-relative.
    /// Dipakai saat file_line_map kosong (FastLexer).
    pub fn with_line_base(mut self, base: usize) -> Self {
        self.line_base = base;
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
        } else if self.line_base > 0 && cumulative_line > self.line_base {
            // Fallback: pakai line_base (FastLexer path)
            cumulative_line - self.line_base
        } else {
            cumulative_line
        };
        (best_file, file_relative)
    }

    fn push_depth(&mut self) -> Result<(), SimError> {
        self.recursion_depth += 1;
        // Limit 1024 (bukan lebih tinggi): build debug, satu level nested
        // begin/end menghabiskan puluhan-KB stack (frame besar Stmt by-value),
        // 2000 level sudah overflow 256MB SEBELUM limit lama (4096) tersentuh.
        // Nesting statement/ekspresi >1024 tidak ada di HDL nyata.
        if self.recursion_depth > 1024 {
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
        } else if msg_str.contains("unclosed")
            || msg_str.contains("unterminated")
            || msg_str.contains("unexpected EOF")
        {
            DiagCode::UnclosedBlock
        } else {
            DiagCode::InvalidSyntax
        };

        // Buat source snippet — source_lines berisi `line directive` + source file.
        // display_line adalah file-relative (dari resolve_source_file), tapi
        // source_lines[0] = `line directive`, jadi offset +1 untuk mapping.
        let source_line = if display_line > 0 {
            // +1 karena source_lines[0] = `line 1 "file"` directive
            let idx = display_line; // display_line 1 → source_lines[1]
            if idx < self.source_lines.len() {
                Some(self.source_lines[idx].clone())
            } else {
                None
            }
        } else {
            None
        };

        let msg_for_diag = msg_str.clone();
        let mut diag = Diagnostic::error(code, msg_for_diag);

        // Generate fix-it hints untuk error umum
        if let Some(sl) = &source_line {
            match code {
                DiagCode::ExpectedSemi => {
                    // Fix-it: tambahkan semicolon di akhir baris
                    let trimmed = sl.trim_end();
                    if !trimmed.ends_with(';') {
                        let fix_it = FixItHint::insert(
                            display_file.clone(),
                            display_line,
                            trimmed.len() + 1,
                            ";",
                            "Add missing semicolon",
                        );
                        diag = diag.with_fix_it(fix_it);
                    }
                }
                DiagCode::UnclosedBlock => {
                    // Fix-it: tambahkan closing keyword di akhir file
                    // Gunakan resolve_source_file untuk konversi combined line ke file line
                    let last_combined_line = self.source_lines.len();
                    let (fix_file, fix_line) = self.resolve_source_file(last_combined_line);
                    let fix_it = FixItHint::insert(
                        fix_file,
                        fix_line + 1, // Insert after last line of original file
                        1,
                        "\nend",
                        "Add missing 'end' to close block",
                    );
                    diag = diag.with_fix_it(fix_it);
                }
                DiagCode::UnexpectedToken => {
                    // Fix-it: hapus token yang tidak diharapkan
                    if let Some(tok) = self.tokens.get(self.pos.get()) {
                        let token_col = tok.2;
                        let token_len = format!("{}", tok.0).len();
                        let fix_it = FixItHint::delete(
                            display_file.clone(),
                            display_line,
                            token_col,
                            display_line,
                            token_col + token_len,
                            "Remove unexpected token",
                        );
                        diag = diag.with_fix_it(fix_it);
                    }
                }
                DiagCode::ExpectedToken => {
                    // Fix-it: untuk expected keyword seperti endmodule, end, endfunction, dll.
                    let msg_lower = msg_str.to_lowercase();
                    if msg_lower.contains("endmodule")
                        || msg_lower.contains("endfunction")
                        || msg_lower.contains("endtask")
                        || msg_lower.contains("endclass")
                        || msg_lower.contains("endinterface")
                        || msg_lower.contains("endpackage")
                        || msg_lower.contains("end")
                    {
                        let last_combined_line = self.source_lines.len();
                        let (fix_file, fix_line) = self.resolve_source_file(last_combined_line);
                        let fix_it = FixItHint::insert(
                            fix_file,
                            fix_line + 1,
                            1,
                            "\nend",
                            "Add missing 'end' keyword",
                        );
                        diag = diag.with_fix_it(fix_it);
                    }
                }
                _ => {}
            }
        } else if display_line > 0 && display_line <= self.source_lines.len() {
            // Fallback: gunakan display_line jika cumulative_line tidak valid
            let sl = &self.source_lines[display_line - 1];
            match code {
                DiagCode::ExpectedSemi => {
                    let trimmed = sl.trim_end();
                    if !trimmed.ends_with(';') {
                        let fix_it = FixItHint::insert(
                            display_file.clone(),
                            display_line,
                            trimmed.len() + 1,
                            ";",
                            "Add missing semicolon",
                        );
                        diag = diag.with_fix_it(fix_it);
                    }
                }
                DiagCode::UnclosedBlock => {
                    let last_combined_line = self.source_lines.len();
                    let (fix_file, fix_line) = self.resolve_source_file(last_combined_line);
                    let fix_it = FixItHint::insert(
                        fix_file,
                        fix_line + 1,
                        1,
                        "\nend",
                        "Add missing 'end' to close block",
                    );
                    diag = diag.with_fix_it(fix_it);
                }
                DiagCode::ExpectedToken => {
                    let msg_lower = msg_str.to_lowercase();
                    if msg_lower.contains("endmodule")
                        || msg_lower.contains("endfunction")
                        || msg_lower.contains("endtask")
                        || msg_lower.contains("endclass")
                        || msg_lower.contains("endinterface")
                        || msg_lower.contains("endpackage")
                        || msg_lower.contains("end")
                    {
                        let last_combined_line = self.source_lines.len();
                        let (fix_file, fix_line) = self.resolve_source_file(last_combined_line);
                        let fix_it = FixItHint::insert(
                            fix_file,
                            fix_line + 1,
                            1,
                            "\nend",
                            "Add missing 'end' keyword",
                        );
                        diag = diag.with_fix_it(fix_it);
                    }
                }
                _ => {}
            }
        } else if !self.source_lines.is_empty() {
            // Fallback 2: error at EOF (display_line=0) - insert at end of file
            match code {
                DiagCode::ExpectedToken => {
                    let msg_lower = msg_str.to_lowercase();
                    if msg_lower.contains("endmodule")
                        || msg_lower.contains("endfunction")
                        || msg_lower.contains("endtask")
                        || msg_lower.contains("endclass")
                        || msg_lower.contains("endinterface")
                        || msg_lower.contains("endpackage")
                        || msg_lower.contains("end")
                    {
                        let last_combined_line = self.source_lines.len();
                        let (fix_file, fix_line) = self.resolve_source_file(last_combined_line);
                        let fix_it = FixItHint::insert(
                            fix_file,
                            fix_line + 1,
                            1,
                            "\nend",
                            "Add missing 'end' keyword",
                        );
                        diag = diag.with_fix_it(fix_it);
                    }
                }
                DiagCode::UnclosedBlock => {
                    let last_combined_line = self.source_lines.len();
                    let (fix_file, fix_line) = self.resolve_source_file(last_combined_line);
                    let fix_it = FixItHint::insert(
                        fix_file,
                        fix_line + 1,
                        1,
                        "\nend",
                        "Add missing 'end' to close block",
                    );
                    diag = diag.with_fix_it(fix_it);
                }
                _ => {}
            }
        }

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
        let msg_for_diag = msg.clone();
        let (display_file, display_line) = self.resolve_source_file(line);
        let mut diag = Diagnostic::new(DiagLevel::Warning, DiagCode::InvalidSyntax, msg_for_diag)
            .with_code_context();
        // NOTE: gunakan display_line (file-relative), bukan line (cumulative lintas file).
        if display_line > 0 && display_line <= self.source_lines.len() {
            let source_line = &self.source_lines[display_line - 1];
            let snippet = SourceSnippet::new(&display_file, display_line, col, source_line.trim_end());
            diag = diag.with_source_snippet(snippet);

            // Generate fix-it for common warnings
            let trimmed = source_line.trim_end();
            if msg.contains("missing semicolon") || msg.contains("expected ';'") {
                if !trimmed.ends_with(';') {
                    let fix_it = FixItHint::insert(
                        display_file.clone(),
                        display_line,
                        trimmed.len() + 1,
                        ";",
                        "Add missing semicolon",
                    );
                    diag = diag.with_fix_it(fix_it);
                }
            }
        }
        self.errors.push(diag);
    }

    fn peek_ahead(&self, n: usize) -> &Token {
        if self.tokens.is_empty() {
            return &Token::Eof;
        }
        // PENTING: jangan me-clamp ke token terakhir. Stream token TIDAK
        // diakhiri `Token::Eof` (Phase 5 lexing membuangnya), sehingga clamp
        // mengembalikan token terakhir (mis. `Semi`) selamanya → loop
        // `peek_bracket_has_range_colon`/`peek_is_packed_dim` yang menunggu
        // `Token::Eof => return false` tidak pernah berhenti (hang di parse
        // saat kombinasi file tertentu, mis. entropy_src DV 676-688).
        // Kembalikan `Token::Eof` begitu indeks melewati akhir stream.
        let idx = self.pos.get() + n;
        if idx >= self.tokens.len() {
            return &Token::Eof;
        }
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
            _ => Err(self.err(format!("expected identifier, found {}", self.peek()))),
        }
    }

    pub fn parse_design(&mut self) -> Result<Design, SimError> {
        // PARSER-11 bug fix: `Token::Error` dari lexer (mis. karakter
        // non-ASCII/Unicode di luar string/komentar, base literal salah)
        // SELAMA ini di-skip parser → `reg café` diterima diam-diam sebagai
        // `reg caf` + token error yang dibuang (sim berjalan dgn ident
        // terpotong). Identifier SV hanya ASCII [a-zA-Z_][a-zA-Z0-9_$]* —
        // token error harus membuat parse GAGAL, bukan diabaikan.
        for (i, (tok, line, col)) in self.tokens.iter().enumerate() {
            // BUG FIX (PARSER-11): token `Token::Error` dari lexer (karakter
            // non-ASCII di luar string/komentar, base literal salah) SELAMA ini
            // di-skip parser → `reg café` diterima diam-diam sebagai `reg caf`
            // + token error dibuang (sim jalan dgn ident terpotong). Identifier
            // SV hanya ASCII — token error harus membuat parse GAGAL. pos di-set
            // ke token error agar lokasi source snippet benar.
            let _ = (line, col);
            if let Token::Error(msg) = tok {
                self.pos.set(i);
                return Err(self.err(format!(
                    "lexical error: {} (SV identifier hanya ASCII; karakter non-ASCII harus di dalam string/komentar)",
                    msg
                )));
            }
        }
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
        self.class_names
            .extend(self.global_class_names.iter().copied());
        self.typedef_names
            .extend(self.global_typedef_names.iter().copied());
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
                    self.push_warning_at(
                        "parser stuck (no progress) during class discovery pass — skipping"
                            .to_string(),
                        line,
                        col,
                    );
                    break;
                }
            } else {
                _stuck = 0;
                _last_pos = self.pos.get();
            }
            if self.peek() == &Token::Class {
                match self.parse_class_fast() {
                    Ok(_) => {}
                    Err(e) => {
                        self.errors.push(e.to_diagnostic());
                        self.skip_to_next_top_level();
                        continue;
                    }
                }
            } else if self.peek() == &Token::Module {
                match self.parse_module_fast() {
                    Ok(_) => {}
                    Err(e) => {
                        self.errors.push(e.to_diagnostic());
                        self.skip_to_next_top_level();
                        continue;
                    }
                }
            } else if self.peek() == &Token::Interface {
                // skip interface in first pass (no class deps needed)
                match self.parse_interface_fast() {
                    Ok(_) => {}
                    Err(e) => {
                        self.errors.push(e.to_diagnostic());
                        self.skip_to_next_top_level();
                        continue;
                    }
                }
            } else if self.peek() == &Token::Program {
                // skip program in first pass
                match self.parse_program_fast() {
                    Ok(_) => {}
                    Err(e) => {
                        self.errors.push(e.to_diagnostic());
                        self.skip_to_next_top_level();
                        continue;
                    }
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
                    Err(e) => {
                        self.errors.push(e.to_diagnostic());
                        self.skip_to_next_top_level();
                        continue;
                    }
                }
            } else if self.peek() == &Token::Covergroup {
                // Skip covergroup in first pass — collect name
                let cg = match self.parse_covergroup() {
                    Ok(cg) => cg,
                    Err(e) => {
                        self.errors.push(e.to_diagnostic());
                        self.skip_to_next_top_level();
                        continue;
                    }
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
                let summary = if tok.len() > 40 {
                    format!("{}...", &tok[..40])
                } else {
                    tok
                };

                self.push_warning_at(
                    format!("skipping top-level construct: {}", summary),
                    line,
                    col,
                );
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
                    self.push_warning_at(
                        "parser stuck (no progress) during second pass — skipping".to_string(),
                        line,
                        col,
                    );
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
                        Ok(m) => {
                            modules.push(m);
                            false
                        }
                        Err(e) => {
                            self.errors.push(e.to_diagnostic());
                            true
                        }
                    }
                }
                Token::Interface => {
                    _n_interface += 1;
                    match self.parse_interface() {
                        Ok(iface) => {
                            interfaces.push(iface);
                            false
                        }
                        Err(e) => {
                            self.errors.push(e.to_diagnostic());
                            true
                        }
                    }
                }
                Token::Class => {
                    _n_class += 1;
                    match self.parse_class() {
                        Ok(c) => {
                            classes.push(c);
                            false
                        }
                        Err(e) => {
                            self.errors.push(e.to_diagnostic());
                            true
                        }
                    }
                }
                Token::Package => {
                    _n_package += 1;
                    match self.parse_package_decl() {
                        Ok(p) => {
                            packages.push(p);
                            false
                        }
                        Err(e) => {
                            self.errors.push(e.to_diagnostic());
                            true
                        }
                    }
                }
                Token::Program => match self.parse_module() {
                    Ok(m) => {
                        modules.push(m);
                        false
                    }
                    Err(e) => {
                        self.errors.push(e.to_diagnostic());
                        true
                    }
                },
                Token::Import => {
                    self.advance();
                    let pkg = match self.expect_ident() {
                        Ok(p) => p,
                        Err(e) => {
                            self.errors.push(e.to_diagnostic());
                            self.skip_to_next_top_level();
                            continue;
                        }
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
                            Err(e) => {
                                self.errors.push(e.to_diagnostic());
                                self.skip_to_next_top_level();
                                continue;
                            }
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
                        Ok(c) => {
                            classes.push(c);
                            false
                        }
                        Err(e) => {
                            self.errors.push(e.to_diagnostic());
                            true
                        }
                    }
                }
                Token::Covergroup => match self.parse_covergroup() {
                    Ok(cg) => {
                        if let Some(m) = modules.first_mut() {
                            m.items.push(ModuleItem::Covergroup(cg));
                        }
                        false
                    }
                    Err(e) => {
                        self.errors.push(e.to_diagnostic());
                        true
                    }
                },
                Token::Bind => {
                    self.advance(); // consume 'bind'
                    let target = match self.expect_ident() {
                        Ok(t) => t,
                        Err(e) => {
                            self.errors.push(e.to_diagnostic());
                            self.skip_to_next_top_level();
                            continue;
                        }
                    };
                    match self.parse_instance() {
                        Ok(instance) => {
                            binds.push(BindDecl { target, instance });
                            false
                        }
                        Err(e) => {
                            self.errors.push(e.to_diagnostic());
                            true
                        }
                    }
                }
                Token::Clocking => match self.parse_clocking_block() {
                    Ok(cb) => {
                        clocking_blocks.push(cb);
                        false
                    }
                    Err(e) => {
                        self.errors.push(e.to_diagnostic());
                        true
                    }
                },
                Token::Export => {
                    self.advance();
                    if self.peek() == &Token::StringLit(Symbol::intern("DPI-C"))
                        || self.peek() == &Token::StringLit(Symbol::intern("DPI"))
                    {
                        match self.parse_dpi_import() {
                            Ok(_) => false,
                            Err(e) => {
                                self.errors.push(e.to_diagnostic());
                                true
                            }
                        }
                    } else {
                        match self.skip_until_semi_or_end() {
                            Ok(_) => false,
                            Err(e) => {
                                self.errors.push(e.to_diagnostic());
                                true
                            }
                        }
                    }
                }
                Token::Config => match self.parse_config_decl() {
                    Ok(cfg) => {
                        configs.push(cfg);
                        false
                    }
                    Err(e) => {
                        self.errors.push(e.to_diagnostic());
                        true
                    }
                },
                Token::Primitive => match self.parse_udp_declaration() {
                    Ok(udp) => {
                        udp_defs.push(udp);
                        false
                    }
                    Err(e) => {
                        self.errors.push(e.to_diagnostic());
                        true
                    }
                },
                Token::Function => match self.parse_function(false) {
                    Ok(func) => {
                        unit_funcs.push(func);
                        false
                    }
                    Err(e) => {
                        self.errors.push(e.to_diagnostic());
                        true
                    }
                },
                Token::Task => match self.parse_task(false) {
                    Ok(task) => {
                        unit_tasks.push(task);
                        false
                    }
                    Err(e) => {
                        self.errors.push(e.to_diagnostic());
                        true
                    }
                },
                Token::Typedef => match self.parse_typedef() {
                    Ok(td) => {
                        unit_typedefs.push(td);
                        false
                    }
                    Err(e) => {
                        self.errors.push(e.to_diagnostic());
                        true
                    }
                },
                Token::Let => {
                    // LANG-40: let di level unit (package) — parse & discard
                    // (belum disimpan; module & class didukung).
                    let _ = self.parse_let_decl();
                    false
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
                        Err(e) => {
                            self.errors.push(e.to_diagnostic());
                            true
                        }
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
                            | Token::Longint
                            | Token::Time
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

                        self.push_warning_at(
                            format!("skipping top-level construct: {}", summary),
                            line,
                            col,
                        );
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

    /// Skip seluruh construct assertion SVA di level module:
    /// `name: assert property (…) else begin … end` (hasil macro ASSERT
    /// prim_assert) maupun `assert (expr);`/`cover property …`. Tracking depth
    /// begin/end agar `else begin … end` dilewati utuh; berhenti di `end`
    /// (penutup else-begin) atau `;` di depth 0.
    fn skip_assert_item(&mut self) {
        let mut depth = 0i32;
        let mut saw_begin = false;
        loop {
            match self.peek() {
                Token::Eof => break,
                Token::Begin => {
                    saw_begin = true;
                    depth += 1;
                    self.advance();
                }
                Token::End => {
                    self.advance();
                    if saw_begin {
                        depth -= 1;
                        if depth <= 0 {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                Token::Semi => {
                    self.advance();
                    if depth <= 0 {
                        break;
                    }
                }
                // Assertion `... else `ASSERT_ERROR(X)` tanpa ';' sebelum
                // endinterface/endmodule/dll (OpenTitan prim_assert style) —
                // jangan menelan keyword penutup blok.
                Token::EndInterface
                | Token::Endmodule
                | Token::EndFunction
                | Token::EndTask
                | Token::EndGroup
                | Token::EndPackage
                | Token::EndGenerate => {
                    break;
                }
                _ => {
                    self.advance();
                }
            }
        }
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
            Token::Dollar => {
                // System task call di level MODULE: `$fatal(...)`, `$error(...)`,
                // `$warning(...)` dll — body static assertion prim_assert
                // (`if (!(cond)) $fatal(2, "msg");` di generate-if). Sebelumnya
                // tidak ada arm Dollar → `$fatal(...)` jatuh ke parsing instance
                // → "expected instance name" dan modul terpotong. Tidak
                // dieksekusi engine; parse & buang (Ok(None)).
                self.parse_syscall()?;
                Ok(None)
            }
            // LANG-03 PSL: directive `default clock = posedge clk;` —
            // deklarasi clock default untuk assertion PSL (IEEE 1850).
            // Tidak ada efek runtime (klaim PSL kita selalu menyebut clock
            // eksplisit `@(posedge clk)`); parse & buang agar file PSL
            // tidak error di module body.
            Token::Default => {
                self.advance(); // 'default'
                                // `default clock = <event>;` — skip token sampai ';'
                while self.peek() != &Token::Semi && self.peek() != &Token::Eof {
                    self.advance();
                }
                self.skip_semi();
                Ok(None)
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
            Token::Assert | Token::Assume | Token::Cover | Token::Restrict => {
                // Concurrent assertion SVA module-level. Bentuk BOOLEAN
                // `assert property (@(posedge clk) expr)` di-parse penuh menjadi
                // ModuleItem::PropertyAssert (LANG-04/11/12/13) — elaborator
                // mengubahnya jadi always block ber-clock. Body yang mengandung
                // operator temporal (`##`, `|->`, `[*]`, `until`, dll) membuat
                // parse gagal → rollback pos + skip_assert_item (perilaku lama,
                // modul tetap utuh — assertion kompleks tidak di-elaborasi).
                let save_pos = self.pos.get();
                let save_errs = self.errors.len();
                let save_steps = self.parse_steps;
                let save_peek = self.peek_count.get();
                match self.parse_immediate_assertion() {
                    Ok(stmt) => Ok(Some(ModuleItem::PropertyAssert(Box::new(stmt)))),
                    Err(_) => {
                        // Rollback: kembalikan posisi token + hapus error parsing
                        // yang ditambahkan oleh percobaan, lalu skip penuh.
                        self.pos.set(save_pos);
                        self.errors.truncate(save_errs);
                        self.parse_steps = save_steps;
                        self.peek_count.set(save_peek);
                        self.skip_assert_item();
                        Ok(None)
                    }
                }
            }
            Token::Ident(name) => {
                // Bentuk berlabel: `SigintCheck0_A: assert property (...) else ...`
                // (dihasilkan macro `ASSERT`). Deteksi label + kata kunci assertion
                // sebelum instance/decl logic diproses.
                if matches!(self.peek_ahead(1), Token::Colon)
                    && matches!(
                        self.peek_ahead(2),
                        Token::Assert | Token::Assume | Token::Cover | Token::Property
                    )
                {
                    self.advance(); // label
                    self.advance(); // ':'
                    self.skip_assert_item();
                    return Ok(None);
                }
                if std::env::var("MARIA_DEBUG_PARSE").is_ok() && name.as_str() == "my_class" {
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
                    // Ambiguitas: `Type name;` / `Type name[N];` bisa berupa
                    // deklarasi variabel bertipe user-defined (typedef/class dari
                    // file lain yang belum dikenal di file ini — mis.
                    // `t_Pmpcfg_ent pmpcfg_n_in[64];`), ATAU module instance
                    // `Module inst(...)`. Karena `Type name;` (tanpa paren)
                    // jauh lebih sering di kode DV/RTL (tipe custom), coba
                    // parse_decl DULU dengan backtracking posisi. Instance
                    // module dikenali via lookahead: `(` / `#` / `[N](` yang
                    // mengikuti nama (port list / parameter instance).
                    if matches!(self.peek_ahead(1), Token::Ident(_))
                        || self.peek_ahead(1) == &Token::LBrack
                    {
                        // Deteksi instance: scan maju dari token setelah `name`
                        // untuk `(`/`#` sebelum `;`/`,` pada depth bracket 0.
                        // Contoh instance: `mod u(.clk(clk));`, `mod u[3:0](...);`,
                        // `mod #(.P(1)) u(...);`. Contoh deklarasi (bukan
                        // instance): `Type arr[64];`, `Type a, b;`.
                        let mut is_instance = false;
                        let mut depth = 0i32;
                        let mut i = 2usize;
                        while i < 24 {
                            match self.peek_ahead(i) {
                                Token::LParen | Token::Hash if depth == 0 => {
                                    is_instance = true;
                                    break;
                                }
                                Token::LBrack => depth += 1,
                                Token::RBrack => depth = depth.saturating_sub(1),
                                Token::Semi | Token::Comma if depth == 0 => break,
                                Token::Eof => break,
                                _ => {}
                            }
                            i += 1;
                        }
                        if !is_instance {
                            let saved = self.pos.get();
                            if let Ok(decl) = self.parse_decl() {
                                return Ok(Some(ModuleItem::Decl(decl)));
                            }
                            self.pos.set(saved);
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
                    self.push_warning_at(
                        format!("skipping unknown construct: {}", summary),
                        line,
                        col,
                    );
                    self.skip_until_semi_or_end()?;
                    Ok(None)
                }
            }
            Token::Property => {
                // Deklarasi SVA `property name(...); ... endproperty` — di-skip
                // penuh (property assertion tidak di-elaborasi). Depth-aware
                // untuk property bertingkat. `endproperty` bukan keyword
                // lexer — di-lex sebagai Ident("endproperty").
                self.advance(); // consume 'property'
                let mut depth = 1usize;
                loop {
                    match self.peek() {
                        Token::Property => {
                            depth += 1;
                            self.advance();
                        }
                        Token::Ident(n) if n == "endproperty" => {
                            depth -= 1;
                            self.advance();
                            if depth == 0 {
                                break;
                            }
                        }
                        Token::Eof => break,
                        _ => {
                            self.advance();
                        }
                    }
                }
                // Konsumsi optional `endproperty : name`
                if self.peek() == &Token::Colon {
                    self.advance();
                    if matches!(self.peek(), Token::Ident(_)) {
                        self.advance();
                    }
                }
                Ok(None)
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
            Token::Let => {
                // LANG-40: `let name[(params)] = expr;` (IEEE 1800-2017 §11.12.2).
                let ld = self.parse_let_decl()?;
                return Ok(Some(ModuleItem::Let(ld)));
            }
            Token::Checker => {
                // LANG-10: `checker name (ports); items endchecker` (IEEE
                // 1800-2017 §17.8) — unit assertion terinstansiasi.
                self.advance(); // consume 'checker'
                let name = self.expect_ident()?;
                let mut ports: Vec<Symbol> = Vec::new();
                if self.peek() == &Token::LParen {
                    self.advance();
                    while self.peek() != &Token::RParen && self.peek() != &Token::Eof {
                        // Port bisa `input a`, `a` (direction optional).
                        // Skip direction/type keywords.
                        match self.peek() {
                            Token::Input
                            | Token::Output
                            | Token::Inout
                            | Token::Wire
                            | Token::Logic
                            | Token::Reg
                            | Token::Bit
                            | Token::Int
                            | Token::Integer
                            | Token::Byte
                            | Token::Shortint
                            | Token::Longint
                            | Token::Tri
                            | Token::Tri0
                            | Token::Tri1
                            | Token::Wand
                            | Token::Wor => {
                                self.advance();
                            }
                            Token::Comma => {
                                self.advance();
                            }
                            Token::Ident(s) => {
                                ports.push(*s);
                                self.advance();
                            }
                            Token::Semi => break,
                            _ => {
                                self.advance();
                            }
                        }
                    }
                    if self.peek() == &Token::RParen {
                        self.advance();
                    }
                }
                self.skip_semi();
                let mut items: Vec<ModuleItem> = Vec::new();
                loop {
                    if matches!(self.peek(), Token::EndChecker | Token::Eof) {
                        self.advance();
                        break;
                    }
                    match self.parse_module_item()? {
                        Some(item) => items.push(item),
                        None => {}
                    }
                }
                return Ok(Some(ModuleItem::Checker(CheckerDecl {
                    name,
                    ports,
                    items,
                })));
            }
            Token::Alias => {
                // LANG-08: `alias a = b = c;` (IEEE 1800-2017 §10.9) — semua
                // net dalam satu rantai jadi satu jaringan (short). Rantai
                // disimpan sebagai pasangan berurutan (a=b, b=c) agar
                // elaborator bisa menyatukan grup via union-find.
                self.advance(); // consume 'alias'
                let mut pairs: Vec<(Expr, Expr)> = Vec::new();
                let mut lhs = self.parse_primary_expr()?;
                loop {
                    match self.peek() {
                        Token::BlockingAssign => {
                            self.advance();
                            let rhs = self.parse_primary_expr()?;
                            pairs.push((lhs.clone(), rhs.clone()));
                            lhs = rhs;
                        }
                        _ => break,
                    }
                }
                self.skip_semi();
                return Ok(Some(ModuleItem::NetAlias(pairs)));
            }
            Token::Nettype => {
                // LANG-08: `nettype <type> <name>;` (IEEE 1800-2017 §6.10).
                // Parse tipe dasar + nama; klausa `with resolution_fn` di-skip.
                self.advance(); // consume 'nettype'
                let base = self.parse_type_expr()?;
                // Range packed `[msb:lsb]` — parse_type_expr TIDAK mengonsumsi
                // range (hanya base type); `nettype logic [7:0] mynet;` butuh
                // range agar lebar tipe benar.
                let range = if self.peek() == &Token::LBrack {
                    self.parse_range()?
                } else {
                    None
                };
                let name = self.expect_ident()?;
                // Klausa `with resolution_fn` — `with` adalah Ident biasa.
                if matches!(self.peek(), Token::Ident(s) if *s == Symbol::intern("with")) {
                    self.advance(); // 'with'
                                    // resolution function: ident optional, konsumsi sampai ';'
                    while !matches!(self.peek(), Token::Semi | Token::Eof) {
                        self.advance();
                    }
                }
                self.skip_semi();
                // Daftarkan nama nettype agar `mynet x;` di-parse sebagai
                // deklarasi (UserDefined), bukan instance `mynet x(...)`.
                self.module_type_params.insert(name);
                return Ok(Some(ModuleItem::Nettype(NettypeDecl { name, base, range })));
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
                    // name [] / name [N] / name [msb:lsb] (multi-dimensi diperbolehkan)
                    while self.peek() == &Token::LBrack {
                        self.advance();
                        if self.peek() != &Token::RBrack {
                            self.parse_expr(0)?;
                            if self.peek() == &Token::Colon {
                                self.advance();
                                self.parse_expr(0)?;
                            }
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
            #[allow(unreachable_patterns)]
            Token::Assert | Token::Assume | Token::Cover | Token::Expect => {
                self.skip_until_semi_or_end()?;
                Ok(None)
            }
            Token::Void | Token::Auto | Token::Static => {
                // F37: `static task foo(); ...` / `automatic function ...` di
                // module level — parse sebagai task/function, bukan skip.
                if matches!(self.peek(), Token::Static | Token::Auto)
                    && matches!(self.peek_ahead(1), Token::Task | Token::Function)
                {
                    self.advance(); // static/automatic
                    if self.peek() == &Token::Function {
                        let func = self.parse_function(true)?;
                        Ok(Some(ModuleItem::Func(func)))
                    } else {
                        let task = self.parse_task(true)?;
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
                } else {
                    self.skip_until_semi_or_end()?;
                    Ok(None)
                }
            }
            Token::Class => {
                // Class inside module — parse class declaration
                let class_decl = self.parse_class()?;
                Ok(Some(ModuleItem::Class(class_decl)))
            }
            Token::EndClass => Ok(None),
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
                Token::Module
                | Token::Class
                | Token::Interface
                | Token::Package
                | Token::Program
                    if depth == 0 =>
                {
                    return
                }
                Token::Function
                | Token::Task
                | Token::Begin
                | Token::Case
                | Token::CaseX
                | Token::CaseZ
                | Token::Fork
                | Token::Specify
                | Token::Generate
                | Token::Covergroup => {
                    depth += 1;
                    self.advance();
                }
                Token::End
                | Token::Endcase
                | Token::Join
                | Token::JoinAny
                | Token::JoinNone
                | Token::EndFunction
                | Token::EndTask
                | Token::Endmodule
                | Token::EndClass
                | Token::EndInterface
                | Token::EndPackage
                | Token::EndPrimitive
                | Token::EndSpecify
                | Token::EndGenerate
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
                Token::Endmodule
                | Token::EndFunction
                | Token::EndTask
                | Token::EndClass
                | Token::EndInterface
                | Token::EndPackage
                | Token::EndProgram
                | Token::EndGenerate
                | Token::EndSpecify
                | Token::EndClocking
                | Token::EndConfig
                | Token::EndPrimitive
                | Token::EndTable
                | Token::EndGroup
                | Token::EndSequence
                | Token::EndEnum
                | Token::Eof => {
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
                    self.push_warning_at(
                        "parser stuck (no progress) skipping class body — aborting".to_string(),
                        self.peek_line(),
                        self.peek_col(),
                    );
                    return; // emergency exit: stuck in skip_class_body header
                }
            } else {
                _stuck = 0;
                _last_pos = self.pos.get();
            }
            match self.peek() {
                Token::Semi => {
                    self.advance();
                    break;
                }
                Token::Hash => {
                    self.advance();
                    if self.peek() == &Token::LParen {
                        let _ = self.skip_balanced_paren_light();
                    }
                }
                Token::EndClass | Token::Eof => return,
                _ => {
                    self.advance();
                }
            }
        }
        // Skip class body until matching endclass
        let mut _last_pos2 = self.pos.get();
        let mut _stuck2 = 0u32;
        loop {
            if self.pos.get() == _last_pos2 {
                _stuck2 += 1;
                if _stuck2 > 5_000 {
                    self.push_warning_at(
                        "parser stuck (no progress) in class body — aborting".to_string(),
                        self.peek_line(),
                        self.peek_col(),
                    );
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
                _ => {
                    self.advance();
                }
            }
        }
    }

    /// Lightweight balanced paren skipping (no error return — just consume tokens).
    fn skip_balanced_paren_light(&mut self) -> Result<(), SimError> {
        let mut depth = 1i32;
        self.advance(); // consume '('
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
                        return Ok(());
                    }
                }
                Token::Eof => return Ok(()),
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
