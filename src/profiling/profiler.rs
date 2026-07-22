//! Profiler — built-in performance profiling.
//!
//! Thread-safe, lock-free counters untuk tracking waktu dan statistik
//! setiap fase pipeline.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

// ─── Thread Counters ───

/// Per-thread profiling counters.
#[derive(Debug, Default)]
pub struct ThreadCounters {
    pub lex_time_ns: u64,
    pub parse_time_ns: u64,
    pub typecheck_time_ns: u64,
    pub elab_time_ns: u64,
    pub lower_time_ns: u64,
    pub sim_time_ns: u64,
    pub tokens_lexed: u64,
    pub nodes_parsed: u64,
    pub signals_elaborated: u64,
    pub ast_cache_hits: u64,
    pub ast_cache_misses: u64,
    pub hir_cache_hits: u64,
    pub hir_cache_misses: u64,
    pub arena_allocated_bytes: u64,
    pub arena_wasted_bytes: u64,
}

// ─── Global Counters ───

/// Atomic global counters for cross-thread aggregation.
pub struct GlobalCounters {
    pub total_time_ns: AtomicU64,
    pub peak_memory_bytes: AtomicU64,
    pub files_processed: AtomicU64,
    pub modules_elaborated: AtomicU64,
    pub errors_count: AtomicU64,
    pub warnings_count: AtomicU64,
}

impl Default for GlobalCounters {
    fn default() -> Self {
        GlobalCounters {
            total_time_ns: AtomicU64::new(0),
            peak_memory_bytes: AtomicU64::new(0),
            files_processed: AtomicU64::new(0),
            modules_elaborated: AtomicU64::new(0),
            errors_count: AtomicU64::new(0),
            warnings_count: AtomicU64::new(0),
        }
    }
}

// ─── Profiler ───

/// Built-in profiler with thread-local counters.
/// Uses atomic counters for cross-thread aggregation (no RefCell).
pub struct Profiler {
    /// Atomic phase counters (ns) — one per phase
    phase_lex_ns: AtomicU64,
    phase_parse_ns: AtomicU64,
    phase_typecheck_ns: AtomicU64,
    phase_elab_ns: AtomicU64,
    phase_lower_ns: AtomicU64,
    phase_sim_ns: AtomicU64,
    /// Atomic stat counters
    tokens_lexed: AtomicU64,
    nodes_parsed: AtomicU64,
    signals_elaborated: AtomicU64,
    ast_cache_hits: AtomicU64,
    ast_cache_misses: AtomicU64,
    hir_cache_hits: AtomicU64,
    hir_cache_misses: AtomicU64,
    arena_allocated: AtomicU64,
    arena_wasted: AtomicU64,
    pub global: GlobalCounters,
    start: Instant,
}

impl Profiler {
    pub fn new() -> Self {
        Profiler {
            phase_lex_ns: AtomicU64::new(0),
            phase_parse_ns: AtomicU64::new(0),
            phase_typecheck_ns: AtomicU64::new(0),
            phase_elab_ns: AtomicU64::new(0),
            phase_lower_ns: AtomicU64::new(0),
            phase_sim_ns: AtomicU64::new(0),
            tokens_lexed: AtomicU64::new(0),
            nodes_parsed: AtomicU64::new(0),
            signals_elaborated: AtomicU64::new(0),
            ast_cache_hits: AtomicU64::new(0),
            ast_cache_misses: AtomicU64::new(0),
            hir_cache_hits: AtomicU64::new(0),
            hir_cache_misses: AtomicU64::new(0),
            arena_allocated: AtomicU64::new(0),
            arena_wasted: AtomicU64::new(0),
            global: GlobalCounters::default(),
            start: Instant::now(),
        }
    }

    /// Record time spent in a phase.
    pub fn record_phase(&self, phase: Phase, duration_ns: u64) {
        match phase {
            Phase::Lex => {
                self.phase_lex_ns.fetch_add(duration_ns, Ordering::Relaxed);
            }
            Phase::Parse => {
                self.phase_parse_ns
                    .fetch_add(duration_ns, Ordering::Relaxed);
            }
            Phase::TypeCheck => {
                self.phase_typecheck_ns
                    .fetch_add(duration_ns, Ordering::Relaxed);
            }
            Phase::Elaborate => {
                self.phase_elab_ns.fetch_add(duration_ns, Ordering::Relaxed);
            }
            Phase::Lower => {
                self.phase_lower_ns
                    .fetch_add(duration_ns, Ordering::Relaxed);
            }
            Phase::Simulate => {
                self.phase_sim_ns.fetch_add(duration_ns, Ordering::Relaxed);
            }
        }
    }

