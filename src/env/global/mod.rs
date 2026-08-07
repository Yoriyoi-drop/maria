//! Global context — root object arsitektur Maria.
//!
//! Berisi `GlobalEnv` (aggregator context) + lifecycle startup/shutdown
//! + metadata build/version.

pub mod build;
pub mod global_env;
pub mod shutdown;
pub mod startup;
pub mod version;

pub use build::BuildInfo;
pub use global_env::GlobalEnv;
pub use shutdown::shutdown;
pub use startup::{for_cli, startup, startup_with};
