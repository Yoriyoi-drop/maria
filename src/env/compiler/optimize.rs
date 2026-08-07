use crate::env::config::ConfigContext;
use crate::ir::IrDesign;

/// OptimizeLevel — level optimisasi IR (0=none, 1=const-fold, 2=+DCE, 3=+peephole).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptimizeLevel(pub u8);

impl OptimizeLevel {
    pub fn from_config(config: &ConfigContext) -> Self {
        OptimizeLevel(config.opt_level().min(3))
    }

    pub fn level(&self) -> u8 {
        self.0
    }

    /// Terapkan optimisasi ke IR. Saat ini pass-through (hooks optimizer
    /// menyusul); IR yang sama dikembalikan tanpa mutasi.
    pub fn apply(self, ir: IrDesign) -> IrDesign {
        ir
    }
}

impl Default for OptimizeLevel {
    fn default() -> Self {
        OptimizeLevel(1)
    }
}

/// Passthrough singkat (sintaks `apply_optimizations(level, ir)`).
pub fn apply_optimizations(level: OptimizeLevel, ir: IrDesign) -> IrDesign {
    level.apply(ir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_optimize_level() {
        assert_eq!(OptimizeLevel(2).level(), 2);
        let mut cfg = crate::config::MariaConfig::default();
        cfg.compiler.opt_level = Some(5); // >3 → dipatok
        let ctx = ConfigContext::new(cfg);
        assert_eq!(OptimizeLevel::from_config(&ctx).level(), 3);
    }
}
