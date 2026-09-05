//! ENT-36: Multi-Project Management — manage multiple sub-projects
//! dalam satu workspace. File: `.maria/workspace.toml`
//!
//! Format:
//! ```toml
//! [[projects]]
//! name = "rtl-core"
//! path = "rtl/"
//! top = "top_module"
//!
//! [[projects]]
//! name = "testbench"
//! path = "tb/"
//! top = "tb_top"
//! depends = ["rtl-core"]
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Workspace configuration — daftar sub-projects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub projects: Vec<ProjectEntry>,
    #[serde(default)]
    pub settings: WorkspaceSettings,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceSettings {
    /// Default parallel jobs
    pub jobs: Option<usize>,
    /// Default max simulation time
    pub max_time: Option<u64>,
    /// Default output directory
    pub output_dir: Option<String>,
}

/// Satu sub-project dalam workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub top: Option<String>,
    #[serde(default)]
    pub depends: Vec<String>,
    #[serde(default)]
    pub incdirs: Vec<String>,
    #[serde(default)]
    pub defines: Vec<String>,
    #[serde(default)]
    pub features: Vec<String>,
}

/// Hasil parse workspace config.
pub struct WorkspaceAnalysis {
    pub config: WorkspaceConfig,
    pub project_dirs: HashMap<String, PathBuf>,
    pub dependency_order: Vec<String>,
    pub errors: Vec<String>,
}

impl WorkspaceConfig {
    /// Load workspace config dari file.
    pub fn load(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("gagal baca {}: {}", path.display(), e))?;
        toml::from_str(&content).map_err(|e| format!("workspace config invalid: {}", e))
    }

    /// Save workspace config.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let content =
            toml::to_string_pretty(self).map_err(|e| format!("gagal serialize: {}", e))?;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(path, &content).map_err(|e| format!("gagal tulis {}: {}", path.display(), e))
    }

    /// Create default workspace config.
    pub fn default_config() -> Self {
        WorkspaceConfig {
            projects: Vec::new(),
            settings: WorkspaceSettings::default(),
        }
    }

    /// Add a project entry.
    pub fn add_project(&mut self, entry: ProjectEntry) {
        // Remove existing entry with same name
        self.projects.retain(|p| p.name != entry.name);
        self.projects.push(entry);
    }

    /// Remove a project by name.
    pub fn remove_project(&mut self, name: &str) -> bool {
        let len_before = self.projects.len();
        self.projects.retain(|p| p.name != name);
        self.projects.len() < len_before
    }

    /// Get project by name.
    pub fn get_project(&self, name: &str) -> Option<&ProjectEntry> {
        self.projects.iter().find(|p| p.name == name)
    }

    /// Analyze workspace: resolve paths, check dependencies, compute order.
    pub fn analyze(&self, workspace_root: &Path) -> WorkspaceAnalysis {
        let mut project_dirs = HashMap::new();
        let mut errors = Vec::new();

        // Resolve project paths
        for proj in &self.projects {
            let dir = workspace_root.join(&proj.path);
            if dir.exists() {
                project_dirs.insert(proj.name.clone(), dir);
            } else {
                errors.push(format!(
                    "project '{}' path not found: {}",
                    proj.name, proj.path
                ));
            }
        }

        // Topological sort of dependencies
        let dependency_order = match topological_sort(&self.projects) {
            Ok(order) => order,
            Err(cycle) => {
                errors.push(format!("circular dependency detected: {}", cycle));
                self.projects.iter().map(|p| p.name.clone()).collect()
            }
        };

        WorkspaceAnalysis {
            config: self.clone(),
            project_dirs,
            dependency_order,
            errors,
        }
    }
}

/// Topological sort of projects based on dependencies.
fn topological_sort(projects: &[ProjectEntry]) -> Result<Vec<String>, String> {
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

    for proj in projects {
        in_degree.entry(proj.name.as_str()).or_insert(0);
        for dep in &proj.depends {
            // proj depends on dep => proj has incoming edge => proj in_degree++
            *in_degree.entry(proj.name.as_str()).or_insert(0) += 1;
            dependents
                .entry(dep.as_str())
                .or_default()
                .push(proj.name.as_str());
        }
    }

    let mut queue: Vec<&str> = in_degree
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(&name, _)| name)
        .collect();

    let mut order = Vec::new();
    while let Some(name) = queue.pop() {
        order.push(name.to_string());
        if let Some(deps) = dependents.get(name) {
            for &dep in deps {
                let degree = in_degree.get_mut(dep).unwrap();
                *degree -= 1;
                if *degree == 0 {
                    queue.push(dep);
                }
            }
        }
    }

    if order.len() != projects.len() {
        // Find the cycle
        let remaining: Vec<String> = projects
            .iter()
            .filter(|p| !order.contains(&p.name))
            .map(|p| p.name.clone())
            .collect();
        return Err(remaining.join(" → "));
    }

    Ok(order)
}

