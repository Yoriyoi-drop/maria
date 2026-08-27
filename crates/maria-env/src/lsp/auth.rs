//! ENT-14: Basic Authentication for LSP Server.
//!
//! Menyediakan API key-based authentication untuk LSP server.
//! Berguna untuk team collaboration di mana LSP dijalankan sebagai
//! remote service.
//!
//! API key disimpan di environment variable `MARIA_LSP_API_KEY`.
//! Bila tidak di-set, auth disabled (backward compatible).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// API key record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub key: String,
    pub owner: String,
    pub scopes: Vec<String>,
    pub created_at: u64,
    pub expires_at: Option<u64>,
}

/// Auth store — menyimpan dan memvalidasi API keys.
pub struct AuthStore {
    keys: Mutex<HashMap<String, ApiKey>>,
    enabled: bool,
}

impl AuthStore {
    /// Buat auth store baru. Enabled bila ada keys atau env var.
    pub fn new() -> Self {
        let mut store = AuthStore {
            keys: Mutex::new(HashMap::new()),
            enabled: false,
        };

        // Load dari environment variable
        if let Ok(key) = std::env::var("MARIA_LSP_API_KEY") {
            store.add_key(ApiKey {
                key: key.clone(),
                owner: "env".into(),
                scopes: vec!["read".into(), "write".into()],
                created_at: timestamp_now(),
                expires_at: None,
            });
            store.enabled = true;
        }

        // Load dari file
        let config_path = Path::new(".maria/auth.json");
        if config_path.exists() {
            if let Ok(content) = std::fs::read_to_string(config_path) {
                if let Ok(keys) = serde_json::from_str::<Vec<ApiKey>>(&content) {
                    for key in keys {
                        store.add_key(key);
                        store.enabled = true;
                    }
                }
            }
        }

        store
    }

    /// Tambah API key.
    pub fn add_key(&self, key: ApiKey) {
        if let Ok(mut keys) = self.keys.lock() {
            keys.insert(key.key.clone(), key);
        }
    }

    /// Validasi API key.
    pub fn validate(&self, key: &str) -> Option<ApiKey> {
        if !self.enabled {
            // Auth disabled — allow all
            return Some(ApiKey {
                key: key.to_string(),
                owner: "anonymous".into(),
                scopes: vec!["read".into(), "write".into()],
                created_at: 0,
                expires_at: None,
            });
        }

        let keys = self.keys.lock().ok()?;
        let api_key = keys.get(key)?;

        // Check expiry
        if let Some(expires) = api_key.expires_at {
            if timestamp_now() > expires {
                return None;
            }
        }

        Some(api_key.clone())
    }

    /// Check apakah key punya scope tertentu.
    pub fn has_scope(&self, key: &str, scope: &str) -> bool {
        if !self.enabled {
            return true;
        }
        self.validate(key)
            .map(|k| k.scopes.contains(&scope.to_string()))
            .unwrap_or(false)
    }

    /// Apakah auth enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Revoke API key.
    pub fn revoke(&self, key: &str) -> bool {
        if let Ok(mut keys) = self.keys.lock() {
            keys.remove(key).is_some()
        } else {
            false
        }
    }

    /// Save keys ke file.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let keys = self.keys.lock().map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(&keys.values().collect::<Vec<_>>())
            .map_err(|e| format!("gagal serialize: {}", e))?;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(path, &json)
            .map_err(|e| format!("gagal tulis {}: {}", path.display(), e))
    }

    /// Generate API key baru.
    pub fn generate_key(owner: &str, scopes: Vec<String>) -> ApiKey {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        owner.hash(&mut hasher);
        timestamp_now().hash(&mut hasher);
        ApiKey {
            key: format!("{:016x}", hasher.finish()),
            owner: owner.to_string(),
            scopes,
            created_at: timestamp_now(),
            expires_at: None,
        }
    }
}

fn timestamp_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl Default for AuthStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_disabled_by_default() {
        // Clear env var
        std::env::remove_var("MARIA_LSP_API_KEY");
        let store = AuthStore::new();
        assert!(!store.is_enabled());
        // When disabled, any key is valid
        assert!(store.validate("any_key").is_some());
    }

    #[test]
    fn test_auth_with_env_key() {
        std::env::set_var("MARIA_LSP_API_KEY", "test-key-123");
        let store = AuthStore::new();
        assert!(store.is_enabled());
        assert!(store.validate("test-key-123").is_some());
        assert!(store.validate("wrong-key").is_none());
        std::env::remove_var("MARIA_LSP_API_KEY");
    }

    #[test]
    fn test_auth_scopes() {
        std::env::set_var("MARIA_LSP_API_KEY", "scoped-key");
        let store = AuthStore::new();
        assert!(store.has_scope("scoped-key", "read"));
        assert!(!store.has_scope("scoped-key", "admin"));
        std::env::remove_var("MARIA_LSP_API_KEY");
    }

    #[test]
    fn test_auth_revoke() {
        std::env::set_var("MARIA_LSP_API_KEY", "revoke-me");
        let store = AuthStore::new();
        assert!(store.validate("revoke-me").is_some());
        store.revoke("revoke-me");
        assert!(store.validate("revoke-me").is_none());
        std::env::remove_var("MARIA_LSP_API_KEY");
    }

    #[test]
    fn test_generate_key() {
        let key = AuthStore::generate_key("user1", vec!["read".to_string()]);
        assert!(!key.key.is_empty());
        assert_eq!(key.key.len(), 16);
        assert_eq!(key.owner, "user1");
        assert_eq!(key.scopes, vec!["read".to_string()]);
    }

    #[test]
    fn test_auth_save_load() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("auth.json");
        let store = AuthStore::new();
        store.add_key(ApiKey {
            key: "save-me".into(),
            owner: "test".into(),
            scopes: vec!["read".into()],
            created_at: 12345,
            expires_at: None,
        });
        store.save(&path).unwrap();

        let _loaded = AuthStore::new();
        // Note: loaded store won't have the saved keys unless we load them
        // This tests the save mechanism
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("save-me"));
    }
}
