/// Metadata build Maria (diambil dari env compile-time).
#[derive(Debug, Clone)]
pub struct BuildInfo {
    pub crate_name: &'static str,
    pub version: &'static str,
    pub profile: &'static str,
    pub target: String,
}

impl BuildInfo {
    pub fn current() -> Self {
        BuildInfo {
            crate_name: env!("CARGO_PKG_NAME"),
            version: env!("CARGO_PKG_VERSION"),
            profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            target: format!(
                "{}-{}-{}",
                std::env::consts::ARCH,
                std::env::consts::FAMILY,
                std::env::consts::OS
            ),
        }
    }

    /// "maria-0.3.0-debug" — identitas kompak untuk snapshot/cache key.
    pub fn short_id(&self) -> String {
        format!("{}-{}-{}", self.crate_name, self.version, self.profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_info() {
        let b = BuildInfo::current();
        assert_eq!(b.crate_name, "maria");
        assert!(!b.version.is_empty());
        assert!(b.profile == "debug" || b.profile == "release");
        assert!(!b.target.is_empty());
        assert!(b.short_id().starts_with("maria-0.3.0-"));
    }
}
