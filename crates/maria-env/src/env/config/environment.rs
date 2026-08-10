//! Override config dari environment (`MARIA_*`).
//! Urutan kekuatan: CLI > Environment > file config > default.

use crate::env::config::ConfigContext;

/// Baca override `MARIA_*` dan terapkan ke config. Tidak error bila variabel
/// tidak ada / tidak valid — nilai tersebut diabaikan.
pub fn apply_env(ctx: &mut ConfigContext) {
    if let Some(v) = parse_usize(&std::env::var("MARIA_JOBS").unwrap_or_default()) {
        if v > 0 {
            ctx.mutate(|c| c.compiler.jobs = Some(v));
        }
    }
    if let Some(v) = parse_u64(&std::env::var("MARIA_MAX_TIME").unwrap_or_default()) {
        ctx.mutate(|c| c.simulation.max_time = Some(v));
    }
    if let Some(v) = parse_bool(&std::env::var("MARIA_FORCE_SIM").unwrap_or_default()) {
        ctx.mutate(|c| c.simulation.force_sim = Some(v));
    }
    if let Some(v) = parse_u8(&std::env::var("MARIA_OPT_LEVEL").unwrap_or_default()) {
        if v <= crate::env::config::defaults::MAX_OPT_LEVEL {
            ctx.mutate(|c| c.compiler.opt_level = Some(v));
        }
    }
    if let Some(v) = parse_bool(&std::env::var("MARIA_INCREMENTAL").unwrap_or_default()) {
        ctx.mutate(|c| c.compiler.incremental = Some(v));
    }
}

// ── Parser nilai (murni, mudah dites) ──

pub fn parse_usize(s: &str) -> Option<usize> {
    s.trim().parse().ok()
}

pub fn parse_u64(s: &str) -> Option<u64> {
    s.trim().parse().ok()
}

pub fn parse_u8(s: &str) -> Option<u8> {
    s.trim().parse().ok()
}

pub fn parse_bool(s: &str) -> Option<bool> {
    match s.trim() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_usize() {
        assert_eq!(parse_usize("4"), Some(4));
        assert_eq!(parse_usize(""), None);
        assert_eq!(parse_usize("abc"), None);
    }

    #[test]
    fn test_parse_u64() {
        assert_eq!(parse_u64(" 1000 "), Some(1000));
        assert_eq!(parse_u64("-5"), None);
    }

    #[test]
    fn test_parse_bool() {
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("1"), Some(true));
        assert_eq!(parse_bool("off"), Some(false));
        assert_eq!(parse_bool(""), None);
        assert_eq!(parse_bool("maybe"), None);
    }
}
