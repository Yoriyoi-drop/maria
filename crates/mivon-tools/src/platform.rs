//! DEBT-04: no_std Compatibility — feature gate marker + platform detection.
//!
//! Provides compile-time platform detection and conditional compilation
//! helpers for future no_std and cross-platform support.
//!
//! Note: Full no_std is not yet supported — this module provides the
//! foundation markers and platform abstraction layer for future work.

/// Platform type detected at compile time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    MacOs,
    Windows,
    Wasm,
    Unknown,
}

impl Platform {
    /// Detect current platform at compile time.
    pub fn detect() -> Self {
        #[cfg(target_os = "linux")]
        {
            Platform::Linux
        }
        #[cfg(target_os = "macos")]
        {
            Platform::MacOs
        }
        #[cfg(target_os = "windows")]
        {
            Platform::Windows
        }
        #[cfg(target_arch = "wasm32")]
        {
            Platform::Wasm
        }
        #[cfg(not(any(
            target_os = "linux",
            target_os = "macos",
            target_os = "windows",
            target_arch = "wasm32"
        )))]
        {
            Platform::Unknown
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Platform::Linux => "linux",
            Platform::MacOs => "macos",
            Platform::Windows => "windows",
            Platform::Wasm => "wasm32",
            Platform::Unknown => "unknown",
        }
    }

    pub fn supports_threading(&self) -> bool {
        !matches!(self, Platform::Wasm)
    }

    pub fn supports_filesystem(&self) -> bool {
        !matches!(self, Platform::Wasm)
    }

    pub fn supports_networking(&self) -> bool {
        !matches!(self, Platform::Wasm)
    }
}

/// Thread pool abstraction (platform-aware).
pub struct ThreadPool {
    platform: Platform,
    size: usize,
}

impl ThreadPool {
    pub fn new(size: usize) -> Self {
        ThreadPool {
            platform: Platform::detect(),
            size,
        }
    }

    pub fn recommended_size() -> usize {
        let platform = Platform::detect();
        if !platform.supports_threading() {
            return 1;
        }
        // Simple heuristic: use available parallelism or fallback to 4
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    }

    pub fn platform(&self) -> Platform {
        self.platform
    }

    pub fn size(&self) -> usize {
        self.size
    }
}

/// Feature flags for conditional compilation.
pub mod features {
    /// Whether SIMD acceleration is available.
    pub const HAS_SIMD: bool = cfg!(target_arch = "x86_64") || cfg!(target_arch = "aarch64");

    /// Whether atomic operations are available (needed for lock-free structures).
    pub const HAS_ATOMICS: bool = true; // All Rust targets support atomics

    /// Whether OS-level threading is available.
    pub const HAS_THREADS: bool = !cfg!(target_arch = "wasm32");

    /// Whether file I/O is available.
    pub const HAS_FILESYSTEM: bool = !cfg!(target_arch = "wasm32");

    /// Whether network I/O is available.
    pub const HAS_NETWORK: bool = !cfg!(target_arch = "wasm32");

    /// Whether the platform supports memory-mapped files.
    pub const HAS_MMAP: bool = cfg!(target_os = "linux") || cfg!(target_os = "macos");

    /// Whether the platform supports io_uring.
    pub const HAS_IO_URING: bool = cfg!(target_os = "linux");

    /// Whether the platform supports epoll.
    pub const HAS_EPOLL: bool = cfg!(target_os = "linux");

    /// Whether the platform supports kqueue.
    pub const HAS_KQUEUE: bool = cfg!(target_os = "macos");
}

/// Conditional compilation helper macro.
///
/// # Example
/// ```ignore
/// platform_dispatch! {
///     linux => { setup_epoll(); }
///     macos => { setup_kqueue(); }
///     default => { setup_select(); }
/// }
/// ```
#[macro_export]
macro_rules! platform_dispatch {
    (linux => { $($linux_body:tt)* } macos => { $($macos_body:tt)* } default => { $($default_body:tt)* }) => {
        #[cfg(target_os = "linux")]
        { $($linux_body)* }
        #[cfg(target_os = "macos")]
        { $($macos_body)* }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        { $($default_body)* }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_detect() {
        let p = Platform::detect();
        // We're running on Linux
        assert_eq!(p, Platform::Linux);
    }

    #[test]
    fn test_platform_name() {
        assert_eq!(Platform::Linux.name(), "linux");
        assert_eq!(Platform::MacOs.name(), "macos");
        assert_eq!(Platform::Windows.name(), "windows");
    }

    #[test]
    fn test_thread_pool() {
        let pool = ThreadPool::new(4);
        assert_eq!(pool.size(), 4);
        assert!(pool.platform().supports_threading());
    }

    #[test]
    fn test_recommended_size() {
        let size = ThreadPool::recommended_size();
        assert!(size >= 1);
    }

    #[test]
    fn test_features() {
        assert!(features::HAS_SIMD);
        assert!(features::HAS_ATOMICS);
        assert!(features::HAS_THREADS);
        assert!(features::HAS_FILESYSTEM);
        assert!(features::HAS_MMAP);
    }
}
