use maria_compiler::profiling::{Counter, Phase, ProfileReport, Profiler};

/// Handle ke profiler pipeline (per-fase timing + counter).
#[derive(Default)]
pub struct ProfilerHandle {
    inner: Option<Profiler>,
}

impl std::fmt::Debug for ProfilerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfilerHandle")
            .field("active", &self.is_active())
            .finish()
    }
}

impl ProfilerHandle {
    pub fn new() -> Self {
        ProfilerHandle {
            inner: Some(Profiler::new()),
        }
    }

    /// Profiler nonaktif (operasi no-op).
    pub fn disabled() -> Self {
        ProfilerHandle { inner: None }
    }

    pub fn is_active(&self) -> bool {
        self.inner.is_some()
    }

    pub fn record_phase(&self, phase: Phase, duration_ns: u64) {
        if let Some(p) = &self.inner {
            p.record_phase(phase, duration_ns);
        }
    }

    pub fn count(&self, counter: Counter, amount: u64) {
        if let Some(p) = &self.inner {
            p.count(counter, amount);
        }
    }

    pub fn report(&self) -> Option<ProfileReport> {
        self.inner.as_ref().map(|p| p.report())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profiler_handle() {
        let p = ProfilerHandle::new();
        assert!(p.is_active());
        p.record_phase(Phase::Lex, 1000);
        p.count(Counter::TokensLexed, 42);
        let r = p.report().unwrap();
        assert!(r.lex_ms > 0.0);
        assert!(r.tokens_lexed >= 42);
    }

    #[test]
    fn test_disabled_noop() {
        let p = ProfilerHandle::disabled();
        assert!(!p.is_active());
        assert!(p.report().is_none());
        p.record_phase(Phase::Parse, 5); // tidak panic
    }
}
