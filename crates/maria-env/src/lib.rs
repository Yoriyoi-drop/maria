//! maria-env — Enterprise Context Architecture + LSP Server + Plugin System.
//!
//! - `env`   : GlobalEnv root object menampung 12 context (Config, Workspace,
//!             Runtime, Compiler, Cache, Database, Diagnostics, Telemetry,
//!             Verification, Simulation, Security, Plugins) — desain 5 doc/env.md.
//! - `lsp`   : Language Server Protocol (tower-lsp) — diagnostics via parser.
//! - `plugin`: Plugin architecture (stub WASM-based, ex src/plugin/).
//!
//! Dependency satu arah: Config → Workspace → Runtime → Compiler →
//! Cache/Database/Diagnostics/Telemetry → Verification → Simulation.

pub mod env;

// ── LSP Server (tower-lsp) — hanya di-compile dengan feature `lsp` ──
#[cfg(feature = "lsp")]
pub mod lsp;

pub mod plugin;

pub use env::{for_cli, shutdown, startup, startup_with, GlobalEnv};
#[cfg(feature = "lsp")]
pub use lsp::{run_lsp_server, LspBackend};
pub use plugin::{ExamplePlugin, Plugin, PluginManager, PluginMetadata};
