use crate::diagnostics::diagnostic::{DiagCode, Diagnostic};
use crate::error::SimError;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const MAX_INCLUDE_DEPTH: usize = 64;

/// Maximum macro expansion depth to prevent infinite recursion.
const MAX_MACRO_EXPANSION_DEPTH: usize = 64;

struct CondFrame {
    taking_branch: bool,
    branch_taken: bool,
}

#[derive(Clone)]
struct MacroDef {
    value: String,
    params: Vec<String>,
}

#[derive(Clone)]
pub struct Preprocessor {
    defines: HashMap<String, MacroDef>,
    search_paths: Vec<PathBuf>,
    warned_includes: HashSet<String>,
    include_stack: Vec<PathBuf>,
    include_set: std::collections::HashSet<PathBuf>,
    pub quiet: bool,
    pub timescale: Option<(String, String)>, // (unit, precision)
    pub warnings: Vec<Diagnostic>,
    /// Line ranges (start, end) inklusif, 1-based, dalam koordinat output
    /// preprocessed, yang di-exclude dari coverage oleh `` `coverage_off ``
    /// ... `` `coverage_on `` (IEEE 1800 simulation control directives).
    pub coverage_exclusions: Vec<(usize, usize)>,
}

impl Preprocessor {
    pub fn new() -> Self {
        Self {
            defines: HashMap::new(),
            search_paths: Vec::new(),
            warned_includes: HashSet::new(),
            include_stack: Vec::new(),
            include_set: std::collections::HashSet::new(),
            quiet: false,
            timescale: None,
            warnings: Vec::new(),
            coverage_exclusions: Vec::new(),
        }
    }

    pub fn max_macro_expansion_depth(&self) -> usize {
        MAX_MACRO_EXPANSION_DEPTH
    }

    pub fn define(&mut self, name: &str, value: &str) {
        self.defines.insert(
            name.to_string(),
            MacroDef {
                value: value.to_string(),
                params: Vec::new(),
            },
        );
    }

    pub fn add_search_path(&mut self, path: &str) {
        self.search_paths.push(PathBuf::from(path));
    }

    pub fn preprocess_file(&mut self, filename: &str) -> Result<String, SimError> {
        let path = Path::new(filename);
        let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let source = fs::read_to_string(filename)
            .map_err(|e| SimError::preprocessor(format!("cannot read '{}': {}", filename, e)))?;
        let processed = self.preprocess(&source, Some(&dir))?;
        Ok(processed)
    }

