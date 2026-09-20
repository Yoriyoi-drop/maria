//! LINUX-04: HugePage support for large designs.
//!
//! Detects HugePage availability and provides configuration
//! for using transparent or static HugePages in simulation.

/// HugePage configuration.
#[derive(Debug, Clone)]
pub struct HugePageConfig {
    pub enabled: bool,
    pub page_size_kb: u64,
    pub total_pages: u64,
    pub free_pages: u64,
    pub use_transparent: bool,
}

impl HugePageConfig {
    /// Detect HugePage configuration.
    pub fn detect() -> Self {
        let page_size = read_sysfs("/sys/kernel/mm/transparent_hugepage/hugepage-kb")
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(2048);

        let total = read_sysfs("/proc/meminfo")
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("HugePages_Total:"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|s| s.parse().ok())
            })
            .unwrap_or(0);

        let free = read_sysfs("/proc/meminfo")
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("HugePages_Free:"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|s| s.parse().ok())
            })
            .unwrap_or(0);

        let thp = read_sysfs("/sys/kernel/mm/transparent_hugepage/enabled")
            .map(|s| s.contains("[always]") || s.contains("[madvise]"))
            .unwrap_or(false);

        HugePageConfig {
            enabled: total > 0,
            page_size_kb: page_size,
            total_pages: total,
            free_pages: free,
            use_transparent: thp,
        }
    }

    /// Recommended HugePage count for a given design size.
    pub fn recommended_pages(&self, design_size_mb: u64) -> u64 {
        if !self.enabled || self.page_size_kb == 0 {
            return 0;
        }
        let page_size_mb = self.page_size_kb / 1024;
        if page_size_mb == 0 {
            return 0;
        }
        // 50% overhead for metadata + alignment
        (design_size_mb * 3 / 2 + page_size_mb - 1) / page_size_mb
    }

    /// Check if HugePages are available for allocation.
    pub fn can_allocate(&self, pages: u64) -> bool {
        self.free_pages >= pages
    }

    /// Summary.
    pub fn summary(&self) -> String {
        format!(
            "HugePages: {} ({} KB each), free: {}, transparent: {}",
            if self.enabled { "enabled" } else { "disabled" },
            self.page_size_kb,
            self.free_pages,
            if self.use_transparent { "yes" } else { "no" },
        )
    }
}

fn read_sysfs(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect() {
        let config = HugePageConfig::detect();
        // Should not panic, even if HugePages are not configured
        let _ = config.summary();
    }

    #[test]
    fn test_recommended_pages() {
        let config = HugePageConfig {
            enabled: true,
            page_size_kb: 2048,
            total_pages: 1024,
            free_pages: 512,
            use_transparent: false,
        };
        let pages = config.recommended_pages(1024); // 1GB design
        assert!(pages > 0);
    }

    #[test]
    fn test_can_allocate() {
        let config = HugePageConfig {
            enabled: true,
            page_size_kb: 2048,
            total_pages: 10,
            free_pages: 5,
            use_transparent: false,
        };
        assert!(config.can_allocate(3));
        assert!(!config.can_allocate(10));
    }

    #[test]
    fn test_disabled() {
        let config = HugePageConfig {
            enabled: false,
            page_size_kb: 0,
            total_pages: 0,
            free_pages: 0,
            use_transparent: false,
        };
        assert_eq!(config.recommended_pages(1024), 0);
        assert!(!config.can_allocate(1));
    }
}
