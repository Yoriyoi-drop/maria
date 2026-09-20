//! LINUX-15: KVM/VM acceleration for simulation.
//!
//! Detection and configuration for running simulation inside
//! KVM virtual machines for isolation and resource control.

use serde::{Deserialize, Serialize};

/// KVM availability status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KvmStatus {
    pub available: bool,
    pub version: Option<String>,
    pub max_vcpus: u32,
    pub has_tsc: bool,
    pub has_msr: bool,
    pub has_xsave: bool,
}

/// VM configuration for simulation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmConfig {
    pub vcpus: u32,
    pub memory_mb: u64,
    pub enable_hugepages: bool,
    pub enable_nesting: bool,
    pub cpu_model: String,
    pub kernel_path: Option<String>,
    pub initrd_path: Option<String>,
    pub extra_args: Vec<String>,
}

impl Default for VmConfig {
    fn default() -> Self {
        VmConfig {
            vcpus: 4,
            memory_mb: 4096,
            enable_hugepages: true,
            enable_nesting: false,
            cpu_model: "host".into(),
            kernel_path: None,
            initrd_path: None,
            extra_args: Vec::new(),
        }
    }
}

/// Detect KVM availability.
pub fn detect_kvm() -> KvmStatus {
    let mut status = KvmStatus {
        available: false,
        version: None,
        max_vcpus: 0,
        has_tsc: false,
        has_msr: false,
        has_xsave: false,
    };

    #[cfg(target_os = "linux")]
    {
        // Check /dev/kvm
        if std::path::Path::new("/dev/kvm").exists() {
            status.available = true;
        }

        // Read version from dmesg or /sys
        if let Ok(content) = std::fs::read_to_string("/sys/module/kvm/version") {
            status.version = Some(content.trim().to_string());
        }

        // Check capabilities from /proc/cpuinfo
        if let Ok(content) = std::fs::read_to_string("/proc/cpuinfo") {
            if content.contains("tsc") {
                status.has_tsc = true;
            }
            if content.contains("msr") {
                status.has_msr = true;
            }
            if content.contains("xsave") {
                status.has_xsave = true;
            }
        }

        // Max vCPUs
        status.max_vcpus = std::thread::available_parallelism()
            .map(|n| n.get() as u32)
            .unwrap_or(1);
    }

    status
}

/// Generate QEMU command line for simulation VM.
pub fn generate_qemu_command(config: &VmConfig) -> String {
    let mut cmd = format!(
        "qemu-system-x86_64 -enable-kvm -smp {} -m {}",
        config.vcpus, config.memory_mb,
    );

    if config.enable_hugepages {
        cmd.push_str(" -mem-path /dev/hugepages");
    }

    if let Some(ref kernel) = config.kernel_path {
        cmd.push_str(&format!(" -kernel {}", kernel));
    }
    if let Some(ref initrd) = config.initrd_path {
        cmd.push_str(&format!(" -initrd {}", initrd));
    }

    for arg in &config.extra_args {
        cmd.push_str(&format!(" {}", arg));
    }

    cmd
}

/// Summary.
pub fn summary(status: &KvmStatus) -> String {
    format!(
        "KVM: {} (v{}), {} vCPUs, tsc={}, xsave={}",
        if status.available {
            "available"
        } else {
            "not available"
        },
        status.version.as_deref().unwrap_or("unknown"),
        status.max_vcpus,
        status.has_tsc,
        status.has_xsave,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_kvm() {
        let status = detect_kvm();
        // Should not panic, even if KVM is not available
        let _ = summary(&status);
    }

    #[test]
    fn test_vm_config_default() {
        let config = VmConfig::default();
        assert_eq!(config.vcpus, 4);
        assert!(config.enable_hugepages);
    }

    #[test]
    fn test_generate_qemu_command() {
        let config = VmConfig::default();
        let cmd = generate_qemu_command(&config);
        assert!(cmd.contains("qemu-system-x86_64"));
        assert!(cmd.contains("-enable-kvm"));
        assert!(cmd.contains("-smp 4"));
    }

    #[test]
    fn test_summary() {
        let status = KvmStatus {
            available: true,
            version: Some("5.15.0".into()),
            max_vcpus: 8,
            has_tsc: true,
            has_msr: false,
            has_xsave: true,
        };
        let s = summary(&status);
        assert!(s.contains("available"));
        assert!(s.contains("5.15.0"));
    }
}
