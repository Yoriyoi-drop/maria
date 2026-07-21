use thiserror::Error;

/// Error context for rich error reporting
#[derive(Debug, Clone)]
pub struct ErrorContext {
    pub file: Option<String>,
    pub line: Option<usize>,
    pub col: Option<usize>,
    pub source_line: Option<String>,
    pub note: Option<String>,
}

impl ErrorContext {
    pub fn new() -> Self {
        ErrorContext {
            file: None,
            line: None,
            col: None,
            source_line: None,
            note: None,
        }
    }

    pub fn with_file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    pub fn with_line(mut self, line: usize) -> Self {
        self.line = Some(line);
        self
    }

    pub fn with_col(mut self, col: usize) -> Self {
        self.col = Some(col);
        self
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source_line = Some(source.into());
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}

static ERROR_CODES: &[(&str, &str)] = &[
    ("E1001", "Unexpected token"),
    ("E1002", "Expected token"),
    ("E1003", "Unknown type"),
    ("E1004", "Undefined signal"),
    ("E1005", "Module not found"),
    ("E1006", "Invalid syntax"),
    ("E1007", "Expected ']'"),
    ("E1008", "Range expression not supported"),
    ("E1009", "Invalid assignment"),
    ("E1010", "Circular dependency"),
    ("E9001", "Runtime error"),
    ("E9002", "Out of bounds"),
    ("E9003", "Type mismatch"),
];

fn get_error_code(msg: &str) -> &'static str {
    let msg_lower = msg.to_lowercase();
    
    // Pattern-based matching
    if msg_lower.contains("expected ']'") || msg_lower.contains("expected rbr") {
        return "E1007";
    }
    if msg_lower.contains("expected") {
        return "E1002";
    }
    if msg_lower.contains("unknown type") || msg_lower.contains("not defined") {
        return "E1003";
    }
    if msg_lower.contains("not found") || msg_lower.contains("undefined") {
        return "E1004";
    }
    if msg_lower.contains("module") && msg_lower.contains("not found") {
        return "E1005";
    }
    if msg_lower.contains("range") {
        return "E1008";
    }
    if msg_lower.contains("out of bounds") {
        return "E9002";
    }
    if msg_lower.contains("type mismatch") {
        return "E9003";
    }
    if msg_lower.contains("runtime") {
        return "E9001";
    }
    
    // Fallback to table-based matching
    for (code, desc) in ERROR_CODES {
        if msg_lower.contains(&desc.to_lowercase()) {
            return code;
        }
    }
    "E0000"
}

#[derive(Error, Debug)]
pub enum SimError {
    #[error("{0}")]
    Parse(String),
    #[error("{0}")]
    Elaborate(String),
    #[error("{0}")]
    Runtime(String),
    #[error("{0}")]
    Preprocessor(String),
    #[error("{0}")]
    Waveform(String),
    #[error("{0}")]
    Debugger(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

impl SimError {
    pub fn new(line: Option<usize>, message: impl Into<String>) -> Self {
        let msg = message.into();
        match line {
            Some(line) => SimError::Parse(format!("line {}: {}", line, msg)),
            None => SimError::Runtime(msg),
        }
    }

    pub fn parse(msg: impl Into<String>) -> Self { SimError::Parse(msg.into()) }
    pub fn elaborate(msg: impl Into<String>) -> Self { SimError::Elaborate(msg.into()) }
    pub fn runtime(msg: impl Into<String>) -> Self { SimError::Runtime(msg.into()) }
    pub fn preprocessor(msg: impl Into<String>) -> Self { SimError::Preprocessor(msg.into()) }
    pub fn waveform(msg: impl Into<String>) -> Self { SimError::Waveform(msg.into()) }
    pub fn debugger(msg: impl Into<String>) -> Self { SimError::Debugger(msg.into()) }

    /// Format error dengan konteks lengkap seperti compiler profesional
    pub fn format_with_context(&self, ctx: &ErrorContext) -> String {
        let msg = self.to_string();
        let kind = match self {
            SimError::Parse(_) => "error",
            SimError::Elaborate(_) => "error",
            SimError::Runtime(_) => "error",
            SimError::Preprocessor(_) => "warning",
            SimError::Waveform(_) => "error",
            SimError::Debugger(_) => "error",
            SimError::Io(_) => "error",
        };

        let code = get_error_code(&msg);
        
        // Extract the actual error message (after "error: CODE: file:line:col: ")
        let error_msg = if let Some(idx) = msg.find(": error: ") {
            &msg[idx + 9..]
                .split(": ")
                .nth(3)
                .unwrap_or(&msg)
        } else if let Some(idx) = msg.find(": ") {
            let remainder = &msg[idx + 2..];
            remainder.split(": ").last().unwrap_or(&remainder)
        } else {
            &msg
        };

        let mut output = format!("{}: {}: {}\n", kind, code, error_msg);

        if let Some(file) = &ctx.file {
            if let (Some(line), Some(col)) = (ctx.line, ctx.col) {
                output.push_str(&format!(" --> {}:{}:{}\n", file, line, col));
            } else if let Some(line) = ctx.line {
                output.push_str(&format!(" --> {}:{}\n", file, line));
            } else {
                output.push_str(&format!(" --> {}\n", file));
            }
        }

        if let Some(source) = &ctx.source_line {
            output.push_str("  |\n");
            if let Some(line) = ctx.line {
                output.push_str(&format!("{} | {}\n", line, source));
            }

            if let Some(col) = ctx.col {
                output.push_str("  | ");
                for _ in 0..col {
                    output.push(' ');
                }
                output.push_str("^\n");
            }
        }

        if let Some(note) = &ctx.note {
            output.push_str("  |\n");
            output.push_str(&format!("  = note: {}\n", note));
        }

        output
    }
}

impl From<String> for SimError {
    fn from(msg: String) -> Self {
        let is_parse = msg.starts_with("line ") && msg[5..].find(':').is_some();
        if is_parse { SimError::Parse(msg) } else { SimError::Runtime(msg) }
    }
}

impl From<&str> for SimError {
    fn from(msg: &str) -> Self { SimError::Runtime(msg.to_string()) }
}
