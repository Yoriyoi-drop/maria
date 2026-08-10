use maria_core::diagnostics::{DiagLevel, DiagSink, Diagnostic};
use crate::env::diagnostics::{DiagStatistics, EmitterHandle};

/// DiagnosticsContext — pusat penerima diagnostik dari seluruh pipeline.
///
/// Posisi dalam dependency rule: hanya MENERIMA data hasil proses, tidak
/// pernah menjalankan parser/compiler. Compiler cukup `diagnostics.report(diag)`.
pub struct DiagnosticsContext {
    sink: DiagSink,
    emitter: EmitterHandle,
    stats: DiagStatistics,
    warnings_are_errors: bool,
}

impl DiagnosticsContext {
    pub fn new() -> Self {
        DiagnosticsContext {
            sink: DiagSink::new(),
            emitter: EmitterHandle::new(),
            stats: DiagStatistics::default(),
            warnings_are_errors: false,
        }
    }

    pub fn with_warnings_as_errors(mut self, on: bool) -> Self {
        self.warnings_are_errors = on;
        self
    }

    /// Sink mentah (MPSC, thread-safe) untuk push lintas thread.
    pub fn sink(&self) -> &DiagSink {
        &self.sink
    }

    pub fn emitter(&self) -> &EmitterHandle {
        &self.emitter
    }

    pub fn stats(&self) -> &DiagStatistics {
        &self.stats
    }

    /// Catat + kirim ke sink (bukan output langsung).
    pub fn push(&self, diag: Diagnostic) {
        self.record(&diag);
        self.sink.push(diag);
    }

    /// Catat + emit ke terminal.
    pub fn emit(&self, diag: &Diagnostic) {
        self.record(diag);
        self.emitter.emit(diag);
    }

    /// Kirim + emit (paket lengkap: sink tersimpan, terminal tercetak).
    pub fn report(&self, diag: Diagnostic) {
        self.record(&diag);
        self.sink.push(diag.clone());
        self.emitter.emit(&diag);
    }

    /// Ambil seluruh diagnostik tersimpan (sorted by file/posisi).
    pub fn drain(&self) -> Vec<Diagnostic> {
        self.sink.diagnostics()
    }

    pub fn error_count(&self) -> usize {
        self.sink.error_count()
    }

    pub fn has_errors(&self) -> bool {
        self.sink.has_errors()
    }

    /// Apakah `warning` harus dianggap error (config warnings_are_errors).
    pub fn is_fatal(&self, diag: &Diagnostic) -> bool {
        diag.is_error() || (self.warnings_are_errors && diag.level == DiagLevel::Warning)
    }

    fn record(&self, diag: &Diagnostic) {
        match diag.level {
            DiagLevel::Error | DiagLevel::Fatal | DiagLevel::Bug => {
                self.stats.record_error("total");
            }
            DiagLevel::Warning => self.stats.record_warning("total"),
            _ => self.stats.record_note(),
        }
    }
}

impl Default for DiagnosticsContext {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maria_core::diagnostics::DiagCode;

    #[test]
    fn test_report_counts() {
        let ctx = DiagnosticsContext::new();
        ctx.report(Diagnostic::error(DiagCode::ModuleNotFound, "m1"));
        ctx.report(Diagnostic::warning(DiagCode::WaveformError, "w1"));
        ctx.report(Diagnostic::error(DiagCode::InvalidSyntax, "m2"));
        assert_eq!(ctx.error_count(), 2);
        assert_eq!(ctx.stats().total(), 3);
        assert!(ctx.has_errors());
        assert!(!ctx.stats().is_clean());
    }

    #[test]
    fn test_warnings_as_errors() {
        let ctx = DiagnosticsContext::new().with_warnings_as_errors(true);
        let w = Diagnostic::warning(DiagCode::WaveformError, "w");
        assert!(ctx.is_fatal(&w));
        let e = Diagnostic::error(DiagCode::InvalidSyntax, "e");
        assert!(ctx.is_fatal(&e));
    }
}
