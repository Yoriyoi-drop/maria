//! Plugin System — extensible plugin architecture for Maria.
//!
//! Phase 6+: WASM-based plugin system (stub for now).
//! Provides hook points for custom analysis passes, formatters, etc.

use std::collections::HashMap;

// ─── Plugin Trait ───

/// Trait that all plugins must implement.
pub trait Plugin: Send + Sync {
    /// Plugin name.
    fn name(&self) -> &str;

    /// Plugin version.
    fn version(&self) -> &str;

    /// Called when plugin is loaded.
    fn on_load(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Called when plugin is unloaded.
    fn on_unload(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Hook: called after parsing a file.
    fn after_parse(&mut self, _file: &str, _ast: &str) -> Result<(), String> {
        Ok(())
    }

    /// Hook: called after elaboration.
    fn after_elaborate(&mut self, _module: &str) -> Result<(), String> {
        Ok(())
    }

    /// Hook: called before simulation starts.
    fn before_simulate(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Hook: called after simulation ends.
    fn after_simulate(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Hook: called on diagnostic emission.
    fn on_diagnostic(&mut self, _level: &str, _code: &str, _message: &str) -> Result<(), String> {
        Ok(())
    }
}

// ─── Plugin Manager ───

/// Manages loaded plugins and dispatches hooks.
pub struct PluginManager {
    plugins: Vec<Box<dyn Plugin>>,
    /// Plugin metadata
    metadata: HashMap<String, PluginMetadata>,
}

#[derive(Debug, Clone)]
pub struct PluginMetadata {
    pub name: String,
    pub version: String,
    pub enabled: bool,
}

impl PluginManager {
    pub fn new() -> Self {
        PluginManager {
            plugins: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    /// Register a plugin.
    pub fn register(&mut self, mut plugin: Box<dyn Plugin>) -> Result<(), String> {
        plugin.on_load()?;
        let name = plugin.name().to_string();
        let version = plugin.version().to_string();

        self.metadata.insert(
            name.clone(),
            PluginMetadata {
                name: name.clone(),
                version,
                enabled: true,
            },
        );

        self.plugins.push(plugin);
        Ok(())
    }

    /// Unregister a plugin by name.
    pub fn unregister(&mut self, name: &str) -> Result<(), String> {
        if let Some(idx) = self.plugins.iter().position(|p| p.name() == name) {
            let mut plugin = self.plugins.remove(idx);
            plugin.on_unload()?;
            self.metadata.remove(name);
            Ok(())
        } else {
            Err(format!("plugin '{}' not found", name))
        }
    }

    /// Dispatch a hook to all enabled plugins.
    pub fn dispatch<F>(&mut self, hook: F)
    where
        F: Fn(&mut dyn Plugin) -> Result<(), String>,
    {
        for plugin in &mut self.plugins {
            if let Some(meta) = self.metadata.get(plugin.name()) {
                if meta.enabled {
                    let _ = hook(plugin.as_mut());
                }
            }
        }
    }

    /// Get list of registered plugins.
    pub fn list(&self) -> Vec<&PluginMetadata> {
        self.metadata.values().collect()
    }

    /// Enable/disable a plugin.
    pub fn set_enabled(&mut self, name: &str, enabled: bool) {
        if let Some(meta) = self.metadata.get_mut(name) {
            meta.enabled = enabled;
        }
    }

    /// Number of registered plugins.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Is empty.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Example Plugin ───

/// A simple example plugin for testing.
pub struct ExamplePlugin {
    name: String,
    parse_count: usize,
}

impl ExamplePlugin {
    pub fn new() -> Self {
        ExamplePlugin {
            name: "example-plugin".to_string(),
            parse_count: 0,
        }
    }
}

impl Plugin for ExamplePlugin {
    fn name(&self) -> &str {
        &self.name
    }

    fn version(&self) -> &str {
        "0.1.0"
    }

    fn after_parse(&mut self, _file: &str, _ast: &str) -> Result<(), String> {
        self.parse_count += 1;
        Ok(())
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_manager_register() {
        let mut pm = PluginManager::new();
        pm.register(Box::new(ExamplePlugin::new())).unwrap();
        assert_eq!(pm.len(), 1);
    }

    #[test]
    fn test_plugin_manager_dispatch() {
        let mut pm = PluginManager::new();
        pm.register(Box::new(ExamplePlugin::new())).unwrap();

        pm.dispatch(|plugin| plugin.after_parse("test.sv", "module test; endmodule"));

        assert_eq!(pm.len(), 1);
    }

    #[test]
    fn test_plugin_manager_unregister() {
        let mut pm = PluginManager::new();
        pm.register(Box::new(ExamplePlugin::new())).unwrap();
        pm.unregister("example-plugin").unwrap();
        assert!(pm.is_empty());
    }
}
