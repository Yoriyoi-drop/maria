//! FEAT-21: ECO Support — Engineering Change Order tracking.
//!
//! Manages RTL modifications during tapeout phases. Tracks what changed,
//! why, who approved it, and impact analysis.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// ECO severity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EcoSeverity {
    Critical, // silicon bug, must fix
    Major,    // functional issue
    Minor,    // timing/cleanup
    Cosmetic, // style only
}

/// ECO status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EcoStatus {
    Draft,
    Submitted,
    Reviewed,
    Approved,
    Implemented,
    Verified,
    Closed,
    Rejected,
}

/// A single Engineering Change Order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EcoEntry {
    pub id: String,
    pub title: String,
    pub description: String,
    pub severity: EcoSeverity,
    pub status: EcoStatus,
    pub author: String,
    pub reviewer: Option<String>,
    pub affected_files: Vec<String>,
    pub affected_modules: Vec<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub closed_at: Option<u64>,
    pub risk_score: f64, // 0.0-1.0
    pub regression_required: bool,
    pub verification_required: bool,
    pub comments: Vec<EcoComment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EcoComment {
    pub author: String,
    pub text: String,
    pub timestamp: u64,
}

/// ECO database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EcoDb {
    pub entries: Vec<EcoEntry>,
    pub next_id: u32,
}

impl EcoDb {
    pub fn new() -> Self {
        EcoDb {
            entries: Vec::new(),
            next_id: 1,
        }
    }

    /// Create a new ECO.
    pub fn create(
        &mut self,
        title: &str,
        description: &str,
        severity: EcoSeverity,
        author: &str,
    ) -> String {
        let id = format!("ECO-{:04}", self.next_id);
        self.next_id += 1;
        let now = now_secs();
        self.entries.push(EcoEntry {
            id: id.clone(),
            title: title.to_string(),
            description: description.to_string(),
            severity,
            status: EcoStatus::Draft,
            author: author.to_string(),
            reviewer: None,
            affected_files: Vec::new(),
            affected_modules: Vec::new(),
            created_at: now,
            updated_at: now,
            closed_at: None,
            risk_score: 0.0,
            regression_required: true,
            verification_required: true,
            comments: Vec::new(),
        });
        id
    }

    /// Transition ECO status.
    pub fn transition(&mut self, id: &str, new_status: EcoStatus) -> Result<(), String> {
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| format!("ECO {} not found", id))?;
        let valid = match (&entry.status, &new_status) {
            (EcoStatus::Draft, EcoStatus::Submitted) => true,
            (EcoStatus::Submitted, EcoStatus::Reviewed) => true,
            (EcoStatus::Reviewed, EcoStatus::Approved) => true,
            (EcoStatus::Reviewed, EcoStatus::Rejected) => true,
            (EcoStatus::Approved, EcoStatus::Implemented) => true,
            (EcoStatus::Implemented, EcoStatus::Verified) => true,
            (EcoStatus::Verified, EcoStatus::Closed) => true,
            _ => false,
        };
        if !valid {
            return Err(format!(
                "Invalid transition {:?} -> {:?}",
                entry.status, new_status
            ));
        }
        if new_status == EcoStatus::Closed {
            entry.closed_at = Some(now_secs());
        }
        entry.status = new_status;
        entry.updated_at = now_secs();
        Ok(())
    }

    /// Add a comment.
    pub fn comment(&mut self, id: &str, author: &str, text: &str) -> Result<(), String> {
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| format!("ECO {} not found", id))?;
        entry.comments.push(EcoComment {
            author: author.to_string(),
            text: text.to_string(),
            timestamp: now_secs(),
        });
        entry.updated_at = now_secs();
        Ok(())
    }

    /// List open ECOs.
    pub fn open_ecos(&self) -> Vec<&EcoEntry> {
        self.entries
            .iter()
            .filter(|e| !matches!(e.status, EcoStatus::Closed | EcoStatus::Rejected))
            .collect()
    }

    /// List by severity.
    pub fn by_severity(&self, sev: &EcoSeverity) -> Vec<&EcoEntry> {
        self.entries.iter().filter(|e| &e.severity == sev).collect()
    }

    /// Summary.
    pub fn summary(&self) -> String {
        let total = self.entries.len();
        let open = self.open_ecos().len();
        let critical = self
            .by_severity(&EcoSeverity::Critical)
            .iter()
            .filter(|e| !matches!(e.status, EcoStatus::Closed | EcoStatus::Rejected))
            .count();
        format!(
            "ECO: {} total, {} open, {} critical open",
            total, open, critical
        )
    }

    /// Save to JSON.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())
    }

    /// Load from JSON.
    pub fn load(path: &Path) -> Result<Self, String> {
        let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&json).map_err(|e| e.to_string())
    }
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
    fn test_create_eco() {
        let mut db = EcoDb::new();
        let id = db.create(
            "Fix counter overflow",
            "Counter wraps at max",
            EcoSeverity::Critical,
            "alice",
        );
        assert_eq!(id, "ECO-0001");
        assert_eq!(db.entries.len(), 1);
    }

    #[test]
    fn test_transition_valid() {
        let mut db = EcoDb::new();
        let id = db.create("test", "desc", EcoSeverity::Major, "alice");
        assert!(db.transition(&id, EcoStatus::Submitted).is_ok());
        assert!(db.transition(&id, EcoStatus::Reviewed).is_ok());
        assert!(db.transition(&id, EcoStatus::Approved).is_ok());
        assert_eq!(db.entries[0].status, EcoStatus::Approved);
    }

    #[test]
    fn test_transition_invalid() {
        let mut db = EcoDb::new();
        let id = db.create("test", "desc", EcoSeverity::Major, "alice");
        // Draft -> Implemented is invalid
        assert!(db.transition(&id, EcoStatus::Implemented).is_err());
    }

    #[test]
    fn test_comment() {
        let mut db = EcoDb::new();
        let id = db.create("test", "desc", EcoSeverity::Major, "alice");
        db.comment(&id, "bob", "Looks good").unwrap();
        assert_eq!(db.entries[0].comments.len(), 1);
    }

    #[test]
    fn test_open_ecos() {
        let mut db = EcoDb::new();
        let id1 = db.create("t1", "d1", EcoSeverity::Major, "alice");
        let id2 = db.create("t2", "d2", EcoSeverity::Minor, "bob");
        db.transition(&id1, EcoStatus::Submitted).unwrap();
        db.transition(&id1, EcoStatus::Reviewed).unwrap();
        db.transition(&id1, EcoStatus::Approved).unwrap();
        db.transition(&id1, EcoStatus::Implemented).unwrap();
        db.transition(&id1, EcoStatus::Verified).unwrap();
        db.transition(&id1, EcoStatus::Closed).unwrap();
        assert_eq!(db.open_ecos().len(), 1);
        assert_eq!(db.open_ecos()[0].id, id2);
    }

    #[test]
    fn test_summary() {
        let mut db = EcoDb::new();
        db.create("t1", "d1", EcoSeverity::Critical, "alice");
        db.create("t2", "d2", EcoSeverity::Minor, "bob");
        let s = db.summary();
        assert!(s.contains("2 total"));
        assert!(s.contains("1 critical open"));
    }
}
