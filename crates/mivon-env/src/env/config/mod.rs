//! Config context — satu-satunya pintu compiler untuk membaca pengaturan.
//!
//! Sumber config: CLI, file TOML/JSON, ENV. Compiler tidak membaca file;
//! ia cukup bertanya `config.max_threads()`, `config.incremental()`,
//! `config.sim_timeout()`.

mod cli;
mod config;
mod defaults;
mod environment;
mod loader;
mod validator;

pub use cli::EnvCliOptions;
pub use config::{ConfigContext, ConfigSource};
pub use loader::{load_from_path, ConfigFileFormat};
