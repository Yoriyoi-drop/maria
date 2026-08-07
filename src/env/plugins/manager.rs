use crate::plugin::{Plugin, PluginManager, PluginMetadata};
use std::sync::Mutex;

/// Handle ke PluginManager — register/unregister/dispatch thread-safe.
#[derive(Default)]
pub struct PluginManagerHandle {
    inner: Mutex<PluginManager>,
}

impl std::fmt::Debug for PluginManagerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginManagerHandle")
            .field("count", &self.len())
            .finish()
    }
}

impl PluginManagerHandle {
    pub fn new() -> Self {
        PluginManagerHandle { inner: Mutex::new(PluginManager::new()) }
    }

    pub fn register(&self, plugin: Box<dyn Plugin>) -> Result<(), String> {
        self.inner.lock().unwrap().register(plugin)
    }

    pub fn unregister(&self, name: &str) -> Result<(), String> {
        self.inner.lock().unwrap().unregister(name)
    }

    /// Dispatch hook ke semua plugin aktif.
    pub fn dispatch<F>(&self, hook: F)
    where
        F: Fn(&mut dyn Plugin) -> Result<(), String>,
    {
        self.inner.lock().unwrap().dispatch(hook);
    }

    pub fn set_enabled(&self, name: &str, enabled: bool) {
        self.inner.lock().unwrap().set_enabled(name, enabled);
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn metadata(&self) -> Vec<PluginMetadata> {
        self.inner.lock().unwrap().list().into_iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::ExamplePlugin;

    #[test]
    fn test_plugin_handle() {
        let h = PluginManagerHandle::new();
        assert!(h.is_empty());
        h.register(Box::new(ExamplePlugin::new())).unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h.metadata()[0].name, "example-plugin");
        h.dispatch(|p| p.after_parse("a.sv", "module a; endmodule"));
        h.unregister("example-plugin").unwrap();
        assert!(h.is_empty());
    }
}
