//! ENT-40: Plugin/Extension Marketplace — discovery, manifest, registry.
//!
//! Menyediakan sistem plugin untuk Maria:
//! - Plugin manifest format (maria-plugin.toml)
//! - Plugin discovery dari directory
//! - Plugin registry (install/uninstall/list/update)
//! - Feature flags per-plugin
//!
//! Format manifest:
//! ```toml
//! [plugin]
//! name = "my-plugin"
//! version = "1.0.0"
//! description = "Does cool things"
//! author = "someone"
//! min_maria_version = "0.3.0"
//! features = ["simulation", "coverage"]
//! ```

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

/// Plugin manifest — metadata dari maria-plugin.toml.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    #[serde(default = "default_version")]
    pub min_maria_version: String,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub entry_point: Option<String>,
}

fn default_version() -> String {
    "0.1.0".into()
}

impl PluginManifest {
    /// Parse dari TOML string.
    pub fn from_toml(toml_str: &str) -> Result<Self, String> {
        toml::from_str(toml_str).map_err(|e| format!("failed to parse plugin manifest: {}", e))
    }

    /// Load dari file.
    pub fn load(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;
        Self::from_toml(&content)
    }

    /// Serialize ke TOML.
    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| format!("failed to serialize: {}", e))
    }

    /// Check compatibility.
    pub fn is_compatible(&self, maria_version: &str) -> bool {
        version_ge(maria_version, &self.min_maria_version)
    }
}

/// Installed plugin info.
#[derive(Debug, Clone)]
pub struct InstalledPlugin {
    pub manifest: PluginManifest,
    pub install_path: PathBuf,
    pub enabled: bool,
}

/// Plugin registry — kelola plugin yang ter-install.
pub struct PluginRegistry {
    registry_dir: PathBuf,
    plugins: HashMap<String, InstalledPlugin>,
}

impl PluginRegistry {
    /// Buat registry baru.
    pub fn new(registry_dir: PathBuf) -> Self {
        let mut registry = PluginRegistry {
            registry_dir,
            plugins: HashMap::new(),
        };
        registry.scan();
        registry
    }

    /// Scan directory untuk plugin yang ter-install.
    pub fn scan(&mut self) {
        self.plugins.clear();
        if !self.registry_dir.exists() {
            return;
        }

        if let Ok(entries) = std::fs::read_dir(&self.registry_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let manifest_path = path.join("maria-plugin.toml");
                    if manifest_path.exists() {
                        if let Ok(manifest) = PluginManifest::load(&manifest_path) {
                            let name = manifest.name.clone();
                            self.plugins.insert(
                                name,
                                InstalledPlugin {
                                    manifest,
                                    install_path: path,
                                    enabled: true,
                                },
                            );
                        }
                    }
                }
            }
        }
    }

    /// Install plugin dari source directory.
    pub fn install(&mut self, source_dir: &Path) -> Result<String, String> {
        let manifest_path = source_dir.join("maria-plugin.toml");
        let manifest = PluginManifest::load(&manifest_path)?;

        let target = self.registry_dir.join(&manifest.name);

        // Copy plugin directory
        if target.exists() {
            std::fs::remove_dir_all(&target)
                .map_err(|e| format!("failed to remove old plugin: {}", e))?;
        }
        copy_dir_recursive(source_dir, &target)
            .map_err(|e| format!("failed to copy plugin: {}", e))?;

        self.plugins.insert(
            manifest.name.clone(),
            InstalledPlugin {
                manifest: manifest.clone(),
                install_path: target,
                enabled: true,
            },
        );

        Ok(format!(
            "installed {} v{}",
            manifest.name, manifest.version
        ))
    }

    /// Uninstall plugin.
    pub fn uninstall(&mut self, name: &str) -> Result<String, String> {
        let plugin = self
            .plugins
            .remove(name)
            .ok_or_else(|| format!("plugin '{}' not found", name))?;

        if plugin.install_path.exists() {
            std::fs::remove_dir_all(&plugin.install_path)
                .map_err(|e| format!("failed to remove plugin: {}", e))?;
        }

        Ok(format!("uninstalled {} v{}", name, plugin.manifest.version))
    }

    /// List all installed plugins.
    pub fn list(&self) -> Vec<&InstalledPlugin> {
        self.plugins.values().collect()
    }

    /// Get plugin by name.
    pub fn get(&self, name: &str) -> Option<&InstalledPlugin> {
        self.plugins.get(name)
    }

    /// Enable/disable plugin.
    pub fn set_enabled(&mut self, name: &str, enabled: bool) -> Result<(), String> {
        if let Some(plugin) = self.plugins.get_mut(name) {
            plugin.enabled = enabled;
            Ok(())
        } else {
            Err(format!("plugin '{}' not found", name))
        }
    }

    /// Check apakah plugin tersedia untuk fitur tertentu.
    pub fn plugins_for_feature(&self, feature: &str) -> Vec<&InstalledPlugin> {
        self.plugins
            .values()
            .filter(|p| p.enabled && p.manifest.features.iter().any(|f| f == feature))
            .collect()
    }

    /// Resolve dependency order.
    pub fn resolve_order(&self) -> Result<Vec<String>, String> {
        let names: Vec<&str> = self.plugins.keys().map(|s| s.as_str()).collect();
        let deps: HashMap<&str, Vec<&str>> = names
            .iter()
            .map(|&name| {
                let plugin = &self.plugins[name];
                let dep_names: Vec<&str> = plugin
                    .manifest
                    .dependencies
                    .iter()
                    .filter(|d| names.contains(&d.as_str()))
                    .map(|s| s.as_str())
                    .collect();
                (name, dep_names)
            })
            .collect();

        topological_sort(&deps)
    }

    /// Summary stats.
    pub fn summary(&self) -> String {
        let total = self.plugins.len();
        let enabled = self.plugins.values().filter(|p| p.enabled).count();
        format!("{} plugins installed, {} enabled", total, enabled)
    }
}

