use crate::config::MariaConfig;

/// LintChecks — check lint mana yang aktif (dari config `[lint]`).
#[derive(Debug, Clone)]
pub struct LintChecks {
    pub check_unused: bool,
    pub check_width: bool,
    pub check_latch: bool,
    pub check_combo_loop: bool,
    pub check_fsm: bool,
    pub check_case_priority: bool,
    pub check_partselect_range: bool,
    pub check_undefined_signal: bool,
}

impl LintChecks {
    pub fn from_config(cfg: &MariaConfig) -> Self {
        LintChecks {
            check_unused: cfg.lint.check_unused.unwrap_or(true),
            check_width: cfg.lint.check_width.unwrap_or(true),
            check_latch: cfg.lint.check_latch.unwrap_or(true),
            check_combo_loop: cfg.lint.check_combo_loop.unwrap_or(true),
            check_fsm: cfg.lint.check_fsm.unwrap_or(true),
            check_case_priority: cfg.lint.check_case_priority.unwrap_or(false),
            check_partselect_range: cfg.lint.check_partselect_range.unwrap_or(true),
            check_undefined_signal: cfg.lint.check_undefined_signal.unwrap_or(true),
        }
    }

    /// Jumlah check aktif.
    pub fn enabled_count(&self) -> usize {
        [
            self.check_unused,
            self.check_width,
            self.check_latch,
            self.check_combo_loop,
            self.check_fsm,
            self.check_case_priority,
            self.check_partselect_range,
            self.check_undefined_signal,
        ]
        .iter()
        .filter(|b| **b)
        .count()
    }
}

impl Default for LintChecks {
    fn default() -> Self {
        LintChecks::from_config(&MariaConfig::default())
    }
}

/// LintReport — ringkasan hasil lint (jumlah temuan per kategori).
#[derive(Debug, Default)]
pub struct LintReport {
    pub unused: usize,
    pub width: usize,
    pub latch: usize,
    pub combo_loop: usize,
    pub fsm: usize,
}

impl LintReport {
    pub fn total(&self) -> usize {
        self.unused + self.width + self.latch + self.combo_loop + self.fsm
    }

    pub fn is_clean(&self) -> bool {
        self.total() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lint_checks_default() {
        let c = LintChecks::default();
        assert!(c.check_width);
        assert!(c.enabled_count() >= 5);
    }

    #[test]
    fn test_lint_report() {
        let mut r = LintReport::default();
        r.unused = 3;
        r.width = 2;
        assert_eq!(r.total(), 5);
        assert!(!r.is_clean());
        assert!(LintReport::default().is_clean());
    }
}
