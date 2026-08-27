//! LINUX-11: Cgroup-aware Resource Limits.
//!
//! Mendeteksi batasan resource dari cgroup (container/Docker/systemd slice)
//! dan mengembalikan batasan yang bisa dipakai untuk throttling Maria.
//!
//! Cgroup v2: `/sys/fs/cgroup/` atau `/proc/self/cgroup`
//! Cgroup v1: `/sys/fs/cgroup/memory/` dll

use std::path::Path;

/// Resource limits dari cgroup.
#[derive(Debug, Clone, Default)]
pub struct CgroupLimits {
    /// Batas memori dalam bytes (None = unlimited)
    pub memory_limit_bytes: Option<u64>,
    /// Batas CPU quota dalam microseconds per period (None = unlimited)
    pub cpu_quota_us: Option<u64>,
    /// CPU period dalam microseconds (default: 100000 = 100ms)
    pub cpu_period_us: Option<u64>,
    /// Jumlah CPU cores yang diizinkan (None = unlimited)
    pub cpu_cores: Option<f64>,
    /// Apakah berjalan di dalam cgroup/container
    pub in_cgroup: bool,
}

impl CgroupLimits {
    /// Deteksi cgroup limits dari environment saat ini.
    pub fn detect() -> Self {
        #[cfg(target_os = "linux")]
        {
            Self::detect_linux()
        }
        #[cfg(not(target_os = "linux"))]
        {
            CgroupLimits::default()
        }
    }

    #[cfg(target_os = "linux")]
    fn detect_linux() -> Self {
        // Coba cgroup v2 dulu, lalu v1
        if let Some(limits) = Self::try_cgroup_v2() {
            return limits;
        }
        Self::try_cgroup_v1()
    }

    #[cfg(target_os = "linux")]
    fn try_cgroup_v2() -> Option<Self> {
        let cgroup_path = Self::find_cgroup_v2_mount()?;
        let mut limits = CgroupLimits {
            in_cgroup: true,
            ..Default::default()
        };

        // Memory limit: memory.max
        let mem_max = cgroup_path.join("memory.max");
        if let Ok(content) = std::fs::read_to_string(&mem_max) {
            let content = content.trim();
            if content != "max" {
                if let Ok(bytes) = content.parse::<u64>() {
                    limits.memory_limit_bytes = Some(bytes);
                }
            }
        }

        // CPU quota: cpu.max (format: "quota period")
        let cpu_max = cgroup_path.join("cpu.max");
        if let Ok(content) = std::fs::read_to_string(&cpu_max) {
            let parts: Vec<&str> = content.trim().split_whitespace().collect();
            if parts.len() == 2 {
                if parts[0] != "max" {
                    if let (Ok(quota), Ok(period)) = (parts[0].parse::<u64>(), parts[1].parse::<u64>()) {
                        limits.cpu_quota_us = Some(quota);
                        limits.cpu_period_us = Some(period);
                        if period > 0 {
                            limits.cpu_cores = Some(quota as f64 / period as f64);
                        }
                    }
                }
            }
        }

        Some(limits)
    }

    #[cfg(target_os = "linux")]
    fn try_cgroup_v1() -> Self {
        let mut limits = CgroupLimits::default();

        // Memory limit: /sys/fs/cgroup/memory/memory.limit_in_bytes
        let mem_limit = Path::new("/sys/fs/cgroup/memory/memory.limit_in_bytes");
        if let Ok(content) = std::fs::read_to_string(mem_limit) {
            if let Ok(bytes) = content.trim().parse::<u64>() {
                // cgroup v1 uses very large number (like 9223372036854771712) for unlimited
                if bytes < u64::MAX / 2 {
                    limits.memory_limit_bytes = Some(bytes);
                    limits.in_cgroup = true;
                }
            }
        }

        // CPU quota: /sys/fs/cgroup/cpu/cpu.cfs_quota_us
        let cpu_quota = Path::new("/sys/fs/cgroup/cpu/cpu.cfs_quota_us");
        let cpu_period = Path::new("/sys/fs/cgroup/cpu/cpu.cfs_period_us");
        if let (Ok(quota_s), Ok(period_s)) = (
            std::fs::read_to_string(cpu_quota),
            std::fs::read_to_string(cpu_period),
        ) {
            if let (Ok(quota), Ok(period)) = (
                quota_s.trim().parse::<u64>(),
                period_s.trim().parse::<u64>(),
            ) {
                if quota > 0 && period > 0 {
                    limits.cpu_quota_us = Some(quota);
                    limits.cpu_period_us = Some(period);
                    limits.cpu_cores = Some(quota as f64 / period as f64);
                    limits.in_cgroup = true;
                }
            }
        }

        limits
    }

