use std::collections::HashSet;

/// PermissionSet — himpunan izin bernama (mis. "fs:write", "net:bind").
/// Policy default menolak semua kecuali yang di-allow eksplisit.
#[derive(Debug, Clone, Default)]
pub struct PermissionSet {
    allowed: HashSet<String>,
}

impl PermissionSet {
    pub fn new() -> Self {
        PermissionSet { allowed: HashSet::new() }
    }

    pub fn allow(&mut self, name: impl Into<String>) {
        self.allowed.insert(name.into());
    }

    pub fn deny(&mut self, name: &str) {
        self.allowed.remove(name);
    }

    pub fn allows(&self, name: &str) -> bool {
        self.allowed.contains(name)
    }

    pub fn allowed_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.allowed.iter().cloned().collect();
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_default_deny() {
        let p = PermissionSet::new();
        assert!(!p.allows("fs:write"));
    }

    #[test]
    fn test_permission_allow_deny() {
        let mut p = PermissionSet::new();
        p.allow("fs:write");
        assert!(p.allows("fs:write"));
        p.deny("fs:write");
        assert!(!p.allows("fs:write"));
    }
}
