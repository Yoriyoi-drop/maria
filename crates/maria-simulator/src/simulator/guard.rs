//! Resource guard untuk simulasi design besar (MARIA-SIM-34).
//!
//! Layer baru yang memantau konsumsi memori simulasi dan mencegah proses
//! di-kill oleh OOM-killer kernel. Design raksasa (mis. OpenTitan-chip)
//! mengeksekusi ratusan ribu process pada waktu-0; alokasi temporernya
//! melambungkan RSS hingga melebihi RAM fisik → swap-thrash (`folio_wait_bit`
//! / `rq_qos_wait`) → kernel OOM kill yang tidak memberi kesempatan
//! menyimpan waveform/checkpoint.
//!
//! Guard ini bekerja pada layer terpisah (1 file = 1 tanggung jawab):
//! - memantau RSS proses via `/proc/self/status` secara periodik;
//! - bila RSS melewati ambang batas → abort graceful dengan `SimError`
//!   ber-diagnostic yang menjelaskan penyebab & cara menghindari OOM
//!   (bukan mati mendadak oleh kernel);
//! - tanpa mengubah semantik simulasi — hanya menambah cek resource.
//!
//! Ambang batas dikonfigurasi via env `MARIA_SIM_MEM_LIMIT_MB`. Nilai `0`
//! = nonaktif. Bila tidak di-set, fallback: 60% total RAM sistem (dibatasi
//! paling tidak 512MB) supaya default tetap melindungi sekaligus tidak
//! mengganggu tes kecil.

use maria_core::diagnostics::{DiagCode, DiagLevel, Diagnostic};
use maria_core::error::SimError;

/// Tipe payload yang dibawa guard saat memicu abort.
pub struct ResourceLimit {
    limit_mb: u64,
    current_mb: u64,
}

impl ResourceLimit {
    pub fn limit_mb(&self) -> u64 {
        self.limit_mb
    }
    pub fn current_mb(&self) -> u64 {
        self.current_mb
    }
}

/// Guard memori simulasi. Instance dibuat sekali di `SimulationEngine::new`,
/// lalu di-poll pada titik-titik strategis (awal time step / saat delta
/// besar) oleh `run()`.
pub struct SimResourceGuard {
    /// Ambang RSS maksimum dalam MiB. `0` = nonaktif.
    limit_mb: u64,
    /// Slot RSS yang dimonitor di-cek sekali setiap N time step.
    check_interval: u64,
}

impl SimResourceGuard {
    /// Bangun guard dari env `MARIA_SIM_MEM_LIMIT_MB` atau default yang
    /// diturunkan dari total RAM sistem.
    pub fn from_env() -> Self {
        let limit_mb = std::env::var("MARIA_SIM_MEM_LIMIT_MB")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or_else(|| default_limit_mb());
        SimResourceGuard {
            limit_mb,
            check_interval: 64,
        }
    }

