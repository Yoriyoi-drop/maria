use crate::env::cache::CacheContext;
use crate::env::compiler::CompilerContext;
use crate::env::config::ConfigContext;
use crate::env::database::DatabaseContext;
use crate::env::diagnostics::DiagnosticsContext;
use crate::env::global::GlobalEnv;
use crate::env::plugins::PluginContext;
use crate::env::runtime::RuntimeContext;
use crate::env::security::SecurityContext;
use crate::env::simulation::SimulationContext;
use crate::env::telemetry::TelemetryContext;
use crate::env::verification::VerificationContext;
use crate::env::workspace::WorkspaceContext;
use std::sync::Arc;

/// Lifecycle startup (doc/env.md):
///
/// ```text
/// Startup → Load Config → Open Workspace → Initialize Runtime
///        → Open Database (MICD) → Initialize Cache → Load Plugins
///        → Compile → Verify → Simulate → Flush Diagnostics
///        → Write Metrics → Shutdown
/// ```
///
/// Startup membangun context; Compile/Verify/Simulate dijalankan oleh caller
/// (lewat `env.compiler()`, `env.verification()`, `env.simulation()`).
pub fn startup() -> Result<GlobalEnv, String> {
    // 1. Load Config (TOML/JSON via loader + override MARIA_* env).
    let config = ConfigContext::load_auto(None)?;
    let workspace = WorkspaceContext::open(&config);
    for_cli(config, workspace)
}

/// Startup dari ConfigContext yang sudah disiapkan caller (mis. dari CLI).
pub fn startup_with(config: ConfigContext) -> Result<GlobalEnv, String> {
    let workspace = WorkspaceContext::open(&config);
    for_cli(config, workspace)
}

/// Startup dengan workspace yang sudah di-seed caller (mis. dari CLI:
/// sources/incdirs/defines eksplisit) — menghindari scan direktori penuh.
pub fn for_cli(config: ConfigContext, workspace: WorkspaceContext) -> Result<GlobalEnv, String> {
    build_env(config, workspace)
}

fn build_env(config: ConfigContext, workspace: WorkspaceContext) -> Result<GlobalEnv, String> {
    let config = Arc::new(config);

    // Workspace sudah dibuka caller (open / open_in / seed dari CLI).
    let workspace = Arc::new(workspace);

    // 3. Initialize Runtime (butuh config: jumlah thread).
    let runtime = Arc::new(RuntimeContext::init(&config)?);

    // 4. Open Database (MICD) — scoped per project.
    let db_root = crate::env::database::default_database_root();
    let sources = workspace.discover_sources();
    let pid = crate::env::database::project_id_for(
        &workspace.root,
        &sources,
        workspace.incdirs().dirs(),
        workspace.defines(),
    );
    let database = Arc::new(DatabaseContext::open(&db_root, &pid));

    // 5. Initialize Cache.
    let cache_root = workspace.root.join(".maria").join("cache");
    let cache = Arc::new(CacheContext::new(cache_root));

    // 6. Context tanpa dependensi: diagnostics, telemetry, security, plugins.
    let diagnostics = Arc::new(DiagnosticsContext::new());
    let telemetry = Arc::new(TelemetryContext::new());
    let mut security_ctx = SecurityContext::dev_mode();
    security_ctx.files.allow_root(workspace.root.clone());
    let security = Arc::new(security_ctx);
    let plugins = Arc::new(PluginContext::new());

    // 7. Compiler (butuh workspace + config).
    let compiler = Arc::new(CompilerContext::new(&workspace, &config));

    // 8. Verification (butuh config untuk lint/coverage settings).
    let verification = Arc::new(VerificationContext::from_config(
        crate::env::verification::LintChecks::from_config(config.raw()),
        crate::env::verification::CoverageSettings::from_config(config.raw()),
    ));

    // 9. Simulation (butuh config: max_time).
    let simulation = Arc::new(SimulationContext::new().with_max_time(config.sim_timeout()));

    telemetry.trace("startup", &format!("env siap: {} file source", compiler.source_count));

    Ok(GlobalEnv::new(
        config,
        workspace,
        runtime,
        compiler,
        verification,
        simulation,
        cache,
        database,
        diagnostics,
        telemetry,
        plugins,
        security,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_startup_builds_env() {
        let env = startup().expect("startup tidak boleh gagal di root project");
        assert_eq!(env.config().source_name(), "file");
        assert!(env.workspace().root().exists());
        assert!(env.runtime().parallelism() >= 1);
        assert!(env.database().is_open());
        assert!(env.telemetry().summary().contains("builds="));
        let s = env.summary();
        assert!(s.contains("maria"));
    }

    #[test]
    fn test_cache_root_is_under_maria() {
        let env = startup().unwrap();
        let root = &env.cache().root;
        assert!(root.to_string_lossy().contains(".maria"));
    }
}
