use crate::env::cache::CacheContext;
use crate::env::compiler::CompilerContext;
use crate::env::config::ConfigContext;
use crate::env::database::DatabaseContext;
use crate::env::diagnostics::DiagnosticsContext;
use crate::env::plugins::PluginContext;
use crate::env::runtime::RuntimeContext;
use crate::env::security::SecurityContext;
use crate::env::simulation::SimulationContext;
use crate::env::telemetry::TelemetryContext;
use crate::env::verification::VerificationContext;
use crate::env::workspace::WorkspaceContext;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Root object arsitektur Maria (doc/env.md, Desain 5).
///
/// GlobalEnv tidak memiliki logika compiler — ia hanya menyimpan service
/// (context) dan menyediakan akses seragam ke mereka. Tiap context adalah
/// `Arc` agar dapat dibagikan aman ke worker / GUI / tools.
pub struct GlobalEnv {
    pub config: Arc<ConfigContext>,
    pub workspace: Arc<WorkspaceContext>,
    pub runtime: Arc<RuntimeContext>,
    pub compiler: Arc<CompilerContext>,
    pub verification: Arc<VerificationContext>,
    pub simulation: Arc<SimulationContext>,
    pub cache: Arc<CacheContext>,
    pub database: Arc<DatabaseContext>,
    pub diagnostics: Arc<DiagnosticsContext>,
    pub telemetry: Arc<TelemetryContext>,
    pub plugins: Arc<PluginContext>,
    pub security: Arc<SecurityContext>,
    started_at: Instant,
}

impl GlobalEnv {
    /// Env minimal dengan semua context default (tanpa load file config).
    /// Dipakai fallback bila startup_with gagal, atau untuk tool/library.
    pub fn minimal() -> Self {
        use crate::env::cache::CacheContext;
        use crate::env::compiler::CompilerContext;
        use crate::env::config::ConfigContext;
        use crate::env::database::DatabaseContext;
        use crate::env::diagnostics::DiagnosticsContext;
        use crate::env::plugins::PluginContext;
        use crate::env::runtime::RuntimeContext;
        use crate::env::security::SecurityContext;
        use crate::env::simulation::SimulationContext;
        use crate::env::telemetry::TelemetryContext;
        use crate::env::verification::VerificationContext;
        use crate::env::workspace::WorkspaceContext;
        use maria_core::config::MariaConfig;

        let config = Arc::new(ConfigContext::new(MariaConfig::default()));
        let workspace = Arc::new(WorkspaceContext::open(&config));
        let runtime = Arc::new(RuntimeContext::new());
        let compiler = Arc::new(CompilerContext::new(&workspace, &config));
        let verification = Arc::new(VerificationContext::from_config(
            crate::env::verification::LintChecks::default(),
            crate::env::verification::CoverageSettings::from_config(config.raw()),
        ));
        let simulation = Arc::new(SimulationContext::new());
        let cache = Arc::new(CacheContext::new(
            workspace.root.join(".maria").join("cache"),
        ));
        let database = Arc::new(DatabaseContext::new());
        let diagnostics = Arc::new(DiagnosticsContext::new());
        let telemetry = Arc::new(TelemetryContext::new());
        let plugins = Arc::new(PluginContext::new());
        let security = Arc::new(SecurityContext::dev_mode());
        GlobalEnv::new(
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
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: Arc<ConfigContext>,
        workspace: Arc<WorkspaceContext>,
        runtime: Arc<RuntimeContext>,
        compiler: Arc<CompilerContext>,
        verification: Arc<VerificationContext>,
        simulation: Arc<SimulationContext>,
        cache: Arc<CacheContext>,
        database: Arc<DatabaseContext>,
        diagnostics: Arc<DiagnosticsContext>,
        telemetry: Arc<TelemetryContext>,
        plugins: Arc<PluginContext>,
        security: Arc<SecurityContext>,
    ) -> Self {
        GlobalEnv {
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
            started_at: Instant::now(),
        }
    }

    pub fn config(&self) -> &ConfigContext {
        &self.config
    }
    pub fn workspace(&self) -> &WorkspaceContext {
        &self.workspace
    }
    pub fn runtime(&self) -> &RuntimeContext {
        &self.runtime
    }
    pub fn compiler(&self) -> &CompilerContext {
        &self.compiler
    }
    pub fn verification(&self) -> &VerificationContext {
        &self.verification
    }
    pub fn simulation(&self) -> &SimulationContext {
        &self.simulation
    }
    pub fn cache(&self) -> &CacheContext {
        &self.cache
    }
    pub fn database(&self) -> &DatabaseContext {
        &self.database
    }
    pub fn diagnostics(&self) -> &DiagnosticsContext {
        &self.diagnostics
    }
    pub fn telemetry(&self) -> &TelemetryContext {
        &self.telemetry
    }
    pub fn plugins(&self) -> &PluginContext {
        &self.plugins
    }
    pub fn security(&self) -> &SecurityContext {
        &self.security
    }

    pub fn started_at(&self) -> Instant {
        self.started_at
    }

    pub fn uptime(&self) -> Duration {
        self.started_at.elapsed()
    }

    /// Ringkasan satu baris untuk telemetry / CLI startup.
    pub fn summary(&self) -> String {
        format!(
            "{} | config:{} sources={} threads={} | uptime={:?}",
            crate::env::global::version::version_string(),
            self.config.source_name(),
            self.compiler.source_count,
            self.runtime.parallelism(),
            self.uptime(),
        )
    }
}
