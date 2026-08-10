/// Informasi memori host, dibaca dari `/proc/meminfo` (Linux).
/// Bila tidak tersedia, semua field = 0.
#[derive(Debug, Clone)]
pub struct MemoryInfo {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

impl MemoryInfo {
    pub fn read() -> Self {
        let Ok(text) = std::fs::read_to_string("/proc/meminfo") else {
            return MemoryInfo { total_bytes: 0, available_bytes: 0 };
        };
        let mut total = 0u64;
        let mut available = 0u64;
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("MemTotal:") {
                total = parse_kb(v);
            } else if let Some(v) = line.strip_prefix("MemAvailable:") {
                available = parse_kb(v);
            }
        }
        MemoryInfo { total_bytes: total, available_bytes: available }
    }

    pub fn total_mib(&self) -> u64 {
        self.total_bytes / (1024 * 1024)
    }

    pub fn available_mib(&self) -> u64 {
        self.available_bytes / (1024 * 1024)
    }
}

/// Parse "1234 kB" → byte.
fn parse_kb(s: &str) -> u64 {
    s.split_whitespace()
        .next()
        .and_then(|n| n.parse::<u64>().ok())
        .unwrap_or(0)
        .saturating_mul(1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_kb() {
        assert_eq!(parse_kb("1234 kB"), 1234 * 1024);
        assert_eq!(parse_kb("0 kB"), 0);
        assert_eq!(parse_kb("abc"), 0);
    }

    #[test]
    fn test_memory_read() {
        let m = MemoryInfo::read();
        // Platform Linux (test ini) selalu punya /proc/meminfo → total > 0.
        if std::path::Path::new("/proc/meminfo").exists() {
            assert!(m.total_bytes > 0);
            assert!(m.total_mib() > 0);
        }
    }
}
