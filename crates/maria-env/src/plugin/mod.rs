//! Plugin — extensible plugin architecture for Maria.
//!
//! Phase 6+: WASM-based plugin system (stub for now).

pub mod plugin;

pub use plugin::{ExamplePlugin, Plugin, PluginManager, PluginMetadata};