/// List all .maria project files in a directory tree.
pub fn discover_projects(root: &Path) -> Vec<PathBuf> {
    let mut projects = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Check for .maria file in subdirectory
                let maria_file = path.join("proj.maria");
                if maria_file.exists() {
                    projects.push(maria_file);
                }
                // Recurse into subdirectory (max depth 2)
                if let Ok(sub_entries) = std::fs::read_dir(&path) {
                    for sub in sub_entries.flatten() {
                        let sub_path = sub.path();
                        if sub_path.is_dir() {
                            let sub_maria = sub_path.join("proj.maria");
                            if sub_maria.exists() {
                                projects.push(sub_maria);
                            }
                        }
                    }
                }
            }
        }
    }
    projects
}

/// Export workspace summary sebagai JSON.
pub fn workspace_summary(config: &WorkspaceConfig, root: &Path) -> String {
    let analysis = config.analyze(root);
    let summary = serde_json::json!({
        "projects": config.projects.len(),
        "dependency_order": analysis.dependency_order,
        "errors": analysis.errors,
        "settings": {
            "jobs": config.settings.jobs,
            "max_time": config.settings.max_time,
        },
    });
    serde_json::to_string_pretty(&summary).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_config_create() {
        let mut config = WorkspaceConfig::default_config();
        config.add_project(ProjectEntry {
            name: "core".into(),
            path: "rtl/".into(),
            top: Some("top".into()),
            depends: vec![],
            incdirs: vec![],
            defines: vec![],
            features: vec![],
        });
        config.add_project(ProjectEntry {
            name: "tb".into(),
            path: "tb/".into(),
            top: Some("tb_top".into()),
            depends: vec!["core".into()],
            incdirs: vec![],
            defines: vec![],
            features: vec![],
        });
        assert_eq!(config.projects.len(), 2);
    }

    #[test]
    fn test_workspace_save_load() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("workspace.toml");
        let mut config = WorkspaceConfig::default_config();
        config.add_project(ProjectEntry {
            name: "test".into(),
            path: "src/".into(),
            top: None,
            depends: vec![],
            incdirs: vec![],
            defines: vec![],
            features: vec![],
        });
        config.save(&path).unwrap();
        let loaded = WorkspaceConfig::load(&path).unwrap();
        assert_eq!(loaded.projects.len(), 1);
        assert_eq!(loaded.projects[0].name, "test");
    }

    #[test]
    fn test_topological_sort() {
        let projects = vec![
            ProjectEntry {
                name: "c".into(),
                path: "".into(),
                top: None,
                depends: vec!["a".into(), "b".into()],
                incdirs: vec![],
                defines: vec![],
                features: vec![],
            },
            ProjectEntry {
                name: "a".into(),
                path: "".into(),
                top: None,
                depends: vec![],
                incdirs: vec![],
                defines: vec![],
                features: vec![],
            },
            ProjectEntry {
                name: "b".into(),
                path: "".into(),
                top: None,
                depends: vec!["a".into()],
                incdirs: vec![],
                defines: vec![],
                features: vec![],
            },
        ];
        let order = topological_sort(&projects).unwrap();
        assert_eq!(order.len(), 3);
        // a must come before b and c
        let a_pos = order.iter().position(|n| n == "a").unwrap();
        let b_pos = order.iter().position(|n| n == "b").unwrap();
        let c_pos = order.iter().position(|n| n == "c").unwrap();
        assert!(a_pos < b_pos);
        assert!(a_pos < c_pos);
        assert!(b_pos < c_pos);
    }

    #[test]
    fn test_topological_sort_cycle() {
        let projects = vec![
            ProjectEntry {
                name: "a".into(),
                path: "".into(),
                top: None,
                depends: vec!["b".into()],
                incdirs: vec![],
                defines: vec![],
                features: vec![],
            },
            ProjectEntry {
                name: "b".into(),
                path: "".into(),
                top: None,
                depends: vec!["a".into()],
                incdirs: vec![],
                defines: vec![],
                features: vec![],
            },
        ];
        let result = topological_sort(&projects);
        assert!(result.is_err());
    }

    #[test]
    fn test_workspace_remove_project() {
        let mut config = WorkspaceConfig::default_config();
        config.add_project(ProjectEntry {
            name: "a".into(),
            path: "".into(),
            top: None,
            depends: vec![],
            incdirs: vec![],
            defines: vec![],
            features: vec![],
        });
        assert!(config.remove_project("a"));
        assert!(!config.remove_project("nonexistent"));
        assert!(config.projects.is_empty());
    }
}
