//! Plugin context — plugin lint/GUI/coverage/export dll.
//!
//! Plugin dimuat lewat manager, terdaftar di registry, dan dibatasi sandbox.
//! Context tidak memiliki logika plugin — hanya menyimpan service.

mod manager;
mod plugin;
mod registry;
mod sandbox;

pub use manager::PluginManagerHandle;
pub use plugin::PluginContext;
pub use registry::PluginRegistry;
pub use sandbox::SandboxPolicy;