    /// Increment a counter.
    pub fn count(&self, counter: Counter, amount: u64) {
        match counter {
            Counter::TokensLexed => {
                self.tokens_lexed.fetch_add(amount, Ordering::Relaxed);
            }
            Counter::NodesParsed => {
                self.nodes_parsed.fetch_add(amount, Ordering::Relaxed);
            }
            Counter::SignalsElaborated => {
                self.signals_elaborated.fetch_add(amount, Ordering::Relaxed);
            }
            Counter::AstCacheHits => {
                self.ast_cache_hits.fetch_add(amount, Ordering::Relaxed);
            }
            Counter::AstCacheMisses => {
                self.ast_cache_misses.fetch_add(amount, Ordering::Relaxed);
            }
            Counter::HirCacheHits => {
                self.hir_cache_hits.fetch_add(amount, Ordering::Relaxed);
            }
            Counter::HirCacheMisses => {
                self.hir_cache_misses.fetch_add(amount, Ordering::Relaxed);
            }
            Counter::ArenaAllocated => {
                self.arena_allocated.fetch_add(amount, Ordering::Relaxed);
            }
            Counter::ArenaWasted => {
                self.arena_wasted.fetch_add(amount, Ordering::Relaxed);
            }
        }
    }

    /// Get elapsed time since profiler creation.
    pub fn elapsed(&self) -> std::time::Duration {
        self.start.elapsed()
    }

    /// Aggregate all counters into a single summary.
    pub fn aggregate(&self) -> ThreadCounters {
        ThreadCounters {
            lex_time_ns: self.phase_lex_ns.load(Ordering::Relaxed),
            parse_time_ns: self.phase_parse_ns.load(Ordering::Relaxed),
            typecheck_time_ns: self.phase_typecheck_ns.load(Ordering::Relaxed),
            elab_time_ns: self.phase_elab_ns.load(Ordering::Relaxed),
            lower_time_ns: self.phase_lower_ns.load(Ordering::Relaxed),
            sim_time_ns: self.phase_sim_ns.load(Ordering::Relaxed),
            tokens_lexed: self.tokens_lexed.load(Ordering::Relaxed),
            nodes_parsed: self.nodes_parsed.load(Ordering::Relaxed),
            signals_elaborated: self.signals_elaborated.load(Ordering::Relaxed),
            ast_cache_hits: self.ast_cache_hits.load(Ordering::Relaxed),
            ast_cache_misses: self.ast_cache_misses.load(Ordering::Relaxed),
            hir_cache_hits: self.hir_cache_hits.load(Ordering::Relaxed),
            hir_cache_misses: self.hir_cache_misses.load(Ordering::Relaxed),
            arena_allocated_bytes: self.arena_allocated.load(Ordering::Relaxed),
            arena_wasted_bytes: self.arena_wasted.load(Ordering::Relaxed),
        }
    }

    /// Generate a profile report.
    pub fn report(&self) -> ProfileReport {
        let agg = self.aggregate();
        let _total_ns = self.global.total_time_ns.load(Ordering::Relaxed);
        let elapsed = self.elapsed();

        ProfileReport {
            total_elapsed: elapsed,
            peak_memory_mb: self.global.peak_memory_bytes.load(Ordering::Relaxed) as f64
                / (1024.0 * 1024.0),
            lex_ms: agg.lex_time_ns as f64 / 1_000_000.0,
            parse_ms: agg.parse_time_ns as f64 / 1_000_000.0,
            typecheck_ms: agg.typecheck_time_ns as f64 / 1_000_000.0,
            elab_ms: agg.elab_time_ns as f64 / 1_000_000.0,
            lower_ms: agg.lower_time_ns as f64 / 1_000_000.0,
            sim_ms: agg.sim_time_ns as f64 / 1_000_000.0,
            tokens_lexed: agg.tokens_lexed,
            nodes_parsed: agg.nodes_parsed,
            files_processed: self.global.files_processed.load(Ordering::Relaxed),
            modules_elaborated: self.global.modules_elaborated.load(Ordering::Relaxed),
            ast_hit_rate: if agg.ast_cache_hits + agg.ast_cache_misses > 0 {
                agg.ast_cache_hits as f64 / (agg.ast_cache_hits + agg.ast_cache_misses) as f64
            } else {
                0.0
            },
            hir_hit_rate: if agg.hir_cache_hits + agg.hir_cache_misses > 0 {
                agg.hir_cache_hits as f64 / (agg.hir_cache_hits + agg.hir_cache_misses) as f64
            } else {
                0.0
            },
            arena_allocated_mb: agg.arena_allocated_bytes as f64 / (1024.0 * 1024.0),
            errors: self.global.errors_count.load(Ordering::Relaxed),
            warnings: self.global.warnings_count.load(Ordering::Relaxed),
        }
    }
}

