use crate::env::verification::{
    AssertionReport, CoverageSettings, LintChecks, SemanticStatus, XPropMode,
};

/// VerificationContext — semua checker (lint, semantic, xprop, assertions,
/// coverage). Context hanya menyimpan konfigurasi + status; eksekusi checker
/// menumpang engine/compiler lewat method di sini.
#[derive(Debug, Clone)]
pub struct VerificationContext {
    pub lint: LintChecks,
    pub coverage: CoverageSettings,
    pub xprop_mode: XPropMode,
    pub semantic: SemanticStatus,
}

impl VerificationContext {
    pub fn from_config(
        lint: LintChecks,
        coverage: CoverageSettings,
    ) -> Self {
        VerificationContext {
            lint,
            coverage,
            xprop_mode: XPropMode::Optimistic,
            semantic: SemanticStatus::default(),
        }
    }

    pub fn set_semantic_status(&mut self, status: SemanticStatus) {
        self.semantic = status;
    }

    pub fn apply_xprop(&self) {
        crate::env::verification::xprop::set_xprop(self.xprop_mode);
    }

    /// Jumlah check lint aktif (untuk telemetry/report).
    pub fn lint_check_count(&self) -> usize {
        self.lint.enabled_count()
    }

    pub fn ready_to_simulate(&self) -> bool {
        self.semantic.ready()
    }

    /// Gabungkan report assertion ke summary sederhana.
    pub fn assertion_status(&self, reports: &[AssertionReport]) -> String {
        let total: usize = reports.iter().map(|r| r.total).sum();
        let failed: usize = reports.iter().map(|r| r.failed + r.errored).sum();
        format!("{}/{} assertion passed", total - failed, total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verification_context() {
        let cfg = maria_core::config::MariaConfig::default();
        let ctx = VerificationContext::from_config(
            LintChecks::default(),
            CoverageSettings::from_config(&cfg),
        );
        assert!(ctx.lint_check_count() >= 5);
        ctx.apply_xprop(); // tidak panic
        assert_eq!(ctx.assertion_status(&[]), "0/0 assertion passed");
    }
}
