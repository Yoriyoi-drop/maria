//! LINUX-09: Built-in flamegraph generation — folded stacks format.
//!
//! Mengumpulkan sampling data dari simulation engine dan menghasilkan
//! output dalam "folded stacks" format yang kompatibel dengan
//! `flamegraph.pl` / `inferno` / `BrendanGregg/FlameGraph`.
//!
//! Format: `stack;frame1;frame2;... count`
//!
//! Contoh output:
//! ```text
//! SimulationEngine::run;Process::Sequential;BlockingAssign 150
//! SimulationEngine::run;Process::CombReactive;BlockingAssign 85
//! SimulationEngine::run;EventLoop;push_event 42
//! ```

use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Profiler untuk sampling-based flamegraph generation.
pub struct FlamegraphProfiler {
    /// Sampling interval (default: 1ms)
    interval: Duration,
    /// Collected samples: stack_trace → count
    samples: Mutex<HashMap<String, u64>>,
    /// Start time
    start: Mutex<Instant>,
    /// Whether profiler is active
    active: Mutex<bool>,
    /// Total samples taken
    total_samples: Mutex<u64>,
}

impl Default for FlamegraphProfiler {
    fn default() -> Self {
        Self::new()
    }
}

impl FlamegraphProfiler {
    /// Buat profiler baru dengan sampling interval default (1ms).
    pub fn new() -> Self {
        FlamegraphProfiler {
            interval: Duration::from_millis(1),
            samples: Mutex::new(HashMap::new()),
            start: Mutex::new(Instant::now()),
            active: Mutex::new(false),
            total_samples: Mutex::new(0),
        }
    }

    /// Buat profiler dengan custom interval.
    pub fn with_interval(interval: Duration) -> Self {
        FlamegraphProfiler {
            interval,
            ..Self::new()
        }
    }

    /// Mulai profiling.
    pub fn start(&self) {
        *self.active.lock().unwrap() = true;
        *self.start.lock().unwrap() = Instant::now();
    }

    /// Stop profiling.
    pub fn stop(&self) {
        *self.active.lock().unwrap() = false;
    }

    /// Record satu sample dengan stack trace.
    ///
    /// `frames` harus diurutkan dari leaf ke root (frame paling dalam duluan).
    pub fn record(&self, frames: &[&str]) {
        if !*self.active.lock().unwrap() {
            return;
        }
        let stack = frames.join(";");
        if let Ok(mut samples) = self.samples.lock() {
            *samples.entry(stack).or_insert(0) += 1;
        }
        if let Ok(mut total) = self.total_samples.lock() {
            *total += 1;
        }
    }

    /// Record satu sample dari string stack trace (folded format).
    pub fn record_folded(&self, stack: &str) {
        if !*self.active.lock().unwrap() {
            return;
        }
        if let Ok(mut samples) = self.samples.lock() {
            *samples.entry(stack.to_string()).or_insert(0) += 1;
        }
        if let Ok(mut total) = self.total_samples.lock() {
            *total += 1;
        }
    }

    /// Get sampling interval.
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Get total samples collected.
    pub fn total_samples(&self) -> u64 {
        *self.total_samples.lock().unwrap()
    }

    /// Get elapsed time since start.
    pub fn elapsed(&self) -> Duration {
        self.start.lock().unwrap().elapsed()
    }

    /// Export data dalam folded stacks format.
    pub fn export_folded(&self) -> String {
        let samples = self.samples.lock().unwrap();
        let mut output = String::with_capacity(samples.len() * 64);
        for (stack, count) in samples.iter() {
            let _ = writeln!(output, "{} {}", stack, count);
        }
        output
    }

    /// Save folded stacks ke file.
    pub fn save_folded(&self, path: &Path) -> Result<(), String> {
        let content = self.export_folded();
        std::fs::write(path, &content).map_err(|e| format!("gagal tulis {}: {}", path.display(), e))
    }

    /// Save folded stacks + header comment ke file.
    pub fn save_with_header(&self, path: &Path) -> Result<(), String> {
        let samples = self.samples.lock().unwrap();
        let total = *self.total_samples.lock().unwrap();
        let elapsed = self.elapsed();

        let mut content = String::with_capacity(samples.len() * 64 + 256);
        let _ = writeln!(content, "# Flamegraph data — Maria HDL Simulator");
        let _ = writeln!(content, "# Total samples: {}", total);
        let _ = writeln!(content, "# Elapsed: {:.2}s", elapsed.as_secs_f64());
        let _ = writeln!(content, "# Interval: {}ms", self.interval.as_millis());
        let _ = writeln!(content, "# Stacks: {}", samples.len());
        let _ = writeln!(content);

        for (stack, count) in samples.iter() {
            let _ = writeln!(content, "{} {}", stack, count);
        }

        std::fs::write(path, &content).map_err(|e| format!("gagal tulis {}: {}", path.display(), e))
    }

    /// Get sorted samples by count (descending).
    pub fn top_stacks(&self, n: usize) -> Vec<(String, u64)> {
        let samples = self.samples.lock().unwrap();
        let mut sorted: Vec<(String, u64)> = samples.iter().map(|(k, v)| (k.clone(), *v)).collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        sorted.truncate(n);
        sorted
    }