    /// Nonaktifkan guard (dipakai guard-by-default bisa di-off bila perlu).
    pub fn disabled() -> Self {
        SimResourceGuard {
            limit_mb: 0,
            check_interval: 64,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.limit_mb > 0
    }

    /// Interval (time step) antar pemeriksaan RSS.
    pub fn check_interval(&self) -> u64 {
        self.check_interval
    }

    pub fn limit_mb(&self) -> u64 {
        self.limit_mb
    }

    /// Set ambang (MiB). 0 = nonaktif.
    pub fn set_limit_mb(&mut self, mb: u64) {
        self.limit_mb = mb;
    }

    /// Poll RSS dan — bila melewati ambang — kembalikan error dengan
    /// diagnostic yang jelas. Dipanggil pada interval time-step.
    pub fn poll(&self, time: u64, tick: u64) -> Result<(), SimError> {
        if !self.is_enabled() {
            return Ok(());
        }
        // Hitung hanya pada interval tertentu agar baca /proc tidak overhead.
        if tick % self.check_interval != 0 {
            return Ok(());
        }
        self.check_rss(time)
    }

    /// Cek RSS tanpa gate interval — dipakai di delta loop (lonjakan time-0
    /// sering terjadi di delta kecil yang dilewati gate interval).
    pub fn check_limit(&self, time: u64) -> Result<(), SimError> {
        if !self.is_enabled() {
            return Ok(());
        }
        self.check_rss(time)
    }

    fn check_rss(&self, time: u64) -> Result<(), SimError> {
        match Self::rss_mb() {
            Some(rss_mb) if rss_mb > self.limit_mb => {
                let diag = Diagnostic::new(
                    DiagLevel::Error,
                    DiagCode::ResourceLimit,
                    format!(
                        "simulasi melebihi batas memori {limit} MiB (RSS kini {rss} MiB) pada waktu {time} ns. \
                         Design terlalu besar untuk RAM yang tersedia; kernel akan meng-OOM-kill proses.\n\
                         Cara mengatasi:\n\
                         \x20 • batasi waktu sim dengan `-T <ns>` (hindari `unlimited` untuk design non-terminating);\n\
                         \x20 • matikan VCD/FST yang tidak perlu atau gunakan waveform streaming (`--waveform-stream`);\n\
                         \x20 • naikkan batas via env `MARIA_SIM_MEM_LIMIT_MB=<lebih_besar>` bila RAM tersedia.",
                        limit = self.limit_mb,
                        rss = rss_mb
                    ),
                );
                Err(SimError::Diagnostic(diag))
            }
            _ => Ok(()),
        }
    }

    /// RSS proses saat ini dalam MiB, dibaca dari `/proc/self/status`.
    fn rss_mb() -> Option<u64> {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                // Format: "VmRSS:    123456 kB"
                let kb: u64 = rest
                    .split_whitespace()
                    .next()
                    .and_then(|v| v.parse().ok())?;
                return Some(kb / 1024);
            }
        }
        None
    }
}

/// Limit default: 60% RAM total, minimal 512 MiB, supaya default melindungi
/// design besar namun tidak menimpa tes kecil.
fn default_limit_mb() -> u64 {
    let total_kb = sysinfo_total_memory_kb().unwrap_or(0);
    if total_kb == 0 {
        return 0; // tidak bisa deteksi — nonaktif, hindari false positive
    }
    let pct = (total_kb / 1024) * 60 / 100;
    pct.max(512)
}

/// Baca total RAM sistem (MiB) dari `/proc/meminfo` MemTotal.
fn sysinfo_total_memory_kb() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in meminfo.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kb: u64 = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok())?;
            return Some(kb);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rss_reads_positive() {
        // RSS harus ter-baca > 0 pada proses yang sedang berjalan.
        assert!(SimResourceGuard::rss_mb().unwrap_or(0) > 0);
    }

    #[test]
    fn disabled_guard_never_polls() {
        let g = SimResourceGuard::disabled();
        assert!(!g.is_enabled());
        assert!(g.poll(0, 0).is_ok());
        assert!(g.limit_mb() == 0);
    }

    #[test]
    fn interval_skips_check() {
        let g = SimResourceGuard {
            limit_mb: 1, // sangat rendah — pasti melebihi
            check_interval: 64,
        };
        // tick bukan kelipatan 64 → skip, Ok
        assert!(g.poll(0, 63).is_ok());
    }

    #[test]
    fn over_limit_errors() {
        let g = SimResourceGuard {
            limit_mb: 1,
            check_interval: 1,
        };
        match g.poll(5, 0) {
            Err(SimError::Diagnostic(d)) => {
                assert_eq!(d.code, DiagCode::ResourceLimit);
            }
            other => panic!("expected ResourceLimit error, got {:?}", other.map(|_| ())),
        }
    }

    #[test]
    fn default_limit_sane() {
        let l = default_limit_mb();
        // Harus > 0 bila sistem punya meminfo (hampir selalu).
        let total = sysinfo_total_memory_kb().unwrap_or(0);
        if total > 0 {
            assert!(l >= 512);
        }
    }
}
