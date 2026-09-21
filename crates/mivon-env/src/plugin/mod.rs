//! Plugin — extensible plugin architecture for Mivon.
//!
//! Phase 6+: WASM-based plugin system (stub for now).

#[allow(clippy::module_inception)]
pub mod plugin;

pub use plugin::{ExamplePlugin, Plugin, PluginManager, PluginMetadata};