    #[cfg(target_os = "linux")]
    fn find_cgroup_v2_mount() -> Option<std::path::PathBuf> {
        // Check /proc/self/cgroup for cgroup v2
        let proc_cgroup = std::fs::read_to_string("/proc/self/cgroup").ok()?;
        for line in proc_cgroup.lines() {
            if line.contains("0::") {
                // Cgroup v2: mount point at /sys/fs/cgroup + path
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 3 {
                    let path = parts[2].trim_start_matches('/');
                    let mount = std::path::PathBuf::from(format!("/sys/fs/cgroup/{}", path));
                    if mount.exists() {
                        return Some(mount);
                    }
                }
            }
        }
        None
    }

    /// Apakah ada batasan memori aktif.
    pub fn has_memory_limit(&self) -> bool {
        self.memory_limit_bytes.is_some()
    }

    /// Apakah ada batasan CPU aktif.
    pub fn has_cpu_limit(&self) -> bool {
        self.cpu_quota_us.is_some()
    }

    /// Batas memori dalam MB (None = unlimited).
    pub fn memory_limit_mb(&self) -> Option<u64> {
        self.memory_limit_bytes.map(|b| b / (1024 * 1024))
    }

    /// Display singkat.
    pub fn summary(&self) -> String {
        if !self.in_cgroup {
            return "outside cgroup".into();
        }
        let mem = self.memory_limit_mb()
            .map(|m| format!("{}MB", m))
            .unwrap_or_else(|| "unlimited".into());
        let cpu = self.cpu_cores
            .map(|c| format!("{:.1} cores", c))
            .unwrap_or_else(|| "unlimited".into());
        format!("mem={} cpu={}", mem, cpu)
    }
}

impl std::fmt::Display for CgroupLimits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "═══ Cgroup Limits ═══")?;
        writeln!(f, "In cgroup:    {}", self.in_cgroup)?;
        writeln!(f, "Memory:       {}", self.memory_limit_mb()
            .map(|m| format!("{} MB", m))
            .unwrap_or_else(|| "unlimited".into()))?;
        writeln!(f, "CPU cores:    {}", self.cpu_cores
            .map(|c| format!("{:.1}", c))
            .unwrap_or_else(|| "unlimited".into()))?;
        if let Some(quota) = self.cpu_quota_us {
            writeln!(f, "CPU quota:    {} us", quota)?;
        }
        if let Some(period) = self.cpu_period_us {
            writeln!(f, "CPU period:   {} us", period)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cgroup_detect() {
        let limits = CgroupLimits::detect();
        // On CI, might be in a container
        println!("cgroup: {}", limits);
        println!("summary: {}", limits.summary());
    }

    #[test]
    fn test_cgroup_no_limit() {
        let limits = CgroupLimits::default();
        assert!(!limits.has_memory_limit());
        assert!(!limits.has_cpu_limit());
        assert_eq!(limits.memory_limit_mb(), None);
        assert_eq!(limits.summary(), "outside cgroup");
    }

    #[test]
    fn test_cgroup_display() {
        let mut limits = CgroupLimits::default();
        limits.in_cgroup = true;
        limits.memory_limit_bytes = Some(2 * 1024 * 1024 * 1024); // 2GB
        limits.cpu_cores = Some(2.0);
        let s = format!("{}", limits);
        assert!(s.contains("Cgroup Limits"));
        assert!(s.contains("2048 MB"));
        assert!(s.contains("2.0"));
    }
}
