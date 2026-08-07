use crate::simulator::coverage_db::CoverageDatabase;
use crate::simulator::SimulationEngine;

/// CoverageStats — snapshot coverage pasca simulasi.
#[derive(Debug, Clone, Default)]
pub struct CoverageStats {
    pub stats: std::collections::HashMap<String, f64>,
}

impl CoverageStats {
    pub fn from_engine(engine: &SimulationEngine) -> Self {
        CoverageStats { stats: engine.coverage_stats() }
    }

    pub fn branch_percent(&self) -> f64 {
        self.stats.get("branch_percent").copied().unwrap_or(0.0)
    }
}

/// Pasang CoverageDatabase ke engine dan merge hasilnya.
pub fn attach_coverage_db(
    engine: &mut SimulationEngine,
    path: &str,
) -> Result<CoverageDatabase, String> {
    let mut db = CoverageDatabase::with_path(path);
    db.merge_from_engine(engine);
    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coverage_stats_default() {
        let s = CoverageStats::default();
        assert_eq!(s.branch_percent(), 0.0);
    }
}