    pub fn preprocess(
        &mut self,
        source: &str,
        current_dir: Option<&PathBuf>,
    ) -> Result<String, SimError> {
        let lines: Vec<&str> = source.lines().collect();
        let mut output = String::new();
        let mut i = 0;
        let mut cond_stack: Vec<CondFrame> = vec![];
        let mut emitting = true;
        // SIM-29: tracking region `` `coverage_off `` ... `` `coverage_on ``
        let mut cov_start: Option<usize> = None;

        while i < lines.len() {
            let mut raw_line = lines[i].to_string();
            loop {
                let te = raw_line.trim_end();
                if !te.ends_with('\\') && !te.ends_with("\\\r") {
                    break;
                }
                if te.ends_with('\r') {
                    raw_line.pop();
                }
                raw_line = raw_line.trim_end().to_string();
                raw_line.pop(); // remove trailing \
                i += 1;
                if i < lines.len() {
                    raw_line.push('\n');
                    raw_line.push_str(lines[i]);
                } else {
                    break;
                }
            }
            let trimmed = raw_line.trim();

            if !trimmed.starts_with('`') {
                if emitting {
                    let expanded = self.expand_inline_macros(&raw_line);
                    output.push_str(&expanded);
                    output.push('\n');
                }
                i += 1;
                continue;
            }

            let directive = &trimmed[1..];
            let (cmd, rest) = self.split_directive(directive);

            match cmd {
                "include" => {
                    if emitting {
                        if rest.trim().starts_with('`') {
                            // Not actually an include — misparsed due to nested backtick
                            i += 1;
                            continue;
                        }
                        let inc_path = match self.parse_include_path(rest) {
                            Ok(p) => p,
                            Err(e) => {
                                if !self.quiet {
                                    self.warnings.push(Diagnostic::warning(DiagCode::InvalidSyntax, format!("{}", e)));
                                }
                                i += 1;
                                continue;
                            }
                        };
                        match self.resolve_path(&inc_path, current_dir) {
                            Ok(resolved) => {
                                if self.include_stack.len() >= MAX_INCLUDE_DEPTH {
                                    return Err(SimError::preprocessor(format!(
                                        "include depth exceeded ({}) — possible circular include for '{}'",
                                        MAX_INCLUDE_DEPTH, inc_path
                                    )));
                                }
                                if self.include_set.contains(&resolved) {
                                    return Err(SimError::preprocessor(format!(
                                        "circular include detected: '{}' already in include stack",
                                        inc_path
                                    )));
                                }
                                self.include_stack.push(resolved.clone());
                                self.include_set.insert(resolved.clone());
                                self.warned_includes.remove(&inc_path);
                                let inc_result = (|| -> Result<(), SimError> {
                                    let inc_source = fs::read_to_string(&resolved)
                                        .map_err(|e| SimError::preprocessor(format!("cannot read include '{}': {}", resolved.display(), e)))?;
                                    let inc_dir = resolved.parent().map(|p| p.to_path_buf());
                                    output.push_str(&format!(
                                        "`line 1 \"{}\"\n",
                                        resolved.display()
                                    ));
                                    let processed = self.preprocess(&inc_source, inc_dir.as_ref())?;
                                    output.push_str(&processed);
                                    if !processed.ends_with('\n') {
                                        output.push('\n');
                                    }
                                    Ok(())
                                })();
                                let popped = self.include_stack.pop();
                                if let Some(p) = popped {
                                    self.include_set.remove(&p);
                                }
                                if let Err(e) = inc_result {
                                    if !self.quiet {
                                        self.warnings.push(Diagnostic::warning(DiagCode::InvalidSyntax, format!("{}", e)));
                                    }
                                }
                            }
                            Err(e) => {
                                if !self.quiet && self.warned_includes.insert(inc_path.clone()) {
                                    self.warnings.push(Diagnostic::warning(DiagCode::InvalidSyntax, format!("{}", e)));
                                }
                            }
                        }
                    }
                }
                "define" => {
                    if emitting {
                        self.parse_define(rest);
                    }
                }
                "undef" => {
                    if emitting {
                        let name = rest.trim();
                        if !name.is_empty() {
                            self.defines.remove(name);
                        }
                    }
                }
                "ifdef" => {
                    let defined = self.eval_ifdef_expr(rest.trim());
                    cond_stack.push(CondFrame {
                        taking_branch: defined,
                        branch_taken: defined,
                    });
                    emitting = cond_stack.iter().all(|f| f.taking_branch);
                }
                "ifndef" => {
                    let defined = self.eval_ifdef_expr(rest.trim());
                    cond_stack.push(CondFrame {
                        taking_branch: !defined,
                        branch_taken: !defined,
                    });
                    emitting = cond_stack.iter().all(|f| f.taking_branch);
                }
                "elsif" => {
                    let frame = cond_stack.last_mut().ok_or_else(|| {
                        SimError::preprocessor(format!(
                            "line {}: `elsif without matching `ifdef/`ifndef",
                            i + 1
                        ))
                    })?;
                    if frame.branch_taken {
                        frame.taking_branch = false;
                    } else {
                        let defined = self.eval_ifdef_expr(rest.trim());
                        if defined {
                            frame.taking_branch = true;
                            frame.branch_taken = true;
                        }
                    }
                    emitting = cond_stack.iter().all(|f| f.taking_branch);
                }
                "else" => {
                    let frame = cond_stack.last_mut().ok_or_else(|| {
                        SimError::preprocessor(format!(
                            "line {}: `else without matching `ifdef/`ifndef",
                            i + 1
                        ))
                    })?;
                    if frame.branch_taken {
                        frame.taking_branch = false;
                    } else {
                        frame.taking_branch = true;
                        frame.branch_taken = true;
                    }
                    emitting = cond_stack.iter().all(|f| f.taking_branch);
                }
                "endif" => {
                    cond_stack.pop().ok_or_else(|| {
                        SimError::preprocessor(format!(
                            "line {}: `endif without matching `ifdef/`ifndef",
                            i + 1
                        ))
                    })?;
                    emitting = cond_stack.iter().all(|f| f.taking_branch);
                }
                "line" => {
                    if emitting {
                        output.push_str(&raw_line);
                        output.push('\n');
                    }
                }
                "timescale" => {
                    // `timescale 1ns / 1ps — parse and store
                    let ts = rest.trim();
                    if let Some(slash_pos) = ts.find('/') {
                        let unit = ts[..slash_pos].trim().to_string();
                        let prec = ts[slash_pos + 1..].trim().to_string();
                        self.timescale = Some((unit, prec));
                    } else if !ts.is_empty() {
                        self.timescale = Some((ts.to_string(), String::new()));
                    }
                }
                "coverage_off" => {
                    if emitting && cov_start.is_none() {
                        // Baris output berikutnya (1-based) jadi awal region exclude.
                        cov_start = Some(output.lines().count() + 1);
                    }
                }
                "coverage_on" => {
                    if emitting {
                        if let Some(start) = cov_start.take() {
                            let end = output.lines().count();
                            if start <= end {
                                self.coverage_exclusions.push((start, end));
                            }
                        }
                    }
                }
                "default_nettype" => {
                    // `default_nettype wire|none|... — track for implicit net declarations
                    // Currently tracked but not enforced in elaborated
                }
                "celldefine"
                | "endcelldefine"
                | "unconnected_drive"
                | "nounconnected_drive"
                | "pragma"
                | "assert"
                | "debug"
                | "resetall"
                | "PICORV32_REGS" => {
                    // Standard or tool-specific Verilog directives that we ignore
                }
                "FORMAL_KEEP" => {
                    // Yosys formal attribute — emit the rest as Verilog declaration
                    if emitting {
                        output.push_str(rest);
                        output.push('\n');
                    }
                }
                _ => {
                    // Unknown backtick directive — skip silently
                }
            }

            i += 1;
        }

        // Tutup region coverage_off yang belum ditutup di akhir file
        if let Some(start) = cov_start.take() {
            let end = output.lines().count();
            if start <= end {
                self.coverage_exclusions.push((start, end));
            }
        }

        if !cond_stack.is_empty() {
            let any_taking = cond_stack.iter().any(|f| f.taking_branch);
            if !self.quiet && any_taking && cond_stack.len() <= 3 {
                // Show the current file name if available from the source
                let file_hint = current_dir
                    .as_ref()
                    .and_then(|d| d.file_name())
                    .map(|n| format!(" in '{}'", n.to_string_lossy()))
                    .unwrap_or_default();
                self.warnings.push(Diagnostic::warning(DiagCode::InvalidSyntax, format!("{} open `ifdef/`ifndef block(s) at end of file{} (auto-closed)", cond_stack.len(), file_hint)));
            }
            // Auto-close remaining conditionals so they don't corrupt subsequent files
            while let Some(_frame) = cond_stack.pop() {
                // Frame automatically dropped
            }
        }

        Ok(output)
    }

