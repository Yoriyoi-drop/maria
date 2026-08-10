//! Formatter diagnostik — membungkus `format_diagnostic` untuk output konsisten.

use maria_core::diagnostics::{Diagnostic, format_diagnostic};

/// Format satu diagnostic menjadi string (untuk CLI/JSON/GUI).
pub fn format(diag: &Diagnostic) -> String {
    format_diagnostic(diag)
}

/// Format daftar diagnostic, dipisah newline.
pub fn format_all(diags: &[Diagnostic]) -> String {
    diags.iter().map(format_diagnostic).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use maria_core::diagnostics::DiagCode;

    #[test]
    fn test_format_nonempty() {
        let d = Diagnostic::error(DiagCode::ModuleNotFound, "module tidak ditemukan");
        let s = format(&d);
        assert!(s.contains("module tidak ditemukan") || s.contains("ModuleNotFound"));
    }
}
