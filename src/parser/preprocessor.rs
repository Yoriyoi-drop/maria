use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

struct CondFrame {
    taking_branch: bool,
    branch_taken: bool,
}

#[derive(Default)]
pub struct Preprocessor {
    defines: HashMap<String, String>,
    search_paths: Vec<PathBuf>,
}

impl Preprocessor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn define(&mut self, name: &str, value: &str) {
        self.defines.insert(name.to_string(), value.to_string());
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
            let raw_line = lines[i];
            let trimmed = raw_line.trim();

            if !trimmed.starts_with('`') {
                if self.is_emitting(&cond_stack) {
                    let expanded = self.expand_inline_macros(raw_line);
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
                        let inc_path = self.parse_include_path(rest)?;
                        let resolved = self.resolve_path(&inc_path, current_dir)?;
                        let inc_source = fs::read_to_string(&resolved)
                            .map_err(|e| format!("cannot include '{}': {}", resolved.display(), e))?;
                        let inc_dir = resolved.parent().map(|p| p.to_path_buf());
                        let processed = self.preprocess(&inc_source, inc_dir.as_ref())?;
                        output.push_str(&processed);
                        if !processed.ends_with('\n') {
                            output.push('\n');
                        }
                    }
                }
                "define" => {
                    let (name, value) = self.parse_define(rest);
                    if self.is_emitting(&cond_stack) {
                        self.defines.insert(name, value);
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
                _ => {
                    // Unknown backtick directive — UVM macro or similar; skip silently
                }
            }

            i += 1;
        }

        if !cond_stack.is_empty() {
            return Err("unterminated `ifdef/`ifndef".to_string());
        }

        Ok(output)
    }

    fn is_emitting(&self, stack: &[CondFrame]) -> bool {
        stack.iter().all(|f| f.taking_branch)
    }

    fn split_directive<'a>(&self, directive: &'a str) -> (&'a str, &'a str) {
        let trimmed = directive.trim_start();
        let end = trimmed.find(|c: char| c.is_whitespace()).unwrap_or(trimmed.len());
        let cmd = &trimmed[..end];
        let rest = trimmed[end..].trim();
        (cmd, rest)
    }

    fn parse_include_path(&self, rest: &str) -> Result<String, String> {
        let s = rest.trim();
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

    fn parse_define(&self, rest: &str) -> (String, String) {
        let s = rest.trim();
        if s.is_empty() {
            return (String::new(), String::new());
        }
        let end = s.find(|c: char| c.is_whitespace()).unwrap_or(s.len());
        let name = s[..end].to_string();
        let value = s[end..].trim().to_string();
        (name, value)
    }

    fn expand_inline_macros(&self, line: &str) -> String {
        let mut result = String::new();
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '`' && i + 1 < chars.len() && (chars[i + 1].is_ascii_alphabetic() || chars[i + 1] == '_') {
                i += 1;
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect();
                if let Some(value) = self.defines.get(&name) {
                    result.push_str(value);
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
}