    /// Clear all collected samples.
    pub fn reset(&self) {
        if let Ok(mut samples) = self.samples.lock() {
            samples.clear();
        }
        *self.total_samples.lock().unwrap() = 0;
    }
}

impl std::fmt::Display for FlamegraphProfiler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let total = self.total_samples();
        let elapsed = self.elapsed();
        let stacks = self.samples.lock().unwrap().len();
        writeln!(f, "═══ Flamegraph Profiler ═══")?;
        writeln!(f, "Samples:   {:>8}", total)?;
        writeln!(f, "Stacks:    {:>8}", stacks)?;
        writeln!(f, "Elapsed:   {:>8.2} s", elapsed.as_secs_f64())?;
        writeln!(f, "Interval:  {:>8} ms", self.interval.as_millis())?;
        if total > 0 {
            writeln!(
                f,
                "Rate:      {:>8.0} samples/s",
                total as f64 / elapsed.as_secs_f64()
            )?;
        }
        writeln!(f)?;
        writeln!(f, "Top stacks:")?;
        for (stack, count) in self.top_stacks(10) {
            let pct = count as f64 / total as f64 * 100.0;
            writeln!(f, "  {:>6.1}% {:>8} {}", pct, count, stack)?;
        }
        Ok(())
    }
}

/// Demo: buat sample folded stacks untuk testing.
pub fn demo_folded() -> String {
    let mut p = FlamegraphProfiler::new();
    p.start();
    p.record(&[
        "BlockingAssign",
        "Process::Sequential",
        "SimulationEngine::run",
    ]);
    p.record(&[
        "BlockingAssign",
        "Process::Sequential",
        "SimulationEngine::run",
    ]);
    p.record(&[
        "BlockingAssign",
        "Process::Sequential",
        "SimulationEngine::run",
    ]);
    p.record(&[
        "evaluate_expr",
        "BlockingAssign",
        "Process::CombReactive",
        "SimulationEngine::run",
    ]);
    p.record(&[
        "evaluate_expr",
        "BlockingAssign",
        "Process::CombReactive",
        "SimulationEngine::run",
    ]);
    p.record(&["push_event", "SimulationEngine::run"]);
    p.record(&["push_event", "SimulationEngine::run"]);
    p.record(&["push_event", "SimulationEngine::run"]);
    p.record(&["push_event", "SimulationEngine::run"]);
    p.export_folded()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flamegraph_basic() {
        let p = FlamegraphProfiler::new();
        p.start();
        p.record(&["frame_a", "frame_b", "frame_c"]);
        p.record(&["frame_a", "frame_b", "frame_c"]);
        p.record(&["frame_x", "frame_y"]);
        assert_eq!(p.total_samples(), 3);
        let folded = p.export_folded();
        assert!(folded.contains("frame_a;frame_b;frame_c 2"));
        assert!(folded.contains("frame_x;frame_y 1"));
    }

    #[test]
    fn test_flamegraph_inactive() {
        let p = FlamegraphProfiler::new();
        // Don't start — samples should not be recorded
        p.record(&["should_not_appear"]);
        assert_eq!(p.total_samples(), 0);
    }

    #[test]
    fn test_flamegraph_top_stacks() {
        let p = FlamegraphProfiler::new();
        p.start();
        p.record(&["hot_path"]);
        p.record(&["hot_path"]);
        p.record(&["hot_path"]);
        p.record(&["cold_path"]);
        let top = p.top_stacks(1);
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].0, "hot_path");
        assert_eq!(top[0].1, 3);
    }

    #[test]
    fn test_flamegraph_save() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("profile.folded");
        let p = FlamegraphProfiler::new();
        p.start();
        p.record(&["main", "foo"]);
        p.save_folded(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("main;foo 1"));
    }

    #[test]
    fn test_flamegraph_save_with_header() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("profile.folded");
        let p = FlamegraphProfiler::new();
        p.start();
        p.record(&["a", "b"]);
        p.save_with_header(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("# Flamegraph data"));
        assert!(content.contains("# Total samples: 1"));
        assert!(content.contains("a;b 1"));
    }

    #[test]
    fn test_flamegraph_reset() {
        let p = FlamegraphProfiler::new();
        p.start();
        p.record(&["x"]);
        p.record(&["y"]);
        assert_eq!(p.total_samples(), 2);
        p.reset();
        assert_eq!(p.total_samples(), 0);
        assert!(p.export_folded().is_empty());
    }

    #[test]
    fn test_flamegraph_demo() {
        let folded = demo_folded();
        assert!(folded.contains("BlockingAssign;Process::Sequential;SimulationEngine::run 3"));
        assert!(folded.contains("push_event;SimulationEngine::run 4"));
    }

    #[test]
    fn test_flamegraph_display() {
        let p = FlamegraphProfiler::new();
        p.start();
        p.record(&["a", "b"]);
        let s = format!("{}", p);
        assert!(s.contains("Flamegraph Profiler"));
        assert!(s.contains("Samples:"));
        assert!(s.contains("Stacks:"));
    }
}
