use maria_simulator::simulator::SimulationEngine;

/// EventQueueStats — statistik event queue setelah simulasi.
#[derive(Debug, Clone, Copy, Default)]
pub struct EventQueueStats {
    pub events_processed: u64,
}

impl EventQueueStats {
    /// Baca dari engine (`sim_perf.counters.events_processed`).
    pub fn from_engine(engine: &SimulationEngine) -> Self {
        EventQueueStats {
            events_processed: engine.sim_perf.counters.events_processed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_stats() {
        let s = EventQueueStats::default();
        assert_eq!(s.events_processed, 0);
    }
}
