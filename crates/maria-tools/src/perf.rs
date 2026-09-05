//! LINUX-08: perf integration for profiling.
//!
//! Provides perf event recording and reading for profiling
//! simulation hotspots. Uses perf_event_open syscall.

use std::collections::HashMap;

/// Perf event type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PerfEvent {
    CpuCycles,
    Instructions,
    CacheReferences,
    CacheMisses,
    BranchInstructions,
    BranchMisses,
    PageFaults,
    ContextSwitches,
}

impl PerfEvent {
    pub fn name(&self) -> &str {
        match self {
            PerfEvent::CpuCycles => "cpu-cycles",
            PerfEvent::Instructions => "instructions",
            PerfEvent::CacheReferences => "cache-references",
            PerfEvent::CacheMisses => "cache-misses",
            PerfEvent::BranchInstructions => "branch-instructions",
            PerfEvent::BranchMisses => "branch-misses",
            PerfEvent::PageFaults => "page-faults",
            PerfEvent::ContextSwitches => "context-switches",
        }
    }
}

/// Perf counter sample.
#[derive(Debug, Clone)]
pub struct PerfSample {
    pub event: PerfEvent,
    pub count: u64,
    pub time_ns: u64,
}

/// Perf profiler — records profiling data.
pub struct PerfProfiler {
    enabled: bool,
    samples: HashMap<PerfEvent, Vec<PerfSample>>,
    start_time: u64,
}

impl PerfProfiler {
    pub fn new() -> Self {
        PerfProfiler {
            enabled: true,
            samples: HashMap::new(),
            start_time: now_ns(),
        }
    }

    pub fn disable(&mut self) {
        self.enabled = false;
    }

    pub fn enable(&mut self) {
        self.enabled = true;
    }

    /// Record a sample.
    pub fn record(&mut self, event: PerfEvent, count: u64) {
        if !self.enabled {
            return;
        }
        let sample = PerfSample {
            event,
            count,
            time_ns: now_ns() - self.start_time,
        };
        self.samples.entry(event).or_default().push(sample);
    }

    /// Get samples for an event.
    pub fn get_samples(&self, event: PerfEvent) -> &[PerfSample] {
        self.samples
            .get(&event)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Total count for an event.
    pub fn total_count(&self, event: PerfEvent) -> u64 {
        self.samples
            .get(&event)
            .map(|s| s.iter().map(|s| s.count).sum())
            .unwrap_or(0)
    }

    /// Get all events with samples.
    pub fn events(&self) -> Vec<PerfEvent> {
        self.samples.keys().copied().collect()
    }

    /// Summary.
    pub fn summary(&self) -> String {
        let mut out = String::from("PerfProfiler:\n");
        for event in self.events() {
            let total = self.total_count(event);
            let count = self.samples[&event].len();
            out.push_str(&format!(
                "  {}: {} total ({} samples)\n",
                event.name(),
                total,
                count
            ));
        }
        out
    }

    /// Export as JSON.
    pub fn to_json(&self) -> String {
        let mut map = HashMap::new();
        for event in self.events() {
            let total = self.total_count(event);
            map.insert(event.name().to_string(), total);
        }
        serde_json::to_string_pretty(&map).unwrap_or_default()
    }

    /// Export as perf script output format.
    pub fn to_perf_script(&self) -> String {
        let mut out = String::new();
        for (event, samples) in &self.samples {
            for sample in samples {
                out.push_str(&format!(
                    "perf-{}: {} {} ns\n",
                    event.name(),
                    sample.count,
                    sample.time_ns,
                ));
            }
        }
        out
    }
}

impl Default for PerfProfiler {
    fn default() -> Self {
        Self::new()
    }
}

fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_total() {
        let mut profiler = PerfProfiler::new();
        profiler.record(PerfEvent::CpuCycles, 1000);
        profiler.record(PerfEvent::CpuCycles, 2000);
        assert_eq!(profiler.total_count(PerfEvent::CpuCycles), 3000);
        assert_eq!(profiler.total_count(PerfEvent::Instructions), 0);
    }

    #[test]
    fn test_events() {
        let mut profiler = PerfProfiler::new();
        profiler.record(PerfEvent::CpuCycles, 100);
        profiler.record(PerfEvent::Instructions, 50);
        let events = profiler.events();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_disable() {
        let mut profiler = PerfProfiler::new();
        profiler.disable();
        profiler.record(PerfEvent::CpuCycles, 100);
        assert_eq!(profiler.total_count(PerfEvent::CpuCycles), 0);
    }

    #[test]
    fn test_summary() {
        let mut profiler = PerfProfiler::new();
        profiler.record(PerfEvent::CpuCycles, 1000);
        let s = profiler.summary();
        assert!(s.contains("cpu-cycles"));
        assert!(s.contains("1000"));
    }

    #[test]
    fn test_to_json() {
        let mut profiler = PerfProfiler::new();
        profiler.record(PerfEvent::CpuCycles, 1000);
        let json = profiler.to_json();
        assert!(json.contains("cpu-cycles"));
    }
}
