//! FEAT-25: Waiver Management — lint/formal violation waiver store.
//!
//! Manages waivers for lint warnings and formal verification violations.
//! Waivers can be scoped by file, rule, instance, or regex pattern.
//! Supports import/export in JSON format and git-compatible diff tracking.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// A single waiver entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Waiver {
    pub id: String,
    pub rule: String,
    pub file_pattern: Option<String>,
    pub instance_pattern: Option<String>,
    pub reason: String,
    pub owner: String,
    pub created_at: u64,
    pub expires_at: Option<u64>,
    pub ticket: Option<String>,
    pub active: bool,
}

/// Waiver match result.
#[derive(Debug, Clone)]
pub struct WaiverMatch {
    pub waiver: Waiver,
    pub confidence: f64,
}

/// Waiver store — manages all waivers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaiverStore {
    pub waivers: Vec<Waiver>,
    pub metadata: StoreMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreMetadata {
    pub version: String,
    pub last_modified: u64,
    pub owner: String,
}

impl Default for StoreMetadata {
    fn default() -> Self {
        StoreMetadata {
            version: "1.0".into(),
            last_modified: now_secs(),
            owner: "unknown".into(),
        }
    }
}

impl WaiverStore {
    pub fn new() -> Self {
        WaiverStore {
            waivers: Vec::new(),
            metadata: StoreMetadata::default(),
        }
    }

    /// Add a waiver.
    pub fn add(&mut self, rule: &str, file_pattern: Option<&str>, reason: &str, owner: &str) -> String {
        let id = format!("W-{:06}", self.waivers.len() + 1);
        let waiver = Waiver {
            id: id.clone(),
            rule: rule.to_string(),
            file_pattern: file_pattern.map(|s| s.to_string()),
            instance_pattern: None,
            reason: reason.to_string(),
            owner: owner.to_string(),
            created_at: now_secs(),
            expires_at: None,
            ticket: None,
            active: true,
        };
        self.waivers.push(waiver);
        self.metadata.last_modified = now_secs();
        id
    }

    /// Check if a violation is waived.
    pub fn is_waived(&self, rule: &str, file: Option<&str>, instance: Option<&str>) -> Option<WaiverMatch> {
        let now = now_secs();
        for w in &self.waivers {
            if !w.active || w.rule != rule {
                continue;
            }
            if let Some(exp) = w.expires_at {
                if now > exp {
                    continue;
                }
            }
            // Check file pattern match
            if let (Some(pattern), Some(f)) = (&w.file_pattern, file) {
                if !simple_match(pattern, f) {
                    continue;
                }
            }
            // Check instance pattern match
            if let (Some(pattern), Some(inst)) = (&w.instance_pattern, instance) {
                if !simple_match(pattern, inst) {
                    continue;
                }
            }
            return Some(WaiverMatch {
                waiver: w.clone(),
                confidence: 1.0,
            });
        }
        None
    }

    /// Deactivate a waiver by ID.
    pub fn deactivate(&mut self, id: &str) -> bool {
        if let Some(w) = self.waivers.iter_mut().find(|w| w.id == id) {
            w.active = false;
            self.metadata.last_modified = now_secs();
            true
        } else {
            false
        }
    }

    /// Count active waivers per rule.
    pub fn summary(&self) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        for w in &self.waivers {
            if w.active {
                *counts.entry(w.rule.clone()).or_insert(0) += 1;
            }
        }
        counts
    }

    /// Export to JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into())
    }

    /// Import from JSON.
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| e.to_string())
    }

    /// Save to file.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = self.to_json();
        std::fs::write(path, json).map_err(|e| e.to_string())
    }

    /// Load from file.
    pub fn load(path: &Path) -> Result<Self, String> {
        let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::from_json(&json)
    }

    /// Remove expired waivers.
    pub fn purge_expired(&mut self) -> usize {
        let now = now_secs();
        let before = self.waivers.len();
        self.waivers.retain(|w| {
            if let Some(exp) = w.expires_at {
                now <= exp
            } else {
                true
            }
        });
        before - self.waivers.len()
    }
}

/// Simple glob-like pattern matching (* and ?).
fn simple_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    glob_match(&p, &t)
}

fn glob_match(pattern: &[char], text: &[char]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }
    if pattern[0] == '*' {
        // Try matching zero or more
        if glob_match(&pattern[1..], text) {
            return true;
        }
        if !text.is_empty() {
            return glob_match(pattern, &text[1..]);
        }
        return false;
    }
    if text.is_empty() {
        return false;
    }
    if pattern[0] == '?' || pattern[0] == text[0] {
        return glob_match(&pattern[1..], &text[1..]);
    }
    false
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_is_waived() {
        let mut store = WaiverStore::new();
        store.add("LINT-001", Some("*.sv"), "known false positive", "alice");
        let m = store.is_waived("LINT-001", Some("counter.sv"), None);
        assert!(m.is_some());
        assert_eq!(m.unwrap().waiver.owner, "alice");
    }

    #[test]
    fn test_not_waived_different_rule() {
        let mut store = WaiverStore::new();
        store.add("LINT-001", None, "test", "bob");
        assert!(store.is_waived("LINT-999", None, None).is_none());
    }

    #[test]
    fn test_deactivate() {
        let mut store = WaiverStore::new();
        let id = store.add("LINT-001", None, "test", "alice");
        assert!(store.deactivate(&id));
        assert!(store.is_waived("LINT-001", None, None).is_none());
    }

    #[test]
    fn test_summary() {
        let mut store = WaiverStore::new();
        store.add("LINT-001", None, "a", "alice");
        store.add("LINT-001", None, "b", "bob");
        store.add("FORMAL-001", None, "c", "carol");
        let s = store.summary();
        assert_eq!(s.get("LINT-001").unwrap(), &2);
        assert_eq!(s.get("FORMAL-001").unwrap(), &1);
    }

    #[test]
    fn test_json_roundtrip() {
        let mut store = WaiverStore::new();
        store.add("LINT-001", Some("*.sv"), "test", "alice");
        let json = store.to_json();
        let restored = WaiverStore::from_json(&json).unwrap();
        assert_eq!(restored.waivers.len(), 1);
        assert_eq!(restored.waivers[0].rule, "LINT-001");
    }

    #[test]
    fn test_glob_match() {
        assert!(simple_match("*.sv", "counter.sv"));
        assert!(simple_match("*.sv", "tb_counter.sv"));
        assert!(!simple_match("*.sv", "counter.v"));
        assert!(simple_match("??", "ab"));
        assert!(!simple_match("??", "a"));
    }

    #[test]
    fn test_purge_expired() {
        let mut store = WaiverStore::new();
        store.add("LINT-001", None, "active", "alice");
        let mut expired = Waiver {
            id: "W-000002".into(),
            rule: "LINT-002".into(),
            file_pattern: None,
            instance_pattern: None,
            reason: "old".into(),
            owner: "bob".into(),
            created_at: 1000,
            expires_at: Some(1001),
            ticket: None,
            active: true,
        };
        store.waivers.push(expired);
        let purged = store.purge_expired();
        assert_eq!(purged, 1);
        assert_eq!(store.waivers.len(), 1);
    }
}
