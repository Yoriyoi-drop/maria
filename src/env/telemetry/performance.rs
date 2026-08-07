//! Performance — pencatat timing per fase + laporan bottleneck.

/// Satu pengukuran fase.
#[derive(Debug, Clone)]
pub struct PhaseTiming {
    pub name: &'static str,
    pub ms: f64,
}

/// Kumpulan timing per fase (diurutkan descending saat laporan).
#[derive(Debug, Default)]
pub struct PhaseTimings {
    pub phases: Vec<PhaseTiming>,
}

impl PhaseTimings {
    pub fn record(&mut self, name: &'static str, ms: f64) {
        if let Some(p) = self.phases.iter_mut().find(|p| p.name == name) {
            p.ms += ms;
        } else {
            self.phases.push(PhaseTiming { name, ms });
        }
    }

    pub fn total_ms(&self) -> f64 {
        self.phases.iter().map(|p| p.ms).sum()
    }

    /// Fase paling lama (bottleneck).
    pub fn bottleneck(&self) -> Option<&PhaseTiming> {
        self.phases.iter().max_by(|a, b| a.ms.total_cmp(&b.ms))
    }

    /// Laporan diurutkan dari paling lambat.
    pub fn sorted(&self) -> Vec<&PhaseTiming> {
        let mut v: Vec<&PhaseTiming> = self.phases.iter().collect();
        v.sort_by(|a, b| b.ms.total_cmp(&a.ms));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_phase_timings() {
        let mut t = PhaseTimings::default();
        t.record("parser", 311.0);
        t.record("lexer", 23.0);
        t.record("elaboration", 721.0);
        assert_eq!(t.total_ms(), 1055.0);
        assert_eq!(t.bottleneck().unwrap().name, "elaboration");
        assert_eq!(t.sorted()[0].name, "elaboration");
    }

    #[test]
    fn test_record_accumulates() {
        let mut t = PhaseTimings::default();
        t.record("parser", 10.0);
        t.record("parser", 5.0);
        assert_eq!(t.phases.len(), 1);
        assert_eq!(t.phases[0].ms, 15.0);
    }
}
