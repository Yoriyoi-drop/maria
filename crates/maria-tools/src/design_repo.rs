//! ENT-38: Design Repository — versioned design storage.
//!
//! Menyediakan repository untuk menyimpan, version, dan query
//! design files. Setiap commit menyimpan metadata (author, timestamp,
//! message, file list, checksums).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Design commit record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignCommit {
    pub hash: String,
    pub author: String,
    pub message: String,
    pub timestamp: u64,
    pub files: Vec<DesignFileInfo>,
    pub parent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignFileInfo {
    pub path: String,
    pub checksum: String,
    pub size: u64,
}

/// Design tag (release marker).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignTag {
    pub name: String,
    pub commit_hash: String,
    pub description: String,
    pub created_at: u64,
}

/// Design repository.
pub struct DesignRepository {
    root: PathBuf,
    commits: Mutex<Vec<DesignCommit>>,
    tags: Mutex<HashMap<String, DesignTag>>,
}

impl DesignRepository {
    pub fn open(root: PathBuf) -> Self {
        DesignRepository {
            root,
            commits: Mutex::new(Vec::new()),
            tags: Mutex::new(HashMap::new()),
        }
    }

    /// Create a new commit.
    pub fn commit(
        &self,
        author: &str,
        message: &str,
        files: Vec<DesignFileInfo>,
    ) -> DesignCommit {
        let commits = self.commits.lock().unwrap();
        let parent = commits.last().map(|c| c.hash.clone());
        let hash = compute_hash(author, message, commits.len());
        drop(commits);

        let commit = DesignCommit {
            hash: hash.clone(),
            author: author.to_string(),
            message: message.to_string(),
            timestamp: now_secs(),
            files,
            parent,
        };

        self.commits.lock().unwrap().push(commit.clone());
        commit
    }

    /// Get commit history.
    pub fn log(&self, max: usize) -> Vec<DesignCommit> {
        self.commits
            .lock()
            .unwrap()
            .iter()
            .rev()
            .take(max)
            .cloned()
            .collect()
    }

    /// Get commit by hash (prefix match).
    pub fn get_commit(&self, hash_prefix: &str) -> Option<DesignCommit> {
        self.commits
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.hash.starts_with(hash_prefix))
            .cloned()
    }

    /// Create a tag.
    pub fn tag(&self, name: &str, commit_hash: &str, description: &str) -> Result<DesignTag, String> {
        let commits = self.commits.lock().unwrap();
        let commit = commits
            .iter()
            .find(|c| c.hash.starts_with(commit_hash))
            .ok_or("commit not found")?;
        let hash = commit.hash.clone();
        drop(commits);

        let tag = DesignTag {
            name: name.to_string(),
            commit_hash: hash,
            description: description.to_string(),
            created_at: now_secs(),
        };

        self.tags
            .lock()
            .unwrap()
            .insert(name.to_string(), tag.clone());
        Ok(tag)
    }

    /// List tags.
    pub fn list_tags(&self) -> Vec<DesignTag> {
        self.tags.lock().unwrap().values().cloned().collect()
    }

    /// Diff between two commits.
    pub fn diff(&self, hash_a: &str, hash_b: &str) -> Option<Vec<String>> {
        let commits = self.commits.lock().unwrap();
        let a = commits.iter().find(|c| c.hash.starts_with(hash_a))?;
        let b = commits.iter().find(|c| c.hash.starts_with(hash_b))?;

        let files_a: HashMap<&str, &DesignFileInfo> =
            a.files.iter().map(|f| (f.path.as_str(), f)).collect();
        let files_b: HashMap<&str, &DesignFileInfo> =
            b.files.iter().map(|f| (f.path.as_str(), f)).collect();

        let mut diffs = Vec::new();
        for (path, fb) in &files_b {
            match files_a.get(path) {
                Some(fa) => {
                    if fa.checksum != fb.checksum {
                        diffs.push(format!("M  {}", path));
                    }
                }
                None => diffs.push(format!("A  {}", path)),
            }
        }
        for path in files_a.keys() {
            if !files_b.contains_key(path) {
                diffs.push(format!("D  {}", path));
            }
        }

        Some(diffs)
    }

    /// Summary.
    pub fn summary(&self) -> String {
        let commits = self.commits.lock().unwrap();
        let tags = self.tags.lock().unwrap();
        format!(
            "DesignRepo: {} commits, {} tags, root: {}",
            commits.len(),
            tags.len(),
            self.root.display()
        )
    }
}

fn compute_hash(author: &str, message: &str, idx: usize) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    author.hash(&mut h);
    message.hash(&mut h);
    idx.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_commit_and_log() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = DesignRepository::open(dir.path().to_path_buf());
        let c1 = repo.commit(
            "alice",
            "initial design",
            vec![DesignFileInfo {
                path: "counter.sv".into(),
                checksum: "abc123".into(),
                size: 1024,
            }],
        );
        assert!(!c1.hash.is_empty());

        let c2 = repo.commit(
            "bob",
            "fix counter",
            vec![DesignFileInfo {
                path: "counter.sv".into(),
                checksum: "def456".into(),
                size: 1030,
            }],
        );
        assert_eq!(repo.log(10).len(), 2);
        assert_eq!(c2.parent, Some(c1.hash.clone()));
    }

    #[test]
    fn test_tag() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = DesignRepository::open(dir.path().to_path_buf());
        let c = repo.commit("alice", "design", vec![]);
        let tag = repo.tag("v1.0", &c.hash, "first release").unwrap();
        assert_eq!(tag.name, "v1.0");
        assert_eq!(repo.list_tags().len(), 1);
    }

    #[test]
    fn test_diff() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = DesignRepository::open(dir.path().to_path_buf());
        let c1 = repo.commit(
            "a",
            "init",
            vec![
                DesignFileInfo { path: "a.sv".into(), checksum: "1".into(), size: 10 },
            ],
        );
        let c2 = repo.commit(
            "a",
            "update",
            vec![
                DesignFileInfo { path: "a.sv".into(), checksum: "2".into(), size: 12 },
                DesignFileInfo { path: "b.sv".into(), checksum: "3".into(), size: 20 },
            ],
        );
        let diffs = repo.diff(&c1.hash, &c2.hash).unwrap();
        assert!(diffs.iter().any(|d| d.contains("M") && d.contains("a.sv")));
        assert!(diffs.iter().any(|d| d.contains("A") && d.contains("b.sv")));
    }

    #[test]
    fn test_summary() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = DesignRepository::open(dir.path().to_path_buf());
        let s = repo.summary();
        assert!(s.contains("0 commits"));
    }
}
