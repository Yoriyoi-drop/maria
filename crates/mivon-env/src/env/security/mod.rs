//! Security context — permission, sandbox file access, validasi input.
//!
//! Mengatur: permission, sandbox, plugin tepercaya, hash/signature.

mod permissions;
mod sandbox;
mod security;
mod validation;

pub use permissions::PermissionSet;
pub use sandbox::FileAccessPolicy;
pub use security::SecurityContext;
pub use validation::{safe_file_name, validate_identifier, validate_path};
