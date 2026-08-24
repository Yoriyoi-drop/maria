use std::path::{Path, PathBuf};

/// FileAccessPolicy — batasi akses file ke root yang diizinkan.
/// `allow_any = true` menonaktifkan pembatasan (mode lokal/dev).
#[derive(Debug, Clone)]
pub struct FileAccessPolicy {
    allowed_roots: Vec<PathBuf>,
    pub allow_any: bool,
}

impl FileAccessPolicy {
    pub fn new() -> Self {
        FileAccessPolicy {
            allowed_roots: Vec::new(),
            allow_any: false,
        }
    }

    pub fn permissive() -> Self {
        FileAccessPolicy {
            allowed_roots: Vec::new(),
            allow_any: true,
        }
    }

    pub fn allow_root(&mut self, root: impl Into<PathBuf>) {
        let r = root.into();
        if !self.allowed_roots.contains(&r) {
            self.allowed_roots.push(r);
        }
    }

    pub fn allowed_roots(&self) -> &[PathBuf] {
        &self.allowed_roots
    }

    /// Apakah `path` boleh diakses?
    pub fn can_access(&self, path: &Path) -> bool {
        if self.allow_any {
            return true;
        }
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(path)
        };
        self.allowed_roots.iter().any(|r| path.starts_with(r))
    }
}

impl Default for FileAccessPolicy {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_denies_outside_root() {
        let mut p = FileAccessPolicy::new();
        p.allow_root("/data/proj");
        assert!(p.can_access(Path::new("/data/proj/rtl/a.sv")));
        assert!(!p.can_access(Path::new("/etc/passwd")));
    }

    #[test]
    fn test_policy_permissive() {
        let p = FileAccessPolicy::permissive();
        assert!(p.can_access(Path::new("/anywhere")));
    }
}
