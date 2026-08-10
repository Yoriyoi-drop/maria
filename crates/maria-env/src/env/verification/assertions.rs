/// AssertionReport — hasil assertion satu run (pass/fail/error).
#[derive(Debug, Clone, Copy, Default)]
pub struct AssertionReport {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub errored: usize,
}

impl AssertionReport {
    pub fn is_clean(&self) -> bool {
        self.failed == 0 && self.errored == 0
    }
}

/// AssertionSummary — agregat assertion lintas covergroup (untuk laporan akhir).
#[derive(Debug, Clone, Default)]
pub struct AssertionSummary {
    pub by_assertion: Vec<AssertionReport>,
}

impl AssertionSummary {
    pub fn total(&self) -> usize {
        self.by_assertion.iter().map(|r| r.total).sum()
    }

    pub fn passed(&self) -> usize {
        self.by_assertion.iter().map(|r| r.passed).sum()
    }

    pub fn failed(&self) -> usize {
        self.by_assertion.iter().map(|r| r.failed).sum()
    }

    pub fn summary_line(&self) -> String {
        format!("{}/{} assertion passed", self.passed(), self.total())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assertion_report() {
        let r = AssertionReport { total: 3, passed: 2, failed: 1, errored: 0 };
        assert!(!r.is_clean());
        assert!(AssertionReport::default().is_clean());
    }

    #[test]
    fn test_assertion_summary() {
        let s = AssertionSummary {
            by_assertion: vec![
                AssertionReport { total: 2, passed: 2, failed: 0, errored: 0 },
                AssertionReport { total: 1, passed: 1, failed: 0, errored: 0 },
            ],
        };
        assert_eq!(s.total(), 3);
        assert_eq!(s.passed(), 3);
        assert_eq!(s.failed(), 0);
        assert!(s.summary_line().starts_with("3/3"));
    }
}
