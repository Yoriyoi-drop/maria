//! Diagnostics — error recovery, diagnostic sink, formatted output.
//!
//! Phase 4 implementation. Thread-safe diagnostic collection via MPSC channel.

pub mod codes;
pub mod diagnostic;
pub mod emitter;
pub mod global;
pub mod recovery;
pub mod suggest;

pub use codes::{all_codes, lookup_code};
pub use diagnostic::{DiagCode, DiagLevel, DiagNote, DiagSink, DiagSpan, Diagnostic, RuntimeContext, SourceSnippet, FixItHint};
pub use emitter::{format_diagnostic, TerminalEmitter};
pub use global::{diag_global, GlobalDiagnosticEngine};
pub use recovery::ParserRecovery;
pub use suggest::{levenshtein, suggest_name, format_suggestion};

/// Resolve nama file + baris relatif-file untuk posisi di merged source.
///
/// Source gabungan berisi directive `` `line N "file" `` di awal tiap file.
/// Directive di index `i` (0-based) mendeklarasikan: baris berikutnya (merged)
/// adalah baris `N` dari `file`. Untuk baris error `line` (1-based merged),
/// baris relatif-file = N + line - (i+1) - 1. Fallback ke `default_file`.
pub fn resolve_source_location(
    source_lines: &[String],
    default_file: &str,
    line: usize,
) -> (String, usize) {
    if line == 0 || line > source_lines.len() {
        return (default_file.to_string(), line);
    }
    // Scan mundur dari baris error untuk directive `line terakhir
    let end = line.saturating_sub(1); // last index to check (0-based)
    for i in (0..=end).rev() {
        let src = &source_lines[i];
        if let Some(rest) = src.strip_prefix('`') {
            let rest = rest.trim_start();
            if rest.starts_with("line ") {
                // Parse angka baris deklarasi: `line N "filename"
                let mut declared: usize = 1;
                let digits = rest.strip_prefix("line ").unwrap_or(rest).trim_start();
                let digits_end = digits
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(digits.len());
                if let Ok(n) = digits[..digits_end].parse::<usize>() {
                    declared = n.max(1);
                }
                if let Some(start) = rest.find('"') {
                    if let Some(end_q) = rest[start + 1..].find('"') {
                        let file = rest[start + 1..start + 1 + end_q].to_string();
                        let relative = declared + line - (i + 1) - 1;
                        return (file, relative.max(1));
                    }
                }
            }
        }
    }
    (default_file.to_string(), line)
}
