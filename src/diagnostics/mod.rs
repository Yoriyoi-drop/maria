//! Diagnostics — error recovery, diagnostic sink, formatted output.
//!
//! Phase 4 implementation. Thread-safe diagnostic collection via MPSC channel.

pub mod codes;
pub mod diagnostic;
pub mod emitter;
pub mod recovery;

pub use codes::{all_codes, lookup_code};
pub use diagnostic::{DiagCode, DiagLevel, DiagNote, DiagSink, DiagSpan, Diagnostic};
pub use emitter::{format_diagnostic, TerminalEmitter};
pub use recovery::ParserRecovery;
