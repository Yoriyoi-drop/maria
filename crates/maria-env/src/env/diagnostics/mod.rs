//! Diagnostics context — penerima data hasil proses, bukan penjalan proses.
//!
//! Diagram (doc/env.md): Compiler → Diagnostic Builder → Formatter → Reporter
//! → CLI/GUI/JSON. Diagnostics hanya menerima data, tidak menjalankan parser.

mod diagnostics;
mod emitter;
mod error;
mod formatter;
mod statistics;
mod warning;

pub use diagnostics::DiagnosticsContext;
pub use emitter::EmitterHandle;
pub use error::ErrorStats;
pub use formatter::{format, format_all};
pub use statistics::DiagStatistics;
pub use warning::WarningStats;
