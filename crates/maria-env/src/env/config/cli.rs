//! Opsi CLI yang relevan untuk ConfigContext.
//!
//! Binary crate (`src/cli.rs`) mendefinisikan `Cli`-nya sendiri. Di sini hanya
//! ada subset opsi yang memengaruhi config; binary bisa memetakan `Cli` → ini.
//! Aturan kekuatan: CLI menang atas file config dan environment.

use crate::env::config::ConfigContext;

#[derive(Debug, Clone, Default)]
pub struct EnvCliOptions {
    /// Jumlah thread parallel (>0 = kunci; None = ikut config/auto).
    pub jobs: Option<usize>,
    /// Batas waktu simulasi (ns).
    pub max_time: Option<u64>,
    pub force_sim: Option<bool>,
    /// Lewati MICD (setara `--recompile`): matikan incremental + cache.
    pub recompile: bool,
    /// Mode elaborasi (StrictSimulation / AnalysisRecovery).
    pub elab_mode: Option<String>,
    pub coverage_threshold: Option<f64>,
    pub deep_debug: Option<bool>,
    pub snap_interval: Option<u64>,
}

impl EnvCliOptions {
    /// Terapkan opsi CLI ke config (CLI menang).
    pub fn apply(&self, ctx: &mut ConfigContext) {
        if let Some(j) = self.jobs {
            if j > 0 {
                ctx.mutate(|c| c.compiler.jobs = Some(j));
            }
        }
        if let Some(t) = self.max_time {
            ctx.mutate(|c| c.simulation.max_time = Some(t));
        }
        if let Some(f) = self.force_sim {
            ctx.mutate(|c| c.simulation.force_sim = Some(f));
        }
        if self.recompile {
            ctx.mutate(|c| {
                c.compiler.incremental = Some(false);
                c.compiler.cache = Some(false);
            });
        }
        if let Some(m) = &self.elab_mode {
            ctx.mutate(|c| c.elaborate.mode = Some(m.clone()));
        }
        if let Some(t) = self.coverage_threshold {
            ctx.mutate(|c| c.coverage.branch_threshold = Some(t));
        }
        if let Some(d) = self.deep_debug {
            ctx.mutate(|c| c.debug.deep = Some(d));
        }
        if let Some(s) = self.snap_interval {
            ctx.mutate(|c| c.debug.snapshot_interval = Some(s));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maria_core::config::MariaConfig;

    #[test]
    fn test_cli_overrides_config() {
        let mut ctx = ConfigContext::new(MariaConfig::default());
        let opts = EnvCliOptions {
            jobs: Some(6),
            max_time: Some(500),
            recompile: true,
            ..Default::default()
        };
        opts.apply(&mut ctx);
        assert_eq!(ctx.jobs(), Some(6));
        assert_eq!(ctx.max_threads(), 6);
        assert_eq!(ctx.sim_timeout(), Some(500));
        assert!(!ctx.incremental());
        assert!(!ctx.cache_enabled());
    }

    #[test]
    fn test_cli_zero_jobs_ignored() {
        let mut ctx = ConfigContext::new(MariaConfig::default());
        EnvCliOptions { jobs: Some(0), ..Default::default() }.apply(&mut ctx);
        // jobs=0 berarti auto — tidak mengunci jumlah thread.
        assert_eq!(ctx.jobs(), None);
        assert!(ctx.max_threads() >= 1);
    }
}
