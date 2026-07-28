use crate::diagnostics::diagnostic::{DiagCode, DiagLevel, Diagnostic, SourceSnippet};
use crate::diagnostics::emitter::format_diagnostic;

/// Error context for rich error reporting.
///
/// ⚠️ DEPRECATED: Use `Diagnostic` with `RuntimeContext` and `SourceSnippet` instead.
/// See `src/diagnostics/diagnostic.rs` for the modern error reporting API.
#[derive(Debug, Clone)]
#[deprecated(note = "Use Diagnostic with RuntimeContext and SourceSnippet instead")]
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

#[derive(Debug, Clone)]
pub enum SimError {
    Parse(String),
    Elaborate(String),
    Runtime(String),
    Preprocessor(String),
    Waveform(String),
    Debugger(String),
    Io(std::io::ErrorKind, String),
    /// Error yang membawa Diagnostic terstruktur (source snippet, error code, dll).
    /// Ini adalah varian modern — menggantikan Parse/String untuk error dengan konteks kaya.
    Diagnostic(Diagnostic),
}

impl SimError {
    pub fn new(line: Option<usize>, message: impl Into<String>) -> Self {
        let msg = message.into();
        match line {
            Some(line) => SimError::Parse(format!("line {}: {}", line, msg)),
            None => SimError::Runtime(msg),
        }
    }

    pub fn parse(msg: impl Into<String>) -> Self {
        SimError::Parse(msg.into())
    }
    pub fn elaborate(msg: impl Into<String>) -> Self {
        SimError::Elaborate(msg.into())
    }
    pub fn runtime(msg: impl Into<String>) -> Self {
        SimError::Runtime(msg.into())
    }
    pub fn preprocessor(msg: impl Into<String>) -> Self {
        SimError::Preprocessor(msg.into())
    }
    pub fn waveform(msg: impl Into<String>) -> Self {
        SimError::Waveform(msg.into())
    }
    pub fn debugger(msg: impl Into<String>) -> Self {
        SimError::Debugger(msg.into())
    }

    /// Buat runtime error dengan diagnostic penuh (error code, konteks, dll).
    pub fn with_diag(code: DiagCode, message: impl Into<String>) -> Self {
        Self::Runtime(format!("[{}] {}", code.as_str(), message.into()))
    }

    /// Buat runtime error dari Diagnostic struct.
    /// OBSOLETE: Gunakan SimError::Diagnostic(diag) langsung.
    pub fn from_diagnostic(diag: &Diagnostic) -> Self {
        Self::Diagnostic(diag.clone())
    }

    /// Buat error parser dari Diagnostic yang sudah jadi dengan source snippet.
    pub fn from_parse_diagnostic(diag: Diagnostic) -> Self {
        Self::Diagnostic(diag)
    }

    /// Buat error elaboration dari Diagnostic.
    pub fn from_elab_diagnostic(diag: Diagnostic) -> Self {
        Self::Diagnostic(diag)
    }

