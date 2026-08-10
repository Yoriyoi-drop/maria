use crate::env::security::{FileAccessPolicy, PermissionSet};

/// SecurityContext — permission + sandbox file access + validasi.
///
/// Tidak menjalankan apa pun; hanya policy yang dipakai komponen lain.
#[derive(Debug, Clone)]
pub struct SecurityContext {
    pub permissions: PermissionSet,
    pub files: FileAccessPolicy,
}

impl SecurityContext {
    pub fn new() -> Self {
        SecurityContext {
            permissions: PermissionSet::new(),
            files: FileAccessPolicy::new(),
        }
    }

    /// Mode dev: izinkan semua akses file (permission tetap deny-by-default).
    pub fn dev_mode() -> Self {
        SecurityContext {
            permissions: PermissionSet::new(),
            files: FileAccessPolicy::permissive(),
        }
    }

    pub fn summary(&self) -> String {
        format!(
            "perms={} file_roots={}",
            self.permissions.allowed_names().len(),
            self.files.allowed_roots().len(),
        )
    }
}

impl Default for SecurityContext {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_security_context() {
        let ctx = SecurityContext::new();
        assert!(!ctx.files.can_access(std::path::Path::new("/etc/passwd")));
        assert!(ctx.summary().contains("perms=0"));
    }
}
