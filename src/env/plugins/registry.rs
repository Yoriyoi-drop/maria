use crate::plugin::PluginMetadata;

/// PluginRegistry — snapshot metadata plugin yang terdaftar.
#[derive(Debug, Clone, Default)]
pub struct PluginRegistry {
    pub plugins: Vec<PluginMetadata>,
}

impl PluginRegistry {
    pub fn from_list(metadata: Vec<PluginMetadata>) -> Self {
        PluginRegistry { plugins: metadata }
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Nama plugin aktif (enabled).
    pub fn active_names(&self) -> Vec<String> {
        self.plugins
            .iter()
            .filter(|m| m.enabled)
            .map(|m| m.name.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry() {
        let meta = vec![
            PluginMetadata { name: "a".into(), version: "1".into(), enabled: true },
            PluginMetadata { name: "b".into(), version: "2".into(), enabled: false },
        ];
        let reg = PluginRegistry::from_list(meta);
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.active_names(), vec!["a"]);
    }
}