    /// Ekstrak error code dari message format "[RT0001] message"
    fn parse_error_code(msg: &str) -> Option<&'static str> {
        if msg.starts_with('[') {
            if let Some(end) = msg.find(']') {
                let code_str = &msg[1..end];
                if let Some(code) = crate::diagnostics::codes::lookup_code(code_str) {
                    return Some(code.as_str());
                }
            }
        }
        None
    }

    /// Try to extract file:line:col from common error format.
    fn extract_location(msg: &str) -> (Option<String>, Option<usize>, Option<usize>) {
        // Format: "filename:line:col: message"
        let parts: Vec<&str> = msg.splitn(4, ':').collect();
        if parts.len() >= 4 {
            let file = parts[0].trim().to_string();
            if let Ok(line) = parts[1].trim().parse::<usize>() {
                if let Ok(col) = parts[2].trim().parse::<usize>() {
                    return (Some(file), Some(line), Some(col));
                }
            }
        }
        // Format: "line N: message"
        if let Some(rest) = msg.strip_prefix("line ") {
            if let Some(end) = rest.find(':') {
                if let Ok(line) = rest[..end].trim().parse::<usize>() {
                    return (None, Some(line), None);
                }
            }
        }
        (None, None, None)
    }

    /// Dapatkan error code yang sesuai.
    pub fn error_code(&self) -> &'static str {
        let msg = self.to_string();
        
        // Coba parse dari format "[RT0001] message"
        if let Some(code) = Self::parse_error_code(&msg) {
            return code;
        }

        // For Diagnostic variant, use its code directly
        if let SimError::Diagnostic(diag) = self {
            return diag.code.as_str();
        }

        match self {
            SimError::Parse(_) => {
                if msg.contains("unexpected token") || msg.contains("Unexpected") {
                    "E1001"
                } else if msg.contains("expected") && msg.contains("';'") {
                    "E1003"
                } else if msg.contains("expected") {
                    "E1002"
                } else if msg.contains("unclosed") {
                    "E1004"
                } else {
                    "E1005"
                }
            }
            SimError::Elaborate(_) => {
                if msg.contains("not found") || msg.contains("module") {
                    "E3001"
                } else if msg.contains("circular") {
                    "E3002"
                } else if msg.contains("parameter") || msg.contains("param") {
                    "E3003"
                } else {
                    "E3004"
                }
            }
            SimError::Runtime(_) => "E9001",
            SimError::Preprocessor(_) => "E0001",
            SimError::Waveform(_) => "E0002",
            SimError::Debugger(_) => "E0003",
            SimError::Io(_, _) => "E0000",
            SimError::Diagnostic(diag) => diag.code.as_str(),
        }
    }

    /// Convert ke Diagnostic struct untuk formatting penuh.
    /// Jika message mengandung "[CODE] ...", extract code dan message terpisah.
    pub fn to_diagnostic(&self) -> Diagnostic {
        // Fast path: already a Diagnostic
        if let SimError::Diagnostic(diag) = self {
            return diag.clone();
        }

        let msg = self.to_string();
        let level = match self {
            SimError::Parse(_) | SimError::Elaborate(_) | SimError::Runtime(_) => DiagLevel::Error,
            SimError::Preprocessor(_) => DiagLevel::Warning,
            SimError::Waveform(_) => DiagLevel::Error,
            SimError::Debugger(_) => DiagLevel::Error,
            SimError::Io(_, _) => DiagLevel::Error,
            SimError::Diagnostic(diag) => diag.level,
        };

        let code_str = self.error_code();
        let code = crate::diagnostics::codes::lookup_code(code_str).unwrap_or(DiagCode::SimulationError);

        // Jika ada format "[CODE] message", gunakan message tanpa prefix [CODE]
        let clean_msg = if msg.starts_with('[') {
            if let Some(end) = msg.find(']') {
                if end + 2 < msg.len() {
                    msg[end + 2..].to_string()  // Skip "] "
                } else {
                    msg.clone()
                }
            } else {
                msg.clone()
            }
        } else {
            msg.clone()
        };

        let mut diag = Diagnostic::new(level, code, clean_msg);

        // Try to reconstruct source snippet from flat string format like "file:line:col: message"
        if let (Some(file), Some(line), Some(col)) = Self::extract_location(&msg) {
            // Try to extract the actual source line from the message
            // The message might be something like "cpu_top.sv:182:17: interface is null"
            // Extract the actual error message after the location prefix
            let parts: Vec<&str> = msg.splitn(4, ':').collect();
            let error_msg = if parts.len() >= 4 {
                parts[3..].join(":").trim().to_string()
            } else {
                msg.clone()
            };

            let snippet = SourceSnippet::new(file, line, col, error_msg);
            diag = diag.with_source_snippet(snippet);
        }

        diag
    }

    /// Format error dengan konteks lengkap seperti compiler profesional.
    pub fn format_with_context(&self, ctx: &ErrorContext) -> String {
        // If we already have a structured diagnostic, use its formatted output
        if let SimError::Diagnostic(diag) = self {
            return format_diagnostic(diag);
        }

        let msg = self.to_string();
        let kind = match self {
            SimError::Parse(_) => "error",
            SimError::Elaborate(_) => "error",
            SimError::Runtime(_) => "error",
            SimError::Preprocessor(_) => "warning",
            SimError::Waveform(_) => "error",
            SimError::Debugger(_) => "error",
            SimError::Io(_, _) => "error",
            SimError::Diagnostic(_) => unreachable!(), // handled above
        };

        let code = self.error_code();

        let mut output = format!("{}[{}]: {}\n", kind, code, msg);

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

impl std::fmt::Display for SimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SimError::Parse(msg) => write!(f, "{}", msg),
            SimError::Elaborate(msg) => write!(f, "{}", msg),
            SimError::Runtime(msg) => write!(f, "{}", msg),
            SimError::Preprocessor(msg) => write!(f, "{}", msg),
            SimError::Waveform(msg) => write!(f, "{}", msg),
            SimError::Debugger(msg) => write!(f, "{}", msg),
            SimError::Io(kind, msg) => write!(f, "I/O error ({}): {}", kind, msg),
            SimError::Diagnostic(diag) => write!(f, "{}[{}]: {}", diag.level, diag.code, diag.message),
        }
    }
}

impl std::error::Error for SimError {}

impl From<std::io::Error> for SimError {
    fn from(e: std::io::Error) -> Self {
        SimError::Io(e.kind(), e.to_string())
    }
}

impl From<String> for SimError {
    fn from(msg: String) -> Self {
        let is_parse = msg.starts_with("line ") && msg[5..].find(':').is_some();
        if is_parse {
            SimError::Parse(msg)
        } else {
            SimError::Runtime(msg)
        }
    }
}

impl From<&str> for SimError {
    fn from(msg: &str) -> Self {
        SimError::Runtime(msg.to_string())
    }
}
