//! Coverage-Directed Test Generation (CDG) — utility seed vs target coverage.
//!
//! Paper #20 (Fine & Ziv, HLDVT 2003): arahkan pembangkitan test ke target
//! coverage yang belum terpenuhi, bukan eksplorasi seragam.
//! Dipakai fuzzer utk memprioritaskan fitur yang belum pernah terexercise
//! (target = fitur unreached dari feature map).

use crate::feature::FeatureMap;

/// Info coverage utk reporting CDG (#20).
#[derive(Debug, Clone, Default)]
pub struct CdgInfo {
    pub targets_total: usize,
    pub targets_hit: usize,
    pub ratio: f64,
}

/// Rencana target: fitur yang belum pernah terlihat = target CDG.
pub fn plan_targets(map: &FeatureMap) -> Vec<String> {
    crate::feature::TRACKED
        .iter()
        .filter(|t| !map.counts.contains_key(**t))
        .map(|t| t.to_string())
        .collect()
}

/// Hitung progress CDG terhadap seluruh fitur yang dikenal.
pub fn report(map: &FeatureMap) -> CdgInfo {
    let total = crate::feature::TRACKED.len();
    let hit = map
        .counts
        .keys()
        .filter(|k| crate::feature::TRACKED.contains(&k.as_str()))
        .count();
    CdgInfo {
        targets_total: total,
        targets_hit: hit,
        ratio: if total == 0 { 0.0 } else { hit as f64 / total as f64 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_targets_lists_unreached() {
        let mut map = FeatureMap::new();
        map.record(&["+".to_string(), "always_ff".to_string()]);
        let targets = plan_targets(&map);
        assert!(targets.contains(&"*".to_string()));
        assert!(!targets.contains(&"+".to_string()));
    }

    #[test]
    fn report_ratio_between_zero_one() {
        let map = FeatureMap::new();
        let r = report(&map);
        assert_eq!(r.targets_hit, 0);
        assert_eq!(r.targets_total, crate::feature::TRACKED.len());
        assert_eq!(r.ratio, 0.0);
    }
}