impl Default for Profiler {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Enums ───

#[derive(Debug, Clone, Copy)]
pub enum Phase {
    Lex,
    Parse,
    TypeCheck,
    Elaborate,
    Lower,
    Simulate,
}

#[derive(Debug, Clone, Copy)]
pub enum Counter {
    TokensLexed,
    NodesParsed,
    SignalsElaborated,
    AstCacheHits,
    AstCacheMisses,
    HirCacheHits,
    HirCacheMisses,
    ArenaAllocated,
    ArenaWasted,
}

// ─── Profile Report ───

/// Complete profiling report.
#[derive(Debug)]
pub struct ProfileReport {
    pub total_elapsed: std::time::Duration,
    pub peak_memory_mb: f64,
    pub lex_ms: f64,
    pub parse_ms: f64,
    pub typecheck_ms: f64,
    pub elab_ms: f64,
    pub lower_ms: f64,
    pub sim_ms: f64,
    pub tokens_lexed: u64,
    pub nodes_parsed: u64,
    pub files_processed: u64,
    pub modules_elaborated: u64,
    pub ast_hit_rate: f64,
    pub hir_hit_rate: f64,
    pub arena_allocated_mb: f64,
    pub errors: u64,
    pub warnings: u64,
}

impl std::fmt::Display for ProfileReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "═══ Maria Profile Report ═══")?;
        writeln!(f, "Total elapsed:  {:.2?}", self.total_elapsed)?;
        writeln!(f, "Peak memory:    {:.1} MB", self.peak_memory_mb)?;
        writeln!(f)?;
        writeln!(f, "Phase breakdown:")?;
        writeln!(f, "  Lex:       {:>8.2} ms", self.lex_ms)?;
        writeln!(f, "  Parse:     {:>8.2} ms", self.parse_ms)?;
        writeln!(f, "  TypeCheck: {:>8.2} ms", self.typecheck_ms)?;
        writeln!(f, "  Elaborate: {:>8.2} ms", self.elab_ms)?;
        writeln!(f, "  Lower:     {:>8.2} ms", self.lower_ms)?;
        writeln!(f, "  Simulate:  {:>8.2} ms", self.sim_ms)?;
        writeln!(f)?;
        writeln!(f, "Statistics:")?;
        writeln!(f, "  Tokens lexed:    {:>10}", self.tokens_lexed)?;
        writeln!(f, "  Nodes parsed:    {:>10}", self.nodes_parsed)?;
        writeln!(f, "  Files processed: {:>10}", self.files_processed)?;
        writeln!(f, "  Modules elab:    {:>10}", self.modules_elaborated)?;
        writeln!(f, "  AST hit rate:    {:>9.1}%", self.ast_hit_rate * 100.0)?;
        writeln!(f, "  HIR hit rate:    {:>9.1}%", self.hir_hit_rate * 100.0)?;
        writeln!(f, "  Arena alloc:     {:>8.1} MB", self.arena_allocated_mb)?;
        writeln!(f)?;
        writeln!(f, "Diagnostics:")?;
        writeln!(f, "  Errors:   {}", self.errors)?;
        writeln!(f, "  Warnings: {}", self.warnings)?;
        Ok(())
    }
}

// ─── Phase Timer ───

/// RAII timer untuk mengukur waktu sebuah phase.
pub struct PhaseTimer<'a> {
    profiler: &'a Profiler,
    phase: Phase,
    start: Instant,
}

impl<'a> PhaseTimer<'a> {
    pub fn new(profiler: &'a Profiler, phase: Phase) -> Self {
        PhaseTimer {
            profiler,
            phase,
            start: Instant::now(),
        }
    }
}

impl<'a> Drop for PhaseTimer<'a> {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed().as_nanos() as u64;
        self.profiler.record_phase(self.phase, elapsed);
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profiler_new() {
        let profiler = Profiler::new();
        assert!(profiler.elapsed().as_nanos() > 0);
    }

    #[test]
    fn test_profiler_record_phase() {
        let profiler = Profiler::new();
        profiler.record_phase(Phase::Lex, 1000);
        profiler.record_phase(Phase::Parse, 2000);

        let agg = profiler.aggregate();
        assert_eq!(agg.lex_time_ns, 1000);
        assert_eq!(agg.parse_time_ns, 2000);
    }

    #[test]
    fn test_profiler_counter() {
        let profiler = Profiler::new();
        profiler.count(Counter::TokensLexed, 100);
        profiler.count(Counter::TokensLexed, 50);

        let agg = profiler.aggregate();
        assert_eq!(agg.tokens_lexed, 150);
    }

    #[test]
    fn test_profiler_report() {
        let profiler = Profiler::new();
        profiler.record_phase(Phase::Lex, 5_000_000);
        profiler.record_phase(Phase::Parse, 10_000_000);
        profiler.count(Counter::TokensLexed, 1000);

        let report = profiler.report();
        assert!(report.lex_ms > 0.0);
        assert!(report.parse_ms > 0.0);
        assert_eq!(report.tokens_lexed, 1000);
    }

    #[test]
    fn test_phase_timer() {
        let profiler = Profiler::new();
        {
            let _timer = PhaseTimer::new(&profiler, Phase::Lex);
            // Simulate some work
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let agg = profiler.aggregate();
        assert!(agg.lex_time_ns > 0);
    }
}
