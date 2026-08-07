use crate::config::MariaConfig;
use crate::env::config::defaults;
use crate::env::config::{environment, loader, validator};
use std::path::{Path, PathBuf};

/// Asal config — untuk diagnostics/telemetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    Default,
    File,
}

impl ConfigSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConfigSource::Default => "default",
            ConfigSource::File => "file",
        }
    }
}

/// ConfigContext — satu-satunya pintu compiler untuk membaca pengaturan.
///
/// Compiler tidak membaca file config; ia cukup bertanya:
/// `config.max_threads()`, `config.incremental()`, `config.sim_timeout()`.
/// Nilai dikumpulkan dari: default → file TOML/JSON → environment (`MARIA_*`)
/// → override CLI (via `EnvCliOptions::apply`).
pub struct ConfigContext {
    inner: MariaConfig,
    source: ConfigSource,
    source_path: Option<PathBuf>,
}

impl ConfigContext {
    /// Config kosong (semua fallback ke default kode).
    pub fn new(inner: MariaConfig) -> Self {
        ConfigContext {
            inner,
            source: ConfigSource::Default,
            source_path: None,
        }
    }

    /// Config dari file (dipakai loader).
    fn from_file(inner: MariaConfig, path: PathBuf) -> Self {
        ConfigContext {
            inner,
            source: ConfigSource::File,
            source_path: Some(path),
        }
    }

    /// Bangun ConfigContext dari `MariaConfig` yang sudah di-load caller
    /// (hindari baca ulang + validasi ganda). Info sumber disimpulkan dari
    /// `explicit` (path `--config`): bila Some → File(path); bila None dan
    /// `configs/compiler.toml` ada → File(default); selain itu Default.
    pub fn from_loaded(inner: MariaConfig, explicit: Option<&str>) -> Self {
        match explicit {
            Some(p) => ConfigContext::from_file(inner, PathBuf::from(p)),
            None => {
                let default = PathBuf::from("configs/compiler.toml");
                if default.exists() {
                    ConfigContext::from_file(inner, default)
                } else {
                    ConfigContext::new(inner)
                }
            }
        }
    }

    /// Muat config: `--config <path>` eksplisit, selain itu default
    /// `configs/compiler.toml` bila ada, lalu override `MARIA_*`.
    pub fn load_auto(explicit: Option<&str>) -> Result<Self, String> {
        let mut ctx = match explicit {
            Some(p) => {
                let path = PathBuf::from(p);
                let cfg = loader::load_from_path(&path)?;
                ConfigContext::from_file(cfg, path)
            }
            None => {
                let default = PathBuf::from("configs/compiler.toml");
                if default.exists() {
                    let cfg = loader::load_from_path(&default)?;
                    ConfigContext::from_file(cfg, default)
                } else {
                    ConfigContext::new(MariaConfig::default())
                }
            }
        };
        environment::apply_env(&mut ctx);
        validator::warn_invalid(&ctx.inner);
        Ok(ctx)
    }

    // ── Akses umum ──

    pub fn source(&self) -> ConfigSource {
        self.source
    }

    pub fn source_name(&self) -> &str {
        self.source.as_str()
    }

    pub fn source_path(&self) -> Option<&Path> {
        self.source_path.as_deref()
    }

    /// Akses config mentah — khusus subsystem yang perlu seluruh field.
    pub fn raw(&self) -> &MariaConfig {
        &self.inner
    }

    /// Mutasi terpandu (dipakai override environment/CLI).
    pub fn mutate<F: FnOnce(&mut MariaConfig)>(&mut self, f: F) {
        f(&mut self.inner);
    }

    // ── Accessor terpilih (compiler tidak perlu akses `raw()`) ──

    /// Jumlah thread parallel — 0/None di config berarti auto (ikut core).
    pub fn max_threads(&self) -> usize {
        self.inner
            .compiler
            .jobs
            .filter(|&j| j > 0)
            .unwrap_or_else(defaults::default_jobs)
    }