/// Compare version strings (semver-ish: major.minor.patch).
fn version_ge(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u32> {
        s.split('.')
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    let va = parse(a);
    let vb = parse(b);

    for i in 0..va.len().max(vb.len()) {
        let a_val = va.get(i).copied().unwrap_or(0);
        let b_val = vb.get(i).copied().unwrap_or(0);
        if a_val > b_val {
            return true;
        }
        if a_val < b_val {
            return false;
        }
    }
    true // equal
}

/// Topological sort for dependency resolution.
fn topological_sort(deps: &HashMap<&str, Vec<&str>>) -> Result<Vec<String>, String> {
    let mut in_degree: HashMap<&str, usize> = deps.keys().map(|&k| (k, 0)).collect();

    for (_, dep_list) in deps {
        for &dep in dep_list {
            if let Some(_d) = in_degree.get_mut(dep) {
                // This doesn't work directly — we need reverse mapping
            }
        }
    }

    // Build reverse: who depends on whom
    let mut reverse: HashMap<&str, Vec<&str>> = HashMap::new();
    for (&name, dep_list) in deps {
        for &dep in dep_list {
            reverse.entry(dep).or_default().push(name);
        }
    }

    // Fix in_degree: in_degree[name] = number of deps name has (that are in the set)
    for (&name, dep_list) in deps {
        *in_degree.get_mut(name).unwrap() = dep_list.len();
    }

    let mut queue: VecDeque<&str> = in_degree
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(&name, _)| name)
        .collect();

    let mut order = Vec::new();
    while let Some(name) = queue.pop_front() {
        order.push(name.to_string());
        if let Some(dependents) = reverse.get(name) {
            for &dep_name in dependents {
                let d = in_degree.get_mut(dep_name).unwrap();
                *d -= 1;
                if *d == 0 {
                    queue.push_back(dep_name);
                }
            }
        }
    }

    if order.len() != deps.len() {
        return Err("circular dependency detected".into());
    }

    Ok(order)
}

