//! AST → HIR Builder.
//!
//! Converts parsed AST into resolved HIR. Type resolution, parameter
//! substitution, and generate unrolling happen here.

use std::collections::HashMap;

use super::hir::*;
use crate::intern::Symbol;

/// Builder: AST → HIR.
pub struct HirBuilder {
    /// Lazy TypeSystem for type resolution
    type_system: std::sync::Arc<super::types::TypeSystem>,
    /// Parameter values for current module
    param_values: HashMap<Symbol, u64>,
}

impl HirBuilder {
    pub fn new() -> Self {
        HirBuilder {
            type_system: std::sync::Arc::new(super::types::TypeSystem::new()),
            param_values: HashMap::new(),
        }
    }

    /// Create builder with a shared type system.
    pub fn with_type_system(type_system: std::sync::Arc<super::types::TypeSystem>) -> Self {
        HirBuilder {
            type_system,
            param_values: HashMap::new(),
        }
    }

    /// Get reference to the type system.
    pub fn type_system(&self) -> &super::types::TypeSystem {
        &self.type_system
    }

    /// Build HIR for a module from AST data.
    ///
    /// This is a skeleton — real implementation converts from existing
    /// AST types (crate::ast::Module) to HIR types.
    pub fn build_module(
        &mut self,
        name: Symbol,
        params: Vec<HirParam>,
        signals: Vec<HirSignal>,
        stmts: Vec<HirStmt>,
    ) -> HirModule {
        let inputs: Vec<usize> = signals
            .iter()
            .enumerate()
            .filter(|(_, s)| s.is_input)
            .map(|(i, _)| i)
            .collect();
        let outputs: Vec<usize> = signals
            .iter()
            .enumerate()
            .filter(|(_, s)| s.is_output)
            .map(|(i, _)| i)
            .collect();

        HirModule {
            name,
            signals,
            inputs,
            outputs,
            params,
            stmts,
            sub_instances: Vec::new(),
            checksum: 0,
        }
    }

    /// Resolve a type name to HirType (lazy, via TypeSystem).
    pub fn resolve_type(&self, module: Symbol, name: Symbol) -> HirType {
        self.type_system.resolve_type(module, name)
    }

    /// Convert an AST DataType to HirType.
    pub fn ast_dtype_to_hir(&self, dtype: &crate::ast::types::DataType, scope: Symbol) -> HirType {
        self.type_system.ast_dtype_to_hir(dtype, scope)
    }

    /// Register typedefs with the type system.
    pub fn register_typedefs(&self, scope: Symbol, typedefs: &[crate::ast::types::TypedefDecl]) {
        self.type_system.register_typedefs(scope, typedefs);
    }

    /// Get reference to the underlying type system for sharing.
    pub fn shared_type_system(&self) -> std::sync::Arc<super::types::TypeSystem> {
        self.type_system.clone()
    }

    /// Set a parameter value.
    pub fn set_param(&mut self, name: Symbol, value: u64) {
        self.param_values.insert(name, value);
    }

    /// Get a parameter value.
    pub fn get_param(&self, name: Symbol) -> Option<u64> {
        self.param_values.get(&name).copied()
    }
}

impl Default for HirBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hir_builder_basic() {
        let mut builder = HirBuilder::new();
        let name = Symbol::intern("test_module");

        let module = builder.build_module(
            name,
            vec![],
            vec![HirSignal {
                name: Symbol::intern("clk"),
                dtype: HirType::BitVec { width: 1 },
                width: 1,
                is_input: true,
                is_output: false,
            }],
            vec![],
        );

        assert_eq!(module.name, name);
        assert_eq!(module.signals.len(), 1);
        assert_eq!(module.inputs.len(), 1);
    }

    #[test]
    fn test_hir_type_width() {
        assert_eq!(HirType::BitVec { width: 8 }.width(), 8);
        assert_eq!(HirType::Int { width: 32 }.width(), 32);
        assert_eq!(HirType::Real.width(), 64);
    }
}
