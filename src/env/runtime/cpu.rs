/// Informasi CPU host — dideteksi saat `RuntimeContext::init`.
#[derive(Debug, Clone)]
pub struct CpuInfo {
    pub logical_cores: usize,
    pub physical_cores: usize,
    pub model: String,
}

impl CpuInfo {
    pub fn detect() -> Self {
        CpuInfo {
            logical_cores: num_cpus::get(),
            physical_cores: num_cpus::get_physical(),
            model: read_cpu_model(),
        }
    }
}

/// Baca nama prosesor dari `/proc/cpuinfo` (Linux). Fallback "unknown".
fn read_cpu_model() -> String {
    let Ok(text) = std::fs::read_to_string("/proc/cpuinfo") else {
        return "unknown".into();
    };
    for line in text.lines() {
        let Some(v) = line.strip_prefix("model name") else {
            continue;
        };
        if let Some(idx) = v.find(':') {
            let model = v[idx + 1..].trim();
            if !model.is_empty() {
                return model.to_string();
            }
        }
    }
    "unknown".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpu_detect() {
        let cpu = CpuInfo::detect();
        assert!(cpu.logical_cores >= 1);
        assert!(cpu.physical_cores >= 1);
        assert!(!cpu.model.is_empty());
    }
}