/// Simple recursive directory copy.
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_plugin_dir(dir: &Path, name: &str, version: &str, deps: &[&str]) {
        let plugin_dir = dir.join(name);
        std::fs::create_dir_all(&plugin_dir).unwrap();
        let deps_toml: Vec<String> = deps.iter().map(|d| format!("\"{}\"", d)).collect();
        let toml = format!(
            "name = \"{}\"\nversion = \"{}\"\ndescription = \"test plugin\"\nauthor = \"test\"\nmin_maria_version = \"0.1.0\"\nfeatures = [\"simulation\"]\ndependencies = [{}]\n",
            name,
            version,
            deps_toml.join(", ")
        );
        std::fs::write(plugin_dir.join("maria-plugin.toml"), toml).unwrap();
    }

    #[test]
    fn test_manifest_parse() {
        let toml = "name = \"my-plugin\"\nversion = \"1.0.0\"\ndescription = \"Cool plugin\"\nauthor = \"someone\"\nmin_maria_version = \"0.3.0\"\nfeatures = [\"simulation\", \"coverage\"]\ndependencies = []\n";
        let manifest = PluginManifest::from_toml(toml).unwrap();
        assert_eq!(manifest.name, "my-plugin");
        assert_eq!(manifest.features.len(), 2);
    }

    #[test]
    fn test_version_compatibility() {
        let toml = "name = \"p\"\nversion = \"1.0.0\"\ndescription = \"d\"\nauthor = \"a\"\nmin_maria_version = \"0.3.0\"\nfeatures = []\ndependencies = []\n";
        let manifest = PluginManifest::from_toml(toml).unwrap();
        assert!(manifest.is_compatible("0.3.0"));
        assert!(manifest.is_compatible("1.0.0"));
        assert!(!manifest.is_compatible("0.2.0"));
    }

    #[test]
    fn test_registry_install_uninstall() {
        let dir = TempDir::new().unwrap();
        let source = dir.path().join("source");
        let registry = dir.path().join("registry");

        make_plugin_dir(&source, "test-plugin", "1.0.0", &[]);

        let mut reg = PluginRegistry::new(registry.clone());
        let result = reg.install(&source.join("test-plugin")).unwrap();
        assert!(result.contains("installed"));
        assert_eq!(reg.list().len(), 1);

        reg.uninstall("test-plugin").unwrap();
        assert_eq!(reg.list().len(), 0);
    }

    #[test]
    fn test_feature_filter() {
        let dir = TempDir::new().unwrap();
        let source = dir.path().join("source");
        let registry = dir.path().join("registry");

        make_plugin_dir(&source, "sim-plugin", "1.0.0", &[]);
        let mut reg = PluginRegistry::new(registry);
        reg.install(&source.join("sim-plugin")).unwrap();

        let sim_plugins = reg.plugins_for_feature("simulation");
        assert_eq!(sim_plugins.len(), 1);
        let lint_plugins = reg.plugins_for_feature("lint");
        assert!(lint_plugins.is_empty());
    }

    #[test]
    fn test_dependency_order() {
        let dir = TempDir::new().unwrap();
        let source = dir.path().join("source");
        let registry = dir.path().join("registry");

        make_plugin_dir(&source, "base", "1.0.0", &[]);
        make_plugin_dir(&source, "top", "1.0.0", &["base"]);

        let mut reg = PluginRegistry::new(registry);
        reg.install(&source.join("top")).unwrap();
        reg.install(&source.join("base")).unwrap();

        let order = reg.resolve_order().unwrap();
        let base_pos = order.iter().position(|n| n == "base").unwrap();
        let top_pos = order.iter().position(|n| n == "top").unwrap();
        assert!(base_pos < top_pos);
    }

    #[test]
    fn test_summary() {
        let dir = TempDir::new().unwrap();
        let registry = dir.path().join("registry");

        let mut reg = PluginRegistry::new(registry);
        let summary = reg.summary();
        assert!(summary.contains("0 plugins"));
    }

    #[test]
    fn test_version_compare() {
        assert!(version_ge("1.0.0", "0.9.0"));
        assert!(version_ge("1.0.0", "1.0.0"));
        assert!(!version_ge("0.9.0", "1.0.0"));
        assert!(version_ge("10.0.0", "9.9.9"));
    }
}
