//! LSP Server — Language Server Protocol implementation for SystemVerilog.
//!
//! Uses tower-lsp to provide IDE features:
//! - Text document synchronization
//! - Diagnostics (compile errors/warnings via the existing parser/diagnostics engine)
//!
//! # Architecture
//!
//! The `LspBackend` implements the `LanguageServer` trait from tower-lsp.
//! On document open/change, it runs the parser and emits diagnostics via
//! `publishDiagnostics`. No persistent project state is kept between
//! requests (stateless design for simplicity).
//!
//! # Usage
//!
//! ```bash
//! cargo run -- --lsp
//! ```

pub mod auth;
mod backend;

pub use backend::run_lsp_server;
pub use backend::LspBackend;

pub use lsp_types;
/// Re-export tower-lsp for convenience
pub use tower_lsp;
