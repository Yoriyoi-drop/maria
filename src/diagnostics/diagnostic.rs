//! Diagnostic — structured error/warning reporting.
//!
//! Setiap diagnostic memiliki: level, code, message, spans, notes, hints.
//! Thread-safe via MPSC channel (DiagSink).

use std::borrow::Cow;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::intern::Symbol;

// ─── Diagnostic Level ───

/// Severity level untuk diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagLevel {
    /// Internal compiler error — bug di compiler itu sendiri.
    Bug,
    /// Pasti salah — harus diperbaiki user.
    Error,
    /// Mencurigakan tapi valid — user harus perhatikan.
    Warning,
    /// Informasi tambahan.
    Note,
    /// Saran perbaikan.
    Help,
}

impl fmt::Display for DiagLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiagLevel::Bug => write!(f, "bug"),
            DiagLevel::Error => write!(f, "error"),
            DiagLevel::Warning => write!(f, "warning"),
            DiagLevel::Note => write!(f, "note"),
            DiagLevel::Help => write!(f, "help"),
        }
    }
}

// ─── Diagnostic Code ───

/// Error code untuk diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagCode {
    // Parse errors: E1xxx
    UnexpectedToken,
    ExpectedToken,
    ExpectedSemi,
    UnclosedBlock,
    // Semantic errors: E2xxx
    UndefinedSignal,
    TypeMismatch,
    WidthMismatch,
    // Elaboration errors: E3xxx
    ModuleNotFound,
    CircularDependency,
    ParamMismatch,
    // Runtime errors: E9xxx
    SimulationError,
    OutOfBounds,
}

impl DiagCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            DiagCode::UnexpectedToken => "E1001",
            DiagCode::ExpectedToken => "E1002",
            DiagCode::ExpectedSemi => "E1003",
            DiagCode::UnclosedBlock => "E1004",
            DiagCode::UndefinedSignal => "E2001",
            DiagCode::TypeMismatch => "E2002",
            DiagCode::WidthMismatch => "E2003",
            DiagCode::ModuleNotFound => "E3001",
            DiagCode::CircularDependency => "E3002",
            DiagCode::ParamMismatch => "E3003",
            DiagCode::SimulationError => "E9001",
            DiagCode::OutOfBounds => "E9002",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            DiagCode::UnexpectedToken => "unexpected token",
            DiagCode::ExpectedToken => "expected token",
            DiagCode::ExpectedSemi => "expected ';'",
            DiagCode::UnclosedBlock => "unclosed block",
            DiagCode::UndefinedSignal => "undefined signal",
            DiagCode::TypeMismatch => "type mismatch",
            DiagCode::WidthMismatch => "width mismatch",
            DiagCode::ModuleNotFound => "module not found",
            DiagCode::CircularDependency => "circular dependency",
            DiagCode::ParamMismatch => "parameter mismatch",
            DiagCode::SimulationError => "simulation error",
            DiagCode::OutOfBounds => "out of bounds",
        }
    }
}

impl fmt::Display for DiagCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ─── Diagnostic Span ───

/// Source location for a diagnostic.
#[derive(Debug, Clone)]
pub struct DiagSpan {
    pub file: Symbol,
    pub start: u32,
    pub end: u32,
    pub label: Option<Cow<'static, str>>,
}

impl DiagSpan {
    pub fn new(file: Symbol, start: u32, end: u32) -> Self {
        DiagSpan {
            file,
            start,
            end,
            label: None,
        }
    }

    pub fn with_label(mut self, label: impl Into<Cow<'static, str>>) -> Self {
        self.label = Some(label.into());
        self
    }
}

// ─── Diagnostic Note ───

/// Additional note attached to a diagnostic.
#[derive(Debug, Clone)]
pub struct DiagNote {
    pub message: Cow<'static, str>,
    pub span: Option<DiagSpan>,
}

impl DiagNote {
    pub fn new(message: impl Into<Cow<'static, str>>) -> Self {
        DiagNote {
            message: message.into(),
            span: None,
        }
    }

    pub fn with_span(mut self, span: DiagSpan) -> Self {
        self.span = Some(span);
        self
    }
}

// ─── Diagnostic ───

/// Structured diagnostic (error/warning/note/help).
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub level: DiagLevel,
    pub code: DiagCode,
    pub message: Cow<'static, str>,
    pub spans: Vec<DiagSpan>,
    pub notes: Vec<DiagNote>,
    pub hints: Vec<Cow<'static, str>>,
}

impl Diagnostic {
    pub fn new(level: DiagLevel, code: DiagCode, message: impl Into<Cow<'static, str>>) -> Self {
        Diagnostic {
            level,
            code,
            message: message.into(),
            spans: Vec::new(),
            notes: Vec::new(),
            hints: Vec::new(),
        }
    }

    pub fn error(code: DiagCode, message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(DiagLevel::Error, code, message)
    }

