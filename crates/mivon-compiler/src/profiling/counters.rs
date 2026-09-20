//! Atomic performance counters for cross-thread aggregation.

use std::sync::atomic::{AtomicU64, Ordering};

/// Atomic counters for lock-free cross-thread stats.
pub struct AtomicCounters {
    pub files_processed: AtomicU64,
    pub tokens_lexed: AtomicU64,
    pub nodes_parsed: AtomicU64,
    pub modules_elaborated: AtomicU64,
    pub signals_resolved: AtomicU64,
    pub cache_hits: AtomicU64,
    pub cache_misses: AtomicU64,
    pub errors: AtomicU64,
    pub warnings: AtomicU64,
    pub bytes_allocated: AtomicU64,
    pub bytes_deallocated: AtomicU64,
}

impl AtomicCounters {
    pub fn new() -> Self {
        AtomicCounters {
            files_processed: AtomicU64::new(0),
            tokens_lexed: AtomicU64::new(0),
            nodes_parsed: AtomicU64::new(0),
            modules_elaborated: AtomicU64::new(0),
            signals_resolved: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            warnings: AtomicU64::new(0),
            bytes_allocated: AtomicU64::new(0),
            bytes_deallocated: AtomicU64::new(0),
        }
    }

    pub fn increment(&self, counter: CounterType) {
        match counter {
            CounterType::FilesProcessed => {
                self.files_processed.fetch_add(1, Ordering::Relaxed);
            }
            CounterType::TokensLexed => {
                self.tokens_lexed.fetch_add(1, Ordering::Relaxed);
            }
            CounterType::NodesParsed => {
                self.nodes_parsed.fetch_add(1, Ordering::Relaxed);
            }
            CounterType::ModulesElaborated => {
                self.modules_elaborated.fetch_add(1, Ordering::Relaxed);
            }
            CounterType::SignalsResolved => {
                self.signals_resolved.fetch_add(1, Ordering::Relaxed);
            }
            CounterType::CacheHits => {
                self.cache_hits.fetch_add(1, Ordering::Relaxed);
            }
            CounterType::CacheMisses => {
                self.cache_misses.fetch_add(1, Ordering::Relaxed);
            }
            CounterType::Errors => {
                self.errors.fetch_add(1, Ordering::Relaxed);
            }
            CounterType::Warnings => {
                self.warnings.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn add(&self, counter: CounterType, amount: u64) {
        match counter {
            CounterType::FilesProcessed => {
                self.files_processed.fetch_add(amount, Ordering::Relaxed);
            }
            CounterType::TokensLexed => {
                self.tokens_lexed.fetch_add(amount, Ordering::Relaxed);
            }
            CounterType::NodesParsed => {
                self.nodes_parsed.fetch_add(amount, Ordering::Relaxed);
            }
            CounterType::ModulesElaborated => {
                self.modules_elaborated.fetch_add(amount, Ordering::Relaxed);
            }
            CounterType::SignalsResolved => {
                self.signals_resolved.fetch_add(amount, Ordering::Relaxed);
            }
            CounterType::CacheHits => {
                self.cache_hits.fetch_add(amount, Ordering::Relaxed);
            }
            CounterType::CacheMisses => {
                self.cache_misses.fetch_add(amount, Ordering::Relaxed);
            }
            CounterType::Errors => {
                self.errors.fetch_add(amount, Ordering::Relaxed);
            }
            CounterType::Warnings => {
                self.warnings.fetch_add(amount, Ordering::Relaxed);
            }
        }
    }

    pub fn get(&self, counter: CounterType) -> u64 {
        match counter {
            CounterType::FilesProcessed => self.files_processed.load(Ordering::Relaxed),
            CounterType::TokensLexed => self.tokens_lexed.load(Ordering::Relaxed),
            CounterType::NodesParsed => self.nodes_parsed.load(Ordering::Relaxed),
            CounterType::ModulesElaborated => self.modules_elaborated.load(Ordering::Relaxed),
            CounterType::SignalsResolved => self.signals_resolved.load(Ordering::Relaxed),
            CounterType::CacheHits => self.cache_hits.load(Ordering::Relaxed),
            CounterType::CacheMisses => self.cache_misses.load(Ordering::Relaxed),
            CounterType::Errors => self.errors.load(Ordering::Relaxed),
            CounterType::Warnings => self.warnings.load(Ordering::Relaxed),
        }
    }

    pub fn cache_hit_rate(&self) -> f64 {
        let hits = self.cache_hits.load(Ordering::Relaxed) as f64;
        let misses = self.cache_misses.load(Ordering::Relaxed) as f64;
        let total = hits + misses;
        if total == 0.0 {
            0.0
        } else {
            hits / total
        }
    }

    pub fn reset(&self) {
        self.files_processed.store(0, Ordering::Relaxed);
        self.tokens_lexed.store(0, Ordering::Relaxed);
        self.nodes_parsed.store(0, Ordering::Relaxed);
        self.modules_elaborated.store(0, Ordering::Relaxed);
        self.signals_resolved.store(0, Ordering::Relaxed);
        self.cache_hits.store(0, Ordering::Relaxed);
        self.cache_misses.store(0, Ordering::Relaxed);
        self.errors.store(0, Ordering::Relaxed);
        self.warnings.store(0, Ordering::Relaxed);
        self.bytes_allocated.store(0, Ordering::Relaxed);
        self.bytes_deallocated.store(0, Ordering::Relaxed);
    }
}

impl Default for AtomicCounters {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum CounterType {
    FilesProcessed,
    TokensLexed,
    NodesParsed,
    ModulesElaborated,
    SignalsResolved,
    CacheHits,
    CacheMisses,
    Errors,
    Warnings,
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atomic_counters() {
        let c = AtomicCounters::new();
        c.increment(CounterType::FilesProcessed);
        c.add(CounterType::TokensLexed, 100);
        assert_eq!(c.get(CounterType::FilesProcessed), 1);
        assert_eq!(c.get(CounterType::TokensLexed), 100);
    }

    #[test]
    fn test_cache_hit_rate() {
        let c = AtomicCounters::new();
        c.add(CounterType::CacheHits, 75);
        c.add(CounterType::CacheMisses, 25);
        assert!((c.cache_hit_rate() - 0.75).abs() < f64::EPSILON);
    }
}
