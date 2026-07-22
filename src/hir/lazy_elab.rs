//! Lazy Elaborator — on-demand module elaboration.
//!
//! Module hanya di-elaborate saat diminta. Hasil di-cache supaya
//! tidak perlu re-elaborate saat parameter atau dependency tidak berubah.

use std::sync::Arc;

use dashmap::DashMap;

use super::builder::HirBuilder;
use super::hir::HirModule;
use crate::intern::Symbol;

/// Lazy elaboration engine — on-demand module elaboration.
pub struct LazyElaborator {
    /// Cache of elaborated modules (name → HIR)
    elaborated: DashMap<Symbol, Arc<HirModule>>,
    /// Currently being elaborated (prevent double-elaboration)
    in_progress: DashSet<Symbol>,
    /// Builder for AST → HIR conversion
    builder: parking_lot::Mutex<HirBuilder>,
}

// Re-export DashSet since it's not in scope
use dashmap::DashSet;

impl LazyElaborator {
    pub fn new() -> Self {
        LazyElaborator {
            elaborated: DashMap::new(),
            in_progress: DashSet::new(),
            builder: parking_lot::Mutex::new(HirBuilder::new()),
        }
    }

    /// Elaborate a module on-demand. Returns cached version if available.
    pub fn elaborate(&self, name: Symbol) -> Option<Arc<HirModule>> {
        // 1. Check cache
        if let Some(module) = self.elaborated.get(&name) {
            return Some(module.clone());
        }

        // 2. Already being elaborated (by another thread)
        if self.in_progress.contains(&name) {
            // Wait and retry
            return self.elaborated.get(&name).map(|m| m.clone());
        }

        // Not found — caller must provide AST data and call elaborate_with_data
        None
    }

    /// Elaborate a module with provided data.
    pub fn elaborate_with_data(
        &self,
        name: Symbol,
        params: Vec<super::hir::HirParam>,
        signals: Vec<super::hir::HirSignal>,
        stmts: Vec<super::hir::HirStmt>,
    ) -> Arc<HirModule> {
        // Check cache first
        if let Some(module) = self.elaborated.get(&name) {
            return module.clone();
        }

        // Mark as in-progress
        self.in_progress.insert(name);

        // Build HIR
        let module = {
            let mut builder = self.builder.lock();
            builder.build_module(name, params, signals, stmts)
        };

        let module = Arc::new(module);

        // Cache and unmark
        self.elaborated.insert(name, module.clone());
        self.in_progress.remove(&name);

        module
    }

    /// Check if a module has been elaborated.
    pub fn is_elaborated(&self, name: Symbol) -> bool {
        self.elaborated.contains_key(&name)
    }

    /// Get all elaborated module names.
    pub fn module_names(&self) -> Vec<Symbol> {
        self.elaborated.iter().map(|entry| *entry.key()).collect()
    }

    /// Number of elaborated modules.
    pub fn len(&self) -> usize {
        self.elaborated.len()
    }

    /// Is empty.
    pub fn is_empty(&self) -> bool {
        self.elaborated.is_empty()
    }

    /// Invalidate all cached modules.
    pub fn invalidate_all(&self) {
        self.elaborated.clear();
    }

    /// Invalidate a specific module.
    pub fn invalidate(&self, name: Symbol) {
        self.elaborated.remove(&name);
    }
}

impl Default for LazyElaborator {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::hir::*;

    #[test]
    fn test_lazy_elab_basic() {
        let elab = LazyElaborator::new();
        let name = Symbol::intern("lazy_mod");

        assert!(!elab.is_elaborated(name));
        assert!(elab.elaborate(name).is_none());

        elab.elaborate_with_data(name, vec![], vec![], vec![]);
        assert!(elab.is_elaborated(name));
        assert!(elab.elaborate(name).is_some());
    }

    #[test]
    fn test_lazy_elab_invalidate() {
        let elab = LazyElaborator::new();
        let name = Symbol::intern("inv_mod");

        elab.elaborate_with_data(name, vec![], vec![], vec![]);
        assert!(elab.is_elaborated(name));

        elab.invalidate(name);
        assert!(!elab.is_elaborated(name));
    }
}
