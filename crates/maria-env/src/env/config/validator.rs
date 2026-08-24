//! Validator config — memeriksa nilai config yang berbahaya/aneh sebelum
//! dipakai compiler. Ringan dan non-blokir (warning), kecuali `validate_or_err`.

use maria_core::config::MariaConfig;

/// Validasi nilai config. Mengembalikan daftar masalah (kosong = valid).
pub fn validate(cfg: &MariaConfig) -> Vec<String> {
    let mut problems = Vec::new();
    if let Some(j) = cfg.compiler.jobs {
        if j == 0 {
            problems.push(
                "compiler.jobs = 0 berarti auto — set >0 untuk mengunci jumlah thread".into(),
            );
        }
    }
    if let Some(l) = cfg.compiler.opt_level {
        if l > 3 {
            problems.push(format!("compiler.opt_level {} di luar rentang 0..=3", l));
        }
    }
    if let Some(m) = &cfg.elaborate.mode {
        if !m.eq_ignore_ascii_case("strictsimulation")
            && !m.eq_ignore_ascii_case("analysisrecovery")
        {
            problems.push(format!(
                "elaborate.mode '{}' tidak dikenal (pakai StrictSimulation / AnalysisRecovery)",
                m
            ));
        }
    }
    if let Some(d) = cfg.debug.snapshot_interval {
        if d == 0 {
            problems.push(
                "debug.snapshot_interval = 0 menonaktifkan snapshot (deep-debug butuh >0)".into(),
            );
        }
    }
    if let Some(th) = cfg.coverage.branch_threshold {
        if !(0.0..=100.0).contains(&th) {
            problems.push(format!(
                "coverage.branch_threshold {} di luar rentang 0..=100",
                th
            ));
        }
    }
    problems
}

/// Cetak masalah config sebagai warning ke stderr (non-blokir).
pub fn warn_invalid(cfg: &MariaConfig) {
    for p in validate(cfg) {
        eprintln!("warning: config: {}", p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_default() {
        assert!(validate(&MariaConfig::default()).is_empty());
    }

    #[test]
    fn test_catches_bad_opt_level() {
        let mut cfg = MariaConfig::default();
        cfg.compiler.opt_level = Some(9);
        assert!(validate(&cfg).iter().any(|p| p.contains("opt_level")));
    }

    #[test]
    fn test_catches_bad_elab_mode() {
        let mut cfg = MariaConfig::default();
        cfg.elaborate.mode = Some("mystery".into());
        assert!(validate(&cfg).iter().any(|p| p.contains("elaborate.mode")));
    }
}
