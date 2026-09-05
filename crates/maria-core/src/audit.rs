//! ENT-15: Audit Trail — Structured logging untuk CI, formal verification,
//! dan simulasi activities.
//!
//! Menyediakan log terstruktur (JSON) yang bisa dianalisis untuk:
//! - Tracking kompilasi/simulasi yang dilakukan
//! - Debugging regression di CI
//! - Compliance audit trail untuk tape-out

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Level severity untuk audit log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AuditLevel {
    Info,
    Warning,
    Error,
    Critical,
}

impl std::fmt::Display for AuditLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditLevel::Info => write!(f, "INFO"),
            AuditLevel::Warning => write!(f, "WARNING"),
            AuditLevel::Error => write!(f, "ERROR"),
            AuditLevel::Critical => write!(f, "CRITICAL"),
        }
    }
}

/// Satu entry audit log.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuditEntry {
    pub timestamp: u64,
    pub timestamp_ms: u32,
    pub level: AuditLevel,
    pub category: String,
    pub message: String,
    pub details: HashMap<String, String>,
}

/// Audit log collector — thread-safe, menulis ke file JSON lines.
pub struct AuditLog {
    path: PathBuf,
    entries: Mutex<Vec<AuditEntry>>,
    max_entries: usize,
}

impl AuditLog {
    /// Buat audit log baru.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        AuditLog {
            path: path.into(),
            entries: Mutex::new(Vec::new()),
            max_entries: 10000,
        }
    }

    /// Buat audit log dengan batas jumlah entries.
    pub fn with_limit(path: impl Into<PathBuf>, max_entries: usize) -> Self {
        AuditLog {
            path: path.into(),
            entries: Mutex::new(Vec::new()),
            max_entries,
        }
    }

    /// Catat entry audit.
    pub fn log(
        &self,
        level: AuditLevel,
        category: &str,
        message: &str,
        details: HashMap<String, String>,
    ) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let entry = AuditEntry {
            timestamp: now.as_secs(),
            timestamp_ms: now.subsec_millis(),
            level,
            category: category.to_string(),
            message: message.to_string(),
            details,
        };

        if let Ok(mut entries) = self.entries.lock() {
            entries.push(entry);
            // Evict jika melebihi batas
            if entries.len() > self.max_entries {
                let drain_count = entries.len() - self.max_entries;
                entries.drain(..drain_count);
            }
        }
    }

    /// Convenience: log info.
    pub fn info(&self, category: &str, message: &str) {
        self.log(AuditLevel::Info, category, message, HashMap::new());
    }

    /// Convenience: log warning.
    pub fn warn(&self, category: &str, message: &str) {
        self.log(AuditLevel::Warning, category, message, HashMap::new());
    }

    /// Convenience: log error.
    pub fn error(&self, category: &str, message: &str) {
        self.log(AuditLevel::Error, category, message, HashMap::new());
    }

    /// Convenience: log critical.
    pub fn critical(&self, category: &str, message: &str) {
        self.log(AuditLevel::Critical, category, message, HashMap::new());
    }

    /// Catat kompilasi.
    pub fn log_compile(&self, file: &str, modules: usize, duration_ms: u64) {
        let mut details = HashMap::new();
        details.insert("file".into(), file.into());
        details.insert("modules".into(), modules.to_string());
        details.insert("duration_ms".into(), duration_ms.to_string());
        self.log(AuditLevel::Info, "compile", "compile completed", details);
    }

    /// Catat simulasi.
    pub fn log_simulation(&self, cycles: u64, signals: usize, duration_ms: u64) {
        let mut details = HashMap::new();
        details.insert("cycles".into(), cycles.to_string());
        details.insert("signals".into(), signals.to_string());
        details.insert("duration_ms".into(), duration_ms.to_string());
        self.log(
            AuditLevel::Info,
            "simulation",
            "simulation completed",
            details,
        );
    }

    /// Catat formal verification.
    pub fn log_formal(&self, assertions: usize, result: &str, bound: u64) {
        let mut details = HashMap::new();
        details.insert("assertions".into(), assertions.to_string());
        details.insert("result".into(), result.into());
        details.insert("bound".into(), bound.to_string());
        self.log(
            AuditLevel::Info,
            "formal",
            "formal check completed",
            details,
        );
    }

    /// Flush semua entries ke file (JSON lines format).
    pub fn flush(&self) -> Result<usize, String> {
        let entries = {
            if let Ok(mut e) = self.entries.lock() {
                let taken: Vec<AuditEntry> = e.drain(..).collect();
                taken
            } else {
                return Err("gagal lock audit log".into());
            }
        };

        if entries.is_empty() {
            return Ok(0);
        }

        // Buat parent directory bila perlu
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        // Append mode: buka file, tulis entries baru
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("gagal buka {}: {}", self.path.display(), e))?;

        let count = entries.len();
        for entry in &entries {
            let json =
                serde_json::to_string(entry).map_err(|e| format!("gagal serialize: {}", e))?;
            writeln!(file, "{}", json).map_err(|e| format!("gagal tulis: {}", e))?;
        }

        Ok(count)
    }

    /// Baca semua entries dari file.
    pub fn load(&self) -> Result<Vec<AuditEntry>, String> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let content = std::fs::read_to_string(&self.path)
            .map_err(|e| format!("gagal baca {}: {}", self.path.display(), e))?;
        let mut entries = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(entry) = serde_json::from_str::<AuditEntry>(line) {
                entries.push(entry);
            }
        }
        Ok(entries)
    }

    /// Dapatkan jumlah entries yang belum di-flush.
    pub fn pending_count(&self) -> usize {
        self.entries.lock().map(|e| e.len()).unwrap_or(0)
    }

    /// Dapatkan path file audit log.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Default path untuk audit log.
pub fn default_audit_path() -> PathBuf {
    PathBuf::from(".maria/audit.log.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_log_info() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("audit.jsonl");
        let log = AuditLog::new(&path);
        log.info("test", "hello world");
        assert_eq!(log.pending_count(), 1);
        let flushed = log.flush().unwrap();
        assert_eq!(flushed, 1);
        let entries = log.load().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].category, "test");
        assert_eq!(entries[0].message, "hello world");
        assert_eq!(entries[0].level, AuditLevel::Info);
    }

    #[test]
    fn test_audit_log_levels() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("audit.jsonl");
        let log = AuditLog::new(&path);
        log.info("c", "i");
        log.warn("c", "w");
        log.error("c", "e");
        log.critical("c", "c");
        log.flush().unwrap();
        let entries = log.load().unwrap();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].level, AuditLevel::Info);
        assert_eq!(entries[1].level, AuditLevel::Warning);
        assert_eq!(entries[2].level, AuditLevel::Error);
        assert_eq!(entries[3].level, AuditLevel::Critical);
    }

    #[test]
    fn test_audit_log_compile() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("audit.jsonl");
        let log = AuditLog::new(&path);
        log.log_compile("test.sv", 10, 500);
        log.flush().unwrap();
        let entries = log.load().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].category, "compile");
        assert_eq!(entries[0].details["modules"], "10");
        assert_eq!(entries[0].details["duration_ms"], "500");
    }

    #[test]
    fn test_audit_log_eviction() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("audit.jsonl");
        let log = AuditLog::with_limit(&path, 3);
        for i in 0..5 {
            log.info("test", &format!("entry {}", i));
        }
        assert_eq!(log.pending_count(), 3); // evicted 2 oldest
    }

    #[test]
    fn test_audit_log_load_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.jsonl");
        let log = AuditLog::new(&path);
        let entries = log.load().unwrap();
        assert!(entries.is_empty());
    }
}
