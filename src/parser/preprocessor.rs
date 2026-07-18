use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

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
    pub quiet: bool,
}

impl Preprocessor {
    pub fn new() -> Self {
        Self {
            defines: HashMap::new(),
            search_paths: Vec::new(),
            warned_includes: HashSet::new(),
            quiet: false,
        }
    }

    pub fn define(&mut self, name: &str, value: &str) {
        self.defines.insert(name.to_string(), MacroDef {
            value: value.to_string(),
            params: Vec::new(),
        });
    }

    pub fn add_search_path(&mut self, path: &str) {
        self.search_paths.push(PathBuf::from(path));
    }

    pub fn preprocess_file(&mut self, filename: &str) -> Result<String, String> {
        let path = Path::new(filename);
        let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let source = fs::read_to_string(filename)
            .map_err(|e| format!("cannot read '{}': {}", filename, e))?;
        let processed = self.preprocess(&source, Some(&dir))?;
        Ok(processed)
    }

    pub fn preprocess(&mut self, source: &str, current_dir: Option<&PathBuf>) -> Result<String, String> {
        let lines: Vec<&str> = source.lines().collect();
        let mut output = String::new();
        let mut i = 0;
        let mut cond_stack: Vec<CondFrame> = vec![];

        while i < lines.len() {
            let mut raw_line = lines[i].to_string();
            // Handle line continuation (trailing \)
            while raw_line.trim_end().ends_with('\\') || raw_line.trim_end().ends_with("\\\r") {
                if raw_line.trim_end().ends_with('\r') {
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
                if self.is_emitting(&cond_stack) {
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
                    if self.is_emitting(&cond_stack) {
                        if rest.trim().starts_with('`') {
                            // Not actually an include — misparsed due to nested backtick
                            i += 1;
                            continue;
                        }
                        let inc_path = match self.parse_include_path(rest) {
                            Ok(p) => p,
                            Err(e) => {
                                if !self.quiet { eprintln!("  ** WARNING: {}", e); }
                                i += 1;
                                continue;
                            }
                        };
                        match self.resolve_path(&inc_path, current_dir) {
                            Ok(resolved) => {
                                self.warned_includes.remove(&inc_path);
                                match fs::read_to_string(&resolved) {
                                    Ok(inc_source) => {
                                        let inc_dir = resolved.parent().map(|p| p.to_path_buf());
                                        output.push_str(&format!("`line 1 \"{}\"\n", resolved.display()));
                                        match self.preprocess(&inc_source, inc_dir.as_ref()) {
                                            Ok(processed) => {
                                                output.push_str(&processed);
                                                if !processed.ends_with('\n') {
                                                    output.push('\n');
                                                }
                                            }
                                            Err(e) => {
                                                if !self.quiet { eprintln!("  ** WARNING: error processing include '{}': {}", resolved.display(), e); }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        if !self.quiet { eprintln!("  ** WARNING: cannot read include '{}': {}", resolved.display(), e); }
                                    }
                                }
                            }
                            Err(e) => {
                                if !self.quiet && self.warned_includes.insert(inc_path.clone()) {
                                    eprintln!("  ** WARNING: {}", e);
                                }
                            }
                        }
                    }
                }
                "define" => {
                    if self.is_emitting(&cond_stack) {
                        self.parse_define(rest);
                    }
                }
                "undef" => {
                    if self.is_emitting(&cond_stack) {
                        let name = rest.trim();
                        if !name.is_empty() {
                            self.defines.remove(name);
                        }
                    }
                }
                "ifdef" => {
                    let macro_name = rest.trim();
                    let defined = self.defines.contains_key(macro_name);
                    cond_stack.push(CondFrame {
                        taking_branch: defined,
                        branch_taken: defined,
                    });
                }
                "ifndef" => {
                    let macro_name = rest.trim();
                    let defined = self.defines.contains_key(macro_name);
                    cond_stack.push(CondFrame {
                        taking_branch: !defined,
                        branch_taken: !defined,
                    });
                }
                "elsif" => {
                    let frame = cond_stack.last_mut().ok_or_else(|| {
                        format!("line {}: `elsif without matching `ifdef/`ifndef", i + 1)
                    })?;
                    if frame.branch_taken {
                        frame.taking_branch = false;
                    } else {
                        let macro_name = rest.trim();
                        let defined = self.defines.contains_key(macro_name);
                        if defined {
                            frame.taking_branch = true;
                            frame.branch_taken = true;
                        }
                    }
                }
                "else" => {
                    let frame = cond_stack.last_mut().ok_or_else(|| {
                        format!("line {}: `else without matching `ifdef/`ifndef", i + 1)
                    })?;
                    if frame.branch_taken {
                        frame.taking_branch = false;
                    } else {
                        frame.taking_branch = true;
                        frame.branch_taken = true;
                    }
                }
                "endif" => {
                    cond_stack.pop().ok_or_else(|| {
                        format!("line {}: `endif without matching `ifdef/`ifndef", i + 1)
                    })?;
                }
                "line" => {
                    if self.is_emitting(&cond_stack) {
                        output.push_str(&raw_line);
                        output.push('\n');
                    }
                }
                "timescale" | "celldefine" | "endcelldefine" | "unconnected_drive" |
                "nounconnected_drive" | "default_nettype" | "pragma" | "assert" |
                "debug" | "PICORV32_REGS" => {
                    // Standard or tool-specific Verilog directives that we ignore
                }
                "FORMAL_KEEP" => {
                    // Yosys formal attribute — emit the rest as Verilog declaration
                    if self.is_emitting(&cond_stack) {
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

        if !cond_stack.is_empty() && !self.quiet {
            eprintln!("  ** WARNING: unterminated `ifdef/`ifndef ({} level(s) remaining at end of file)", cond_stack.len());
        }

        Ok(output)
    }

    fn is_emitting(&self, stack: &[CondFrame]) -> bool {
        stack.iter().all(|f| f.taking_branch)
    }

    fn split_directive<'a>(&self, directive: &'a str) -> (&'a str, &'a str) {
        let trimmed = directive.trim_start();
        let end = trimmed.find(|c: char| c.is_whitespace() || c == '(' || c == '[').unwrap_or(trimmed.len());
        let cmd = &trimmed[..end];
        let rest = trimmed[end..].trim();
        (cmd, rest)
    }

    fn parse_include_path(&self, rest: &str) -> Result<String, String> {
        let s = rest.trim();
        if s.starts_with('`') {
            return Err(format!("include path is a macro reference (not a string literal): {}", s));
        }
        if s.starts_with('"') {
            let end = s[1..].find('"').ok_or_else(|| format!("unterminated include path"))?;
            Ok(s[1..=end].to_string())
        } else if s.starts_with('<') {
            let end = s[1..].find('>').ok_or_else(|| format!("unterminated include path"))?;
            Ok(s[1..=end].to_string())
        } else {
            Err(format!("invalid include syntax: {}", s))
        }
    }

    fn resolve_path(&self, inc_path: &str, current_dir: Option<&PathBuf>) -> Result<PathBuf, String> {
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
        Err(format!("include file '{}' not found", inc_path))
    }

    fn parse_define(&mut self, rest: &str) {
        let s = rest.trim();
        if s.is_empty() { return; }

        let (name, params, value) = if let Some(open_paren) = s.find('(') {
            let name = s[..open_paren].trim().to_string();
            let close_paren = s[open_paren..].find(')')
                .map(|p| open_paren + p)
                .unwrap_or(s.len());
            let params_str = if open_paren + 1 <= close_paren && close_paren <= s.len() {
                &s[open_paren + 1..close_paren]
            } else {
                ""
            };
            let params: Vec<String> = params_str.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect();
            let value = if close_paren + 1 <= s.len() {
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

    fn expand_inline_macros(&self, line: &str) -> String {
        let mut result = String::new();
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            // Skip macro expansion inside // comments
            if chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
                // Push remaining chars from current position to end using chars iterator
                while i < chars.len() {
                    result.push(chars[i]);
                    i += 1;
                }
                break;
            }
            if chars[i] == '`' && i + 1 < chars.len() && (chars[i + 1].is_ascii_alphabetic() || chars[i + 1] == '_') {
                i += 1;
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect();
                if let Some(mdef) = self.defines.get(&name) {
                    if mdef.params.is_empty() {
                        result.push_str(&mdef.value);
                    } else {
                        let args = if i < chars.len() && chars[i] == '(' {
                            let args_start = i + 1;
                            let mut depth = 1;
                            let mut args_end = args_start;
                            while args_end < chars.len() && depth > 0 {
                                if chars[args_end] == '(' { depth += 1; }
                                else if chars[args_end] == ')' { depth -= 1; }
                                args_end += 1;
                            }
                            let args_str: String = chars[args_start..args_end - 1].iter().collect();
                            i = args_end;
                            self.split_macro_args(&args_str, mdef.params.len())
                        } else {
                            Vec::new()
                        };
                        let mut expanded = mdef.value.clone();
                        for (param, arg) in mdef.params.iter().zip(args.iter()) {
                            expanded = expanded.replace(param, arg);
                        }
                        result.push_str(&expanded);
                    }
                } else {
                    result.push('`');
                    result.push_str(&name);
                }
            } else {
                result.push(chars[i]);
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
                '(' => { depth += 1; current.push(c); }
                ')' => { depth = depth.saturating_sub(1); current.push(c); }
                ',' if depth == 0 => {
                    args.push(current.trim().to_string());
                    current.clear();
                }
                _ => { current.push(c); }
            }
        }
        let last = current.trim().to_string();
        if !last.is_empty() || args.len() < expected_count {
            args.push(last);
        }
        args
    }
}
