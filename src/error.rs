use crate::diagnostics::diagnostic::{DiagCode, DiagLevel, Diagnostic, SourceSnippet};
use crate::diagnostics::emitter::format_diagnostic;

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

    /// Buat error dengan structured Diagnostic (error code, konteks, dll).
    /// Menggantikan format string-based `[CODE] message` dengan Diagnostic nyata.
    pub fn with_diag(code: DiagCode, message: impl Into<String>) -> Self {
        let diag = Diagnostic::new(DiagLevel::Error, code, message.into())
            .with_code_context();
        Self::Diagnostic(diag)
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

    /// Tebak DiagCode dari pesan parse error.
    fn parse_code(msg: &str) -> DiagCode {
        if msg.contains("unexpected token") || msg.contains("Unexpected") {
            DiagCode::UnexpectedToken
        } else if msg.contains("expected") && msg.contains("';'") {
            DiagCode::ExpectedSemi
        } else if msg.contains("expected") {
            DiagCode::ExpectedToken
        } else if msg.contains("unclosed") {
            DiagCode::UnclosedBlock
        } else {
            DiagCode::InvalidSyntax
        }
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
            SimError::Parse(ref _msg) => Self::parse_code(&msg).as_str(),
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
            SimError::Preprocessor(_) => "E0101",
            SimError::Waveform(_) => "W0001",
            SimError::Debugger(_) => "E0003",
            SimError::Io(_, _) => "E0004",
            SimError::Diagnostic(diag) => diag.code.as_str(),
        }
    }

    /// Convert ke Diagnostic struct untuk formatting penuh.
    /// Jika message mengandung "[CODE] ...", extract code dan message terpisah.
    pub fn to_diagnostic(&self) -> Diagnostic {
        // Fast path: already a Diagnostic — return clone immediately
        if let SimError::Diagnostic(diag) = self {
            return diag.clone();
        }

        let msg = self.to_string();
        let (level, code) = match self {
            SimError::Parse(_) => (DiagLevel::Error, Self::parse_code(&msg)),
            SimError::Elaborate(_) => (DiagLevel::Error, DiagCode::SimulationError),
            SimError::Runtime(_) => (DiagLevel::Error, DiagCode::SimulationError),
            SimError::Preprocessor(_) => (DiagLevel::Error, DiagCode::PreprocessorError),
            SimError::Waveform(_) => (DiagLevel::Error, DiagCode::WaveformError),
            SimError::Debugger(_) => (DiagLevel::Error, DiagCode::DebuggerError),
            SimError::Io(_, _) => (DiagLevel::Error, DiagCode::IoError),
            SimError::Diagnostic(_) => unreachable!(), // caught by fast path above
        };

        // Jika ada format "[CODE] message", extract code dan gunakan message bersih
        let clean_msg = if msg.starts_with('[') {
            if let Some(end) = msg.find(']') {
                if end + 2 < msg.len() {
                    msg[end + 2..].to_string()
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
            let parts: Vec<&str> = msg.splitn(4, ':').collect();
            let error_msg = if parts.len() >= 4 {
                parts[3..].join(":").trim().to_string()
            } else {
                msg.clone()
            };

            let snippet = SourceSnippet::new(file, line, col, error_msg);
            diag = diag.with_source_snippet(snippet);
        }

        // Tambah explanation + help dari DiagCode untuk diagnostic yang kaya
        diag = diag.with_code_context();

        diag
    }

    /// Format error sebagai string dengan konteks.
    /// Saat ini menggunakan format default dari `Display`.
    /// Untuk format yang lebih kaya, gunakan `TerminalEmitter` dengan `Diagnostic`.
    pub fn format_with_context(&self) -> String {
        if let SimError::Diagnostic(diag) = self {
            return format_diagnostic(diag);
        }
        self.to_string()
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
