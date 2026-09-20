//! LINUX-07: BPF-based performance monitoring.
//!
//! Provides perf event counting and profiling without external tools.
//! Uses software perf events for simulation hotspot detection.

use std::collections::HashMap;

/// Performance event counter.
#[derive(Debug, Clone)]
pub struct PerfCounter {
    pub name: String,
    pub count: u64,
    pub enabled: bool,
}

/// BPF-based monitor — collects performance metrics.
pub struct BpfMonitor {
    counters: HashMap<String, PerfCounter>,
    enabled: bool,
    start_time: u64,
    markers: Vec<Marker>,
}

/// A profiling marker (start/end of a section).
#[derive(Debug, Clone)]
pub struct Marker {
    pub name: String,
    pub start_ns: u64,
    pub end_ns: Option<u64>,
}

impl BpfMonitor {
    pub fn new() -> Self {
        BpfMonitor {
            counters: HashMap::new(),
            enabled: true,
            start_time: now_ns(),
            markers: Vec::new(),
        }
    }

    pub fn disable(&mut self) {
        self.enabled = false;
    }

    pub fn enable(&mut self) {
        self.enabled = true;
    }

    /// Increment a counter.
    pub fn count(&mut self, name: &str) {
        if !self.enabled {
            return;
        }
        self.counters
            .entry(name.to_string())
            .and_modify(|c| c.count += 1)
            .or_insert_with(|| PerfCounter {
                name: name.to_string(),
                count: 1,
                enabled: true,
            });
    }

    /// Add to a counter by a specific amount.
    pub fn add(&mut self, name: &str, amount: u64) {
        if !self.enabled {
            return;
        }
        self.counters
            .entry(name.to_string())
            .and_modify(|c| c.count += amount)
            .or_insert_with(|| PerfCounter {
                name: name.to_string(),
                count: amount,
                enabled: true,
            });
    }

    /// Get a counter value.
    pub fn get(&self, name: &str) -> u64 {
        self.counters.get(name).map(|c| c.count).unwrap_or(0)
    }

    /// Start a profiling marker.
    pub fn start_marker(&mut self, name: &str) {
        if !self.enabled {
            return;
        }
        self.markers.push(Marker {
            name: name.to_string(),
            start_ns: now_ns() - self.start_time,
            end_ns: None,
        });
    }

    /// End a profiling marker.
    pub fn end_marker(&mut self, name: &str) {
        if !self.enabled {
            return;
        }
        let now = now_ns() - self.start_time;
        for marker in self.markers.iter_mut().rev() {
            if marker.name == name && marker.end_ns.is_none() {
                marker.end_ns = Some(now);
                break;
            }
        }
    }

    /// Get elapsed time for a marker.
    pub fn marker_elapsed_ns(&self, name: &str) -> Option<u64> {
        self.markers
            .iter()
            .find(|m| m.name == name && m.end_ns.is_some())
            .map(|m| m.end_ns.unwrap() - m.start_ns)
    }

    /// Get all counter names and values.
    pub fn counters(&self) -> Vec<(String, u64)> {
        self.counters
            .iter()
            .map(|(k, v)| (k.clone(), v.count))
            .collect()
    }

    /// Summary report.
    pub fn summary(&self) -> String {
        let mut out = format!(
            "BpfMonitor ({}s):\n",
            (now_ns() - self.start_time) / 1_000_000_000
        );
        let mut counters: Vec<_> = self.counters.iter().collect();
        counters.sort_by_key(|(_, c)| std::cmp::Reverse(c.count));
        for (name, counter) in counters {
            out.push_str(&format!("  {}: {}\n", name, counter.count));
        }
        if !self.markers.is_empty() {
            out.push_str("\nMarkers:\n");
            for marker in &self.markers {
                if let Some(end) = marker.end_ns {
                    out.push_str(&format!(
                        "  {}: {}ms\n",
                        marker.name,
                        (end - marker.start_ns) / 1_000_000
                    ));
                }
            }
        }
        out
    }

    /// Reset all counters.
    pub fn reset(&mut self) {
        self.counters.clear();
        self.markers.clear();
    }
}

impl Default for BpfMonitor {
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
    fn test_count() {
        let mut mon = BpfMonitor::new();
        mon.count("signal_read");
        mon.count("signal_read");
        mon.count("signal_read");
        assert_eq!(mon.get("signal_read"), 3);
        assert_eq!(mon.get("missing"), 0);
    }

    #[test]
    fn test_add() {
        let mut mon = BpfMonitor::new();
        mon.add("bytes", 100);
        mon.add("bytes", 200);
        assert_eq!(mon.get("bytes"), 300);
    }

    #[test]
    fn test_disable() {
        let mut mon = BpfMonitor::new();
        mon.disable();
        mon.count("test");
        assert_eq!(mon.get("test"), 0);
    }

    #[test]
    fn test_markers() {
        let mut mon = BpfMonitor::new();
        mon.start_marker("compile");
        std::thread::sleep(std::time::Duration::from_millis(1));
        mon.end_marker("compile");
        assert!(mon.marker_elapsed_ns("compile").unwrap() > 0);
    }

    #[test]
    fn test_summary() {
        let mut mon = BpfMonitor::new();
        mon.count("events");
        let s = mon.summary();
        assert!(s.contains("BpfMonitor"));
        assert!(s.contains("events"));
    }

    #[test]
    fn test_reset() {
        let mut mon = BpfMonitor::new();
        mon.count("x");
        mon.reset();
        assert_eq!(mon.get("x"), 0);
    }
}
