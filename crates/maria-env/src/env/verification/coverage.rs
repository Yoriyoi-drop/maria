use maria_core::config::MariaConfig;

/// CoverageSettings — pengaturan coverage (dari config `[coverage]`).
#[derive(Debug, Clone)]
pub struct CoverageSettings {
    pub enabled: bool,
    pub branch_threshold: f64,
    pub line_threshold: f64,
    pub json: bool,
    pub html: bool,
    pub ucis: bool,
    pub output_prefix: Option<String>,
}

impl CoverageSettings {
    pub fn from_config(cfg: &MariaConfig) -> Self {
        CoverageSettings {
            enabled: cfg.coverage.enable.unwrap_or(false),
            branch_threshold: cfg.coverage.branch_threshold.unwrap_or(0.0),
            line_threshold: cfg.coverage.line_threshold.unwrap_or(0.0),
            json: cfg.coverage.json.unwrap_or(false),
            html: cfg.coverage.html.unwrap_or(false),
            ucis: cfg.coverage.ucis.unwrap_or(false),
            output_prefix: cfg.coverage.output_prefix.clone(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Ringkasan coverage (persentase branch/line).
#[derive(Debug, Clone, Default)]
pub struct CoverageSummary {
    pub branch_percent: f64,
    pub line_percent: f64,
    pub total_cov_groups: usize,
}

/// Bangun ringkasan dari map stats engine (key `branch_percent`, `line_percent`).
pub fn coverage_summary(
    stats: &std::collections::HashMap<String, f64>,
    groups: usize,
) -> CoverageSummary {
    CoverageSummary {
        branch_percent: stats.get("branch_percent").copied().unwrap_or(0.0),
        line_percent: stats.get("line_percent").copied().unwrap_or(0.0),
        total_cov_groups: groups,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coverage_settings_default() {
        let s = CoverageSettings::from_config(&MariaConfig::default());
        assert!(!s.is_enabled());
    }

    #[test]
    fn test_coverage_summary() {
        let mut stats = std::collections::HashMap::new();
        stats.insert("branch_percent".into(), 87.5);
        let s = coverage_summary(&stats, 3);
        assert_eq!(s.branch_percent, 87.5);
        assert_eq!(s.line_percent, 0.0);
        assert_eq!(s.total_cov_groups, 3);
    }
}
