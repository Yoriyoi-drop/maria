/// SandboxPolicy — batasan eksekusi plugin (hook mana yang diizinkan,
/// plugin mana yang dipercaya). Stub keamanan awal; plugin WASM terisolasi
/// menyusul.
#[derive(Debug, Clone)]
pub struct SandboxPolicy {
    /// Apakah hook plugin boleh berjalan (false = semua plugin no-op).
    pub allow_hooks: bool,
    /// Nama plugin yang dipercaya (kosong = tidak ada yang dipercaya khusus).
    trusted: Vec<String>,
}

impl SandboxPolicy {
    pub fn new() -> Self {
        SandboxPolicy { allow_hooks: true, trusted: Vec::new() }
    }

    pub fn restricted() -> Self {
        SandboxPolicy { allow_hooks: false, trusted: Vec::new() }
    }

    pub fn trust(&mut self, name: impl Into<String>) {
        let n = name.into();
        if !self.trusted.contains(&n) {
            self.trusted.push(n);
        }
    }

    pub fn is_trusted(&self, name: &str) -> bool {
        self.trusted.contains(&name.to_string())
    }

    /// Bolehkan plugin `name` menjalankan hook-nya.
    pub fn hook_allowed(&self, name: &str) -> bool {
        self.allow_hooks && (self.trusted.is_empty() || self.is_trusted(name))
    }
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_default_allows_all() {
        let s = SandboxPolicy::new();
        assert!(s.hook_allowed("any"));
    }

    #[test]
    fn test_sandbox_restricted() {
        let s = SandboxPolicy::restricted();
        assert!(!s.hook_allowed("any"));
    }

    #[test]
    fn test_sandbox_trust_list() {
        let mut s = SandboxPolicy::restricted();
        s.allow_hooks = true;
        s.trust("good");
        assert!(s.hook_allowed("good"));
        assert!(!s.hook_allowed("evil"));
    }
}
