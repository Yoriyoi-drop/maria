use maria_compiler::hir::{HirSignal, LazyElaborator};

/// HirHandle — elaborasi lazy on-demand (HIR-based).
#[derive(Default)]
pub struct HirHandle {
    inner: LazyElaborator,
}

impl std::fmt::Debug for HirHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HirHandle").finish()
    }
}

impl HirHandle {
    pub fn new() -> Self {
        HirHandle {
            inner: LazyElaborator::new(),
        }
    }

    /// Daftarkan module dengan sinyal port-nya (param/statement di-resolve on-demand).
    pub fn register_module(&mut self, name: maria_core::intern::Symbol, signals: Vec<HirSignal>) {
        self.inner
            .elaborate_with_data(name, vec![], signals, vec![]);
    }

    pub fn inner(&self) -> &LazyElaborator {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut LazyElaborator {
        &mut self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maria_compiler::hir::HirType;
    use maria_core::intern::Symbol;

    #[test]
    fn test_hir_handle() {
        let mut h = HirHandle::new();
        let sig = HirSignal {
            name: Symbol::intern("clk"),
            dtype: HirType::BitVec { width: 1 },
            width: 1,
            is_input: true,
            is_output: false,
        };
        h.register_module(Symbol::intern("top"), vec![sig]);
    }
}
