//! Resource monitor — sampel CPU, RAM, thread, dan load system dari `/proc`
//! (Linux, tanpa dependensi baru). Fallback: semua nol di platform lain.
//!
//! `ResourceState` disimpan di `GuiState` dan di-refresh berkala (tiap 1s)
//! oleh status bar — bar CPU/RAM + tooltip detail.

use eframe::egui::Color32;
use std::time::Instant;

/// Satu snapshot resource.
#[derive(Debug, Clone)]
pub struct ResourceState {
    /// Pemakaian CPU sejak sampel terakhir (0..100). -1 = belum ada data.
    pub cpu_percent: f64,
    /// RAM terpakai (GB).
    pub mem_used_gb: f64,
    /// RAM total (GB).
    pub mem_total_gb: f64,
    /// Jumlah thread proses maria.
    pub threads: usize,
    /// Load average 1 menit.
    pub load_1: f64,
    /// Waktu sampel terakhir (untuk throttling).
    last: Instant,
    /// Total jiffies CPU pada sampel terakhir (untuk delta).
    prev_total: u64,
    /// Jiffies idle pada sampel terakhir.
    prev_idle: u64,
}

impl Default for ResourceState {
    fn default() -> Self {
        Self {
            cpu_percent: -1.0,
            mem_used_gb: 0.0,
            mem_total_gb: 0.0,
            threads: 0,
            load_1: 0.0,
            last: Instant::now(),
            prev_total: 0,
            prev_idle: 0,
        }
    }
}

impl ResourceState {
    /// Refresh sampel jika sudah lewat `interval`. Dipanggil tiap frame —
    /// throttling internal mencegah pembacaan /proc berlebihan. Mengembalikan
    /// true jika sampel baru benar-benar diambil (pemanggil memakai ini untuk
    /// men-push riwayat grafis sekali per detik — bukan per frame, mencegah
    /// duplikat saat status bar & panel Benchmark sama-sama memanggil).
    pub fn refresh(&mut self, interval: std::time::Duration) -> bool {
        if self.last.elapsed() < interval {
            return false;
        }
        self.last = Instant::now();
        self.sample();
        true
    }

    /// Warna status CPU (hijau/kuning/merah) — dipakai bersama oleh status bar
    /// dan panel Benchmark agar konsisten (satu sumber kebenaran).
    pub fn cpu_color(&self) -> Color32 {
        if self.cpu_percent > 80.0 {
            Color32::from_rgb(239, 68, 68)
        } else if self.cpu_percent > 50.0 {
            Color32::from_rgb(234, 179, 8)
        } else {
            Color32::from_rgb(34, 197, 94)
        }
    }

    /// Baca semua metrik sekaligus (tanpa throttling — internal).
    fn sample(&mut self) {
        // ── CPU (delta jiffies dari /proc/stat) ──
        if let Some((total, idle)) = read_cpu_jiffies() {
            if self.prev_total > 0 && total >= self.prev_total {
                let d_total = total - self.prev_total;
                let d_idle = idle - self.prev_idle.min(idle);
                if d_total > 0 {
                    let busy = d_total.saturating_sub(d_idle) as f64 / d_total as f64;
                    self.cpu_percent = (busy * 100.0).clamp(0.0, 100.0);
                }
            }
            self.prev_total = total;
            self.prev_idle = idle;
        } else {
            self.cpu_percent = -1.0;
        }

        // ── RAM (/proc/meminfo) ──
        if let Some((total_kb, avail_kb)) = read_meminfo() {
            self.mem_total_gb = total_kb as f64 / (1024.0 * 1024.0);
            self.mem_used_gb = (total_kb.saturating_sub(avail_kb)) as f64 / (1024.0 * 1024.0);
        }

        // ── Threads proses (/proc/self/status) ──
        self.threads = read_thread_count();

        // ── Load average (/proc/loadavg) ──
        self.load_1 = read_loadavg();
    }
}

/// Baca (total_jiffies, idle_jiffies) dari baris `cpu ` di /proc/stat.
fn read_cpu_jiffies() -> Option<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/stat").ok()?;
    parse_cpu_jiffies(&text)
}