    fn is_emitting(&self, stack: &[CondFrame]) -> bool {
        stack.iter().all(|f| f.taking_branch)
    }

    fn split_directive<'a>(&self, directive: &'a str) -> (&'a str, &'a str) {
        let trimmed = directive.trim_start();
        let end = trimmed
            .find(|c: char| c.is_whitespace() || c == '(' || c == '[')
            .unwrap_or(trimmed.len());
        let cmd = &trimmed[..end];
        let rest = trimmed[end..].trim();
        (cmd, rest)
    }

    fn parse_include_path(&self, rest: &str) -> Result<String, SimError> {
        let s = rest.trim();
        if s.starts_with('`') {
            return Err(SimError::preprocessor(format!(
                "include path is a macro reference (not a string literal): {}",
                s
            )));
        }
        if let Some(rest) = s.strip_prefix('"') {
            let end = rest
                .find('"')
                .ok_or_else(|| SimError::preprocessor("unterminated include path"))?;
            Ok(rest[..end].to_string())
        } else if let Some(rest) = s.strip_prefix('<') {
            let end = rest
                .find('>')
                .ok_or_else(|| SimError::preprocessor("unterminated include path"))?;
            Ok(rest[..end].to_string())
        } else {
            Err(SimError::preprocessor(format!(
                "invalid include syntax: {}",
                s
            )))
        }
    }

    fn resolve_path(
        &self,
        inc_path: &str,
        current_dir: Option<&PathBuf>,
    ) -> Result<PathBuf, SimError> {
        if let Some(dir) = current_dir {
            let candidate = dir.join(inc_path);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
        for search_path in &self.search_paths {
            let candidate = search_path.join(inc_path);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
        let candidate = PathBuf::from(inc_path);
        if candidate.exists() {
            return Ok(candidate);
        }
        Err(SimError::preprocessor(format!(
            "include file '{}' not found",
            inc_path
        )))
    }

    fn parse_define(&mut self, rest: &str) {
        let s = rest.trim();
        if s.is_empty() {
            return;
        }

        let (name, params, value) = if let Some(open_paren) = s.find('(') {
            let name = s[..open_paren].trim().to_string();
            let close_paren = s[open_paren..]
                .find(')')
                .map(|p| open_paren + p)
                .unwrap_or(s.len());
            let params_str = if open_paren < close_paren && close_paren <= s.len() {
                &s[open_paren + 1..close_paren]
            } else {
                ""
            };
            let params: Vec<String> = params_str
                .split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect();
            let value = if close_paren < s.len() {
                s[close_paren + 1..].trim().to_string()
            } else {
                String::new()
            };
            (name, params, value)
        } else {
            let end = s.find(|c: char| c.is_whitespace()).unwrap_or(s.len());
            let name = s[..end].to_string();
            let value = s[end..].trim().to_string();
            (name, Vec::new(), value)
        };

        self.defines.insert(name, MacroDef { value, params });
    }

    /// Evaluate `ifdef/`ifndef expression. Supports:
    ///   MACRO                — true if defined
    ///   MACRO_A || MACRO_B  — true if any defined
    ///   MACRO_A && MACRO_B  — true if all defined
    fn eval_ifdef_expr(&self, expr: &str) -> bool {
        let expr = expr.trim();
        if expr.is_empty() {
            return false;
        }
        if let Some((a, b)) = expr.split_once("||") {
            let a = a.trim();
            let b = b.trim();
            let left = if a.contains("&&") {
                a.split("&&").all(|m| self.defines.contains_key(m.trim()))
            } else {
                self.defines.contains_key(a)
            };
            let right = if b.contains("&&") {
                b.split("&&").all(|m| self.defines.contains_key(m.trim()))
            } else {
                self.defines.contains_key(b)
            };
            return left || right;
        }
        if let Some((a, b)) = expr.split_once("&&") {
            return self.defines.contains_key(a.trim())
                && self.defines.contains_key(b.trim());
        }
        self.defines.contains_key(expr)
    }

    /// Expand inline macros in a single line (wrapper with depth tracking).
    fn expand_inline_macros(&self, line: &str) -> String {
        self.expand_inline_macros_depth(line, 0)
    }

    /// Expand inline macros with recursive depth tracking.
    /// If `depth` exceeds MAX_MACRO_EXPANSION_DEPTH, returns the line as-is
    /// to prevent infinite recursion from circular macro definitions.
    fn expand_inline_macros_depth(&self, line: &str, depth: usize) -> String {
        if depth >= MAX_MACRO_EXPANSION_DEPTH {
            return line.to_string();
        }

        let mut result = String::new();
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                result.push_str(&line[i..]);
                break;
            }
            if bytes[i] == b'`'
                && i + 1 < bytes.len()
                && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_')
            {
                i += 1;
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let name = &line[start..i];
                if let Some(mdef) = self.defines.get(name) {
                    if mdef.params.is_empty() {
                        let expanded = self.expand_inline_macros_depth(&mdef.value, depth + 1);
                        result.push_str(&expanded);
                    } else {
                        let args = if i < bytes.len() && bytes[i] == b'(' {
                            let args_start = i + 1;
                            let mut paren_depth = 1;
                            let mut args_end = args_start;
                            while args_end < bytes.len() && paren_depth > 0 {
                                if bytes[args_end] == b'(' {
                                    paren_depth += 1;
                                } else if bytes[args_end] == b')' {
                                    paren_depth -= 1;
                                }
                                args_end += 1;
                            }
                            let args_str = &line[args_start..args_end - 1];
                            i = args_end;
                            self.split_macro_args(args_str, mdef.params.len())
                        } else {
                            Vec::new()
                        };
                        let expanded_args: Vec<String> = args
                            .iter()
                            .map(|arg| self.expand_inline_macros_depth(arg, depth + 1))
                            .collect();
                        // Substitute parameters with expanded arguments — single pass on bytes
                        let mut expanded = String::with_capacity(mdef.value.len());
                        let val_bytes = mdef.value.as_bytes();
                        let mut pos = 0;
                        while pos < val_bytes.len() {
                            let mut matched = false;
                            for (param, arg) in mdef.params.iter().zip(expanded_args.iter()) {
                                if !param.is_empty() && pos + param.len() <= val_bytes.len()
                                    && &val_bytes[pos..pos + param.len()] == param.as_bytes()
                                {
                                    expanded.push_str(arg);
                                    pos += param.len();
                                    matched = true;
                                    break;
                                }
                            }
                            if !matched {
                                expanded.push(val_bytes[pos] as char);
                                pos += 1;
                            }
                        }
                        let expanded = self.expand_inline_macros_depth(&expanded, depth + 1);
                        result.push_str(&expanded);
                    }
                } else {
                    result.push('`');
                    result.push_str(name);
                }
            } else {
                result.push(bytes[i] as char);
                i += 1;
            }
        }
        result
    }

    fn split_macro_args(&self, args_str: &str, expected_count: usize) -> Vec<String> {
        let mut args = Vec::new();
        let mut current = String::new();
        let mut depth = 0usize;
        for c in args_str.chars() {
            match c {
                '(' => {
                    depth += 1;
                    current.push(c);
                }
                ')' => {
                    depth = depth.saturating_sub(1);
                    current.push(c);
                }
                ',' if depth == 0 => {
                    args.push(current.trim().to_string());
                    current.clear();
                }
                _ => {
                    current.push(c);
                }
            }
        }
        let last = current.trim().to_string();
        if !last.is_empty() || args.len() < expected_count {
            args.push(last);
        }
        args
    }
}