    pub fn warning(code: DiagCode, message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(DiagLevel::Warning, code, message)
    }

    pub fn bug(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(DiagLevel::Bug, DiagCode::SimulationError, message)
    }

    pub fn with_span(mut self, span: DiagSpan) -> Self {
        self.spans.push(span);
        self
    }

    pub fn with_note(mut self, note: impl Into<Cow<'static, str>>) -> Self {
        self.notes.push(DiagNote::new(note));
        self
    }

    pub fn with_hint(mut self, hint: impl Into<Cow<'static, str>>) -> Self {
        self.hints.push(hint.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.level == DiagLevel::Error || self.level == DiagLevel::Bug
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}: {}", self.level, self.code, self.message)?;

        for span in &self.spans {
            write!(f, "\n  --> {}:{}:{}", span.file, span.start, span.end)?;
            if let Some(label) = &span.label {
                write!(f, " — {}", label)?;
            }
        }

        for note in &self.notes {
            write!(f, "\n  = note: {}", note.message)?;
        }

        for hint in &self.hints {
            write!(f, "\n  = help: {}", hint)?;
        }

        Ok(())
    }
}

// ─── Diagnostic Sink (thread-safe) ───

/// Thread-safe diagnostic collection using crossbeam channel.
pub struct DiagSink {
    /// MPSC channel for cross-thread diagnostics
    sender: crossbeam::channel::Sender<Diagnostic>,
    receiver: crossbeam::channel::Receiver<Diagnostic>,
    /// Collected diagnostics (after flush)
    collected: Mutex<Vec<Diagnostic>>,
    /// Total diagnostics pushed (atomic counter)
    pub total_pushed: AtomicUsize,
}

impl DiagSink {
    pub fn new() -> Self {
        let (sender, receiver) = crossbeam::channel::unbounded();
        DiagSink {
            sender,
            receiver,
            collected: Mutex::new(Vec::new()),
            total_pushed: AtomicUsize::new(0),
        }
    }

    /// Push a diagnostic (non-blocking, lock-free fast path).
    pub fn push(&self, diag: Diagnostic) {
        self.total_pushed.fetch_add(1, Ordering::Relaxed);
        let _ = self.sender.try_send(diag);
    }

    /// Flush all pending diagnostics into collected vec.
    pub fn flush(&self) {
        while let Ok(diag) = self.receiver.try_recv() {
            self.collected.lock().unwrap().push(diag);
        }
    }

    /// Get all collected diagnostics (flush first).
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        self.flush();
        let mut all = self.collected.lock().unwrap().clone();
        // Sort by file, then by position
        all.sort_by(|a, b| {
            let file_a = a.spans.first().map(|s| s.file.index());
            let file_b = b.spans.first().map(|s| s.file.index());
            file_a.cmp(&file_b).then_with(|| {
                let pos_a = a.spans.first().map(|s| s.start).unwrap_or(0);
                let pos_b = b.spans.first().map(|s| s.start).unwrap_or(0);
                pos_a.cmp(&pos_b)
            })
        });
        all
    }

    /// Get count of errors (not warnings/notes).
    pub fn error_count(&self) -> usize {
        self.flush();
        self.collected
            .lock()
            .unwrap()
            .iter()
            .filter(|d| d.is_error())
            .count()
    }

    /// Are there any errors?
    pub fn has_errors(&self) -> bool {
        self.error_count() > 0
    }

    /// Clear all collected diagnostics.
    pub fn clear(&self) {
        self.collected.lock().unwrap().clear();
    }
}

impl Default for DiagSink {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnostic_basic() {
        let d = Diagnostic::error(DiagCode::UnexpectedToken, "found 'foo' where ';' expected");
        assert!(d.is_error());
        assert_eq!(d.level, DiagLevel::Error);
        assert_eq!(d.code, DiagCode::UnexpectedToken);
    }

    #[test]
    fn test_diagnostic_with_span() {
        let file = Symbol::intern("test.sv");
        let d = Diagnostic::error(DiagCode::UndefinedSignal, "signal 'foo' not found")
            .with_span(DiagSpan::new(file, 10, 13).with_label("here"));
        assert_eq!(d.spans.len(), 1);
    }

    #[test]
    fn test_diag_sink_push_flush() {
        let sink = DiagSink::new();
        sink.push(Diagnostic::error(DiagCode::UnexpectedToken, "bad token"));
        sink.push(Diagnostic::warning(
            DiagCode::WidthMismatch,
            "width differs",
        ));

        let diags = sink.diagnostics();
        assert_eq!(diags.len(), 2);
        assert_eq!(sink.error_count(), 1);
    }

    #[test]
    fn test_diag_code_display() {
        assert_eq!(DiagCode::UnexpectedToken.as_str(), "E1001");
        assert_eq!(DiagCode::ModuleNotFound.as_str(), "E3001");
    }
}