/// Parse (total_jiffies, idle_jiffies) dari konten /proc/stat (testable).
fn parse_cpu_jiffies(text: &str) -> Option<(u64, u64)> {
    let line = text.lines().find(|l| l.starts_with("cpu "))?;
    let mut nums = line
        .split_whitespace()
        .skip(1) // lewati "cpu"
        .filter_map(|s| s.parse::<u64>().ok());
    let user = nums.next()?;
    let nice = nums.next()?;
    let system = nums.next()?;
    let idle = nums.next()?;
    let iowait = nums.next().unwrap_or(0);
    let irq = nums.next().unwrap_or(0);
    let softirq = nums.next().unwrap_or(0);
    let steal = nums.next().unwrap_or(0);
    let total = user + nice + system + idle + iowait + irq + softirq + steal;
    Some((total, idle + iowait))
}

/// Baca (total_kb, available_kb) dari /proc/meminfo.
fn read_meminfo() -> Option<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    parse_meminfo(&text)
}

/// Parse (total_kb, available_kb) dari konten /proc/meminfo (testable).
fn parse_meminfo(text: &str) -> Option<(u64, u64)> {
    let mut total = 0u64;
    let mut avail = 0u64;
    for line in text.lines() {
        if line.starts_with("MemTotal:") {
            total = line.split_whitespace().nth(1)?.parse().ok()?;
        } else if line.starts_with("MemAvailable:") {
            avail = line.split_whitespace().nth(1)?.parse().ok()?;
        }
    }
    Some((total, avail))
}

/// Baca jumlah thread proses saat ini dari /proc/self/status.
fn read_thread_count() -> usize {
    let text = match std::fs::read_to_string("/proc/self/status") {
        Ok(t) => t,
        Err(_) => return 0,
    };
    parse_thread_count(&text)
}

/// Parse jumlah thread dari konten /proc/self/status (testable).
fn parse_thread_count(text: &str) -> usize {
    text.lines()
        .find(|l| l.starts_with("Threads:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Baca load average 1 menit dari /proc/loadavg.
fn read_loadavg() -> f64 {
    let text = match std::fs::read_to_string("/proc/loadavg") {
        Ok(t) => t,
        Err(_) => return 0.0,
    };
    parse_loadavg(&text)
}

/// Parse load average 1 menit dari konten /proc/loadavg (testable).
fn parse_loadavg(text: &str) -> f64 {
    text.split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_jiffies_parsing() {
        let stat = "cpu  100 20 30 40 10 5 2 1\ncpu0 50 10 15 20 5 2 1 1\n";
        let (total, idle) = parse_cpu_jiffies(stat).expect("cpu line exists");
        // 100+20+30+40+10+5+2+1 = 208 total; idle+iowait = 40+10 = 50
        assert_eq!(total, 208);
        assert_eq!(idle, 50);
    }

    #[test]
    fn cpu_jiffies_missing_line_returns_none() {
        assert!(parse_cpu_jiffies("intr 1234\n").is_none());
    }

    #[test]
    fn meminfo_parsing_kb() {
        let mem = "MemTotal:       16261528 kB\nMemFree:          1245 kB\nMemAvailable:   9000000 kB\n";
        let (total, avail) = parse_meminfo(mem).expect("meminfo parsed");
        assert_eq!(total, 16261528);
        assert_eq!(avail, 9000000);
    }

    #[test]
    fn meminfo_missing_total_defaults_to_zero() {
        // MemTotal tidak ada → total 0 (bukan None) — sample() menanganinya
        // via guard mem_total_gb > 0.0 di status bar.
        assert_eq!(parse_meminfo("MemFree: 123 kB\n"), Some((0, 0)));
    }

    #[test]
    fn thread_count_parsing() {
        let status = "Name:\tmaria\nThreads:\t12\nTgid:\t1234\n";
        assert_eq!(parse_thread_count(status), 12);
        assert_eq!(parse_thread_count("Name: x\n"), 0);
    }

    #[test]
    fn loadavg_parsing() {
        assert_eq!(parse_loadavg("1.25 0.50 0.10 2/345 6789\n"), 1.25);
        assert_eq!(parse_loadavg(""), 0.0);
    }
}