    pub fn jobs(&self) -> Option<usize> {
        self.inner.compiler.jobs
    }

    pub fn incremental(&self) -> bool {
        self.inner.compiler.incremental.unwrap_or_else(defaults::default_incremental)
    }

    pub fn cache_enabled(&self) -> bool {
        self.inner.compiler.cache.unwrap_or_else(defaults::default_cache)
    }

    pub fn opt_level(&self) -> u8 {
        self.inner.compiler.opt_level.unwrap_or(defaults::DEFAULT_OPT_LEVEL)
    }

    pub fn fail_fast(&self) -> bool {
        self.inner.compiler.fail_fast.unwrap_or(false)
    }

    pub fn edition(&self) -> &str {
        self.inner.compiler.edition.as_deref().unwrap_or(defaults::DEFAULT_EDITION)
    }

    pub fn target(&self) -> &str {
        self.inner.compiler.target.as_deref().unwrap_or(defaults::DEFAULT_TARGET)
    }

    /// Batas waktu simulasi (ns). None = unlimited (jalan sampai `$finish`).
    pub fn sim_timeout(&self) -> Option<u64> {
        self.inner.simulation.max_time
    }

    pub fn sim_force(&self) -> bool {
        self.inner.simulation.force_sim.unwrap_or(false)
    }

    pub fn elab_mode(&self) -> Option<&str> {
        self.inner.elaborate.mode.as_deref()
    }

    pub fn expand_all_generates(&self) -> bool {
        self.inner.elaborate.expand_all_generates.unwrap_or(false)
    }

    pub fn coverage_threshold(&self) -> Option<f64> {
        self.inner.coverage.branch_threshold
    }

    pub fn deep_debug(&self) -> bool {
        self.inner.debug.deep.unwrap_or(false)
    }

    pub fn snapshot_interval(&self) -> u64 {
        self.inner
            .debug
            .snapshot_interval
            .unwrap_or(defaults::DEFAULT_SNAP_INTERVAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let ctx = ConfigContext::new(MariaConfig::default());
        assert_eq!(ctx.source(), ConfigSource::Default);
        assert_eq!(ctx.source_name(), "default");
        assert!(ctx.max_threads() >= 1);
        assert!(ctx.incremental());
        assert!(ctx.cache_enabled());
        assert_eq!(ctx.edition(), "2012");
        assert_eq!(ctx.sim_timeout(), None);
        assert!(!ctx.sim_force());
        assert_eq!(ctx.snapshot_interval(), 1000);
    }

    #[test]
    fn test_from_file_source() {
        let ctx = ConfigContext::from_file(MariaConfig::default(), PathBuf::from("x.toml"));
        assert_eq!(ctx.source(), ConfigSource::File);
        assert_eq!(ctx.source_path(), Some(Path::new("x.toml")));
    }

    #[test]
    fn test_load_auto_from_default_file() {
        // `configs/compiler.toml` ada di root project → ter-load sebagai File.
        let ctx = ConfigContext::load_auto(None).expect("load_auto harus sukses");
        assert_eq!(ctx.source(), ConfigSource::File);
        assert!(ctx.source_path().is_some());
    }

    #[test]
    fn test_load_auto_explicit_path() {
        let dir = std::env::temp_dir().join("maria_env_cfg_test");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("c.toml");
        std::fs::write(&p, "[simulation]\nmax_time = 777\n").unwrap();
        let ctx = ConfigContext::load_auto(Some(p.to_str().unwrap())).unwrap();
        assert_eq!(ctx.sim_timeout(), Some(777));
        assert_eq!(ctx.source_name(), "file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_mutate() {
        let mut ctx = ConfigContext::new(MariaConfig::default());
        ctx.mutate(|c| c.compiler.jobs = Some(8));
        assert_eq!(ctx.max_threads(), 8);
    }
}
