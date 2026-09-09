//! Semantic Interaction Graph — pasangan area semantik yang berinteraksi.
//!
//! Bug serius lahir di PERPOTONGAN fitur (scheduler × NBA × multi-writer),
//! bukan fitur individu. Graph menawarkan interaksi berbobot sesuai tekanan
//! (`SemanticPressure`) — komposer mengambil interaksi, bukan template.

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::Rng;

use crate::semantic::{SemArea, SemanticPressure};

/// Satu edge interaksi: area_a × area_b saling mempengaruhi semantik.
#[derive(Debug, Clone, Copy)]
pub struct Interaction {
    pub a: SemArea,
    pub b: SemArea,
    /// Bobot dasar (seberapa produktif utk menemukan bug di perpotongan ini).
    pub base_weight: u8,
    /// Catatan eksperimental — kenapa interaksi ini berbahaya.
    pub note: &'static str,
}

/// Graph interaksi semantik (P0 scheduler/concurrency/timing dulu — sumber
/// bug simulator: ordering event salah = output masuk akal tapi timing salah).
pub struct InteractionGraph {
    pub edges: Vec<Interaction>,
}

impl InteractionGraph {
    pub fn default() -> Self {
        Self {
            edges: vec![
                // ── P0: scheduler & konkurensi ─────────────────────────────
                Interaction {
                    a: SemArea::Scheduler,
                    b: SemArea::Concurrency,
                    base_weight: 95,
                    note: "NBA + fork/join + delta + writer ganda — race ordering",
                },
                Interaction {
                    a: SemArea::Scheduler,
                    b: SemArea::Timing,
                    base_weight: 90,
                    note: "NBA vs #0 vs event-control — region ordering salah",
                },
                Interaction {
                    a: SemArea::Scheduler,
                    b: SemArea::Xz,
                    base_weight: 80,
                    note: "X/Z di delta-cycle — propagasi zero-time",
                },
                Interaction {
                    a: SemArea::Scheduler,
                    b: SemArea::Assertion,
                    base_weight: 75,
                    note: "SVA observed/reactive region vs NBA update",
                },
                // ── P0: ekspresi, lebar, tanda ─────────────────────────────
                Interaction {
                    a: SemArea::Width,
                    b: SemArea::Signedness,
                    base_weight: 90,
                    note: "operand sizing §11.6 + ekstensi tanda — hasil salah-konsisten",
                },
                Interaction {
                    a: SemArea::Width,
                    b: SemArea::TypeSystem,
                    base_weight: 80,
                    note: "cast/typedef vs lebar — truncation/extension",
                },
                Interaction {
                    a: SemArea::Signedness,
                    b: SemArea::TypeSystem,
                    base_weight: 70,
                    note: "signed unsigned cast — perbandingan tanda",
                },
                // ── P0: elaborasi / hirarki ────────────────────────────────
                Interaction {
                    a: SemArea::Elaboration,
                    b: SemArea::Hierarchy,
                    base_weight: 85,
                    note: "param override + hier ref + port type binding",
                },
                Interaction {
                    a: SemArea::Generate,
                    b: SemArea::Hierarchy,
                    base_weight: 80,
                    note: "generate loop + hier signal — naming/scope",
                },
                Interaction {
                    a: SemArea::Package,
                    b: SemArea::Elaboration,
                    base_weight: 70,
                    note: "import scope vs param/typedef resolution",
                },
                // ── P1: lifetime & fungsi ──────────────────────────────────
                Interaction {
                    a: SemArea::Lifetime,
                    b: SemArea::Concurrency,
                    base_weight: 75,
                    note: "automatic/static di fork — state bocor antar branch",
                },
                Interaction {
                    a: SemArea::Lifetime,
                    b: SemArea::Scheduler,
                    base_weight: 70,
                    note: "function call didalam #0/NBA — stack/region",
                },
                // ── P1: OOP ────────────────────────────────────────────────
                Interaction {
                    a: SemArea::Class,
                    b: SemArea::Constraint,
                    base_weight: 75,
                    note: "inheritance + constraint — randomize override",
                },
                Interaction {
                    a: SemArea::Class,
                    b: SemArea::Lifetime,
                    base_weight: 60,
                    note: "static class member vs instance — shared state",
                },
                // ── P1: interface & assertion ──────────────────────────────
                Interaction {
                    a: SemArea::Interface,
                    b: SemArea::Scheduler,
                    base_weight: 70,
                    note: "clocking block sync region vs NBA",
                },
                Interaction {
                    a: SemArea::Interface,
                    b: SemArea::Hierarchy,
                    base_weight: 75,
                    note: "virtual interface + modport binding",
                },
                Interaction {
                    a: SemArea::Assertion,
                    b: SemArea::Timing,
                    base_weight: 65,
                    note: "assert di clocking/event — sampled value",
                },
                Interaction {
                    a: SemArea::Coverage,
                    b: SemArea::Scheduler,
                    base_weight: 50,
                    note: "covergroup sampling vs delta",
                },
            ],
        }
    }

    /// Pilih interaksi berbobot tekanan: skor = base × (pressure a + pressure
    /// b)/2 + base. Area tekanan tinggi → edge-nya lebih sering terpilih.
    pub fn sample(&self, rng: &mut StdRng, pressure: &SemanticPressure) -> Interaction {
        let mut total: u32 = 0;
        let mut scores: Vec<u32> = Vec::with_capacity(self.edges.len());
        for e in &self.edges {
            let pa = (pressure.get(e.a) as u32).min(100);
            let pb = (pressure.get(e.b) as u32).min(100);
            let score = (e.base_weight as u32)
                .saturating_mul(1 + (pa + pb) / 2)
                + e.base_weight as u32;
            scores.push(score);
            total += score;
        }
        if total == 0 {
            return self.edges[0];
        }
        let mut pick = rng.gen_range(0..total);
        for (i, s) in scores.iter().enumerate() {
            if pick < *s {
                return self.edges[i];
            }
            pick -= s;
        }
        self.edges[self.edges.len() - 1]
    }

    /// Edge tak pernah menghasilkan mismatch (pertimbangkan penurunan bobot
    /// di masa depan) — placeholder utk adaptasi berbasis umpan balik.
    pub fn edges_for(&self, a: SemArea) -> Vec<&Interaction> {
        self.edges.iter().filter(|e| e.a == a || e.b == a).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn sample_pressure_bias() {
        let mut rng = StdRng::seed_from_u64(1);
        let g = InteractionGraph::default();
        // Tekanan scheduler tinggi — sample harus sering memilih edge scheduler.
        let p = crate::semantic::SemanticPressure::scheduler_directed();
        let mut picked_sched = 0;
        let n = 200;
        for _ in 0..n {
            let e = g.sample(&mut rng, &p);
            if e.a == SemArea::Scheduler || e.b == SemArea::Scheduler {
                picked_sched += 1;
            }
        }
        assert!(
            picked_sched > n / 2,
            "tekanan scheduler harus mem-bias pemilihan edge ({}/{})",
            picked_sched,
            n
        );
    }

    #[test]
    fn edges_cover_p0() {
        let g = InteractionGraph::default();
        assert!(
            g.edges
                .iter()
                .any(|e| (e.a == SemArea::Scheduler && e.b == SemArea::Concurrency)
                    || (e.a == SemArea::Concurrency && e.b == SemArea::Scheduler)),
            "scheduler×concurrency harus ada (P0)"
        );
    }
}