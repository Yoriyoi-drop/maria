use crate::env::plugins::{PluginManagerHandle, PluginRegistry, SandboxPolicy};

/// PluginContext — service plugin: manager + registry + sandbox.
///
/// Tidak memiliki logika plugin; hanya menyimpan service yang terpisah
/// tanggung jawab (1 file = 1 tanggung jawab).
#[derive(Debug, Default)]
pub struct PluginContext {
    pub manager: PluginManagerHandle,
    pub sandbox: SandboxPolicy,
}

impl PluginContext {
    pub fn new() -> Self {
        PluginContext {
            manager: PluginManagerHandle::new(),
            sandbox: SandboxPolicy::new(),
        }
    }

    /// Snapshot registry (untuk GUI/CLI).
    pub fn registry(&self) -> PluginRegistry {
        PluginRegistry::from_list(self.manager.metadata())
    }

    pub fn plugin_count(&self) -> usize {
        self.manager.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::ExamplePlugin;

    #[test]
    fn test_plugin_context() {
        let ctx = PluginContext::new();
        assert_eq!(ctx.plugin_count(), 0);
        ctx.manager.register(Box::new(ExamplePlugin::new())).unwrap();
        assert_eq!(ctx.plugin_count(), 1);
        assert_eq!(ctx.registry().active_names(), vec!["example-plugin"]);
    }
}
