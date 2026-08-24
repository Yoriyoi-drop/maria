use super::build::BuildInfo;

/// String versi lengkap untuk output CLI/GUI, mis. "maria 0.3.0 (debug, x86_64-unknown-linux-gnu)".
pub fn version_string() -> String {
    let b = BuildInfo::current();
    format!(
        "{} {} ({}, {})",
        b.crate_name, b.version, b.profile, b.target
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_string() {
        let s = version_string();
        assert!(s.starts_with("maria 0.3.0"));
        assert!(s.contains("debug") || s.contains("release"));
    }
}
