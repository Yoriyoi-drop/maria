//! Model semantik SystemVerilog (IEEE 1800-2017) — area semantic + tekanan.
//!
//! Filosofi (non-template-centric): generator tidak lagi "buat template
//! always", tapi "buat kombinasi semantic yang menekan area X∩Y". Tiap area
//! punya bobot tekanan (`SemanticPressure`) — seed yang lahir = eksperimen
//! semantic, bukan template ke-N.

use std::collections::HashMap;

/// Area semantik IEEE 1800 — target fuzzing (bukan konstruk sintaks individu).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SemArea {
    /// Event regions (active/inactive/NBA/observed/reactive/postponed),
    /// delta-cycle, `#0`, NBA-vs-blocking ordering.
    Scheduler,
    /// Tipe data & casting.
    TypeSystem,
    /// Lebar & ekstensi operan (operand sizing §11.6/11.8).
    Width,
    /// Signed/unsigned, ekstensi tanda.
    Signedness,
    /// Lifetime static/automatic (function/task/class member).
    Lifetime,
    /// Elaborasi: param, generate-instantiation, scope import.
    Elaboration,
    /// Hierarki: referensi lintas modul/instance, port binding.
    Hierarchy,
    /// Timing: delay, event control, edge, clocking block sync.
    Timing,
    /// Propagasi X/Z, unresolved, 4-state.
    Xz,
    /// Konkurensi: fork/join, beberapa proses, race, writer ganda.
    Concurrency,
    /// SVA: property/sequence, observed/reactive region.
    Assertion,
    /// Constraint/randomize.
    Constraint,
    /// Covergroup/coverpoint/cross.
    Coverage,
    /// OOP: class/inheritance/virtual/override.
    Class,
    /// Interface/modport/clocking block.
    Interface,
    /// Package & import scope.
    Package,
    /// Generate blocks & genvar.
    Generate,
    /// DPI/VPI/PLI.
    Dpi,
}

impl SemArea {
    /// Semua area (urutan tetap — profil & laporan konsisten).
    pub const ALL: [SemArea; 18] = [
        SemArea::Scheduler,
        SemArea::TypeSystem,
        SemArea::Width,
        SemArea::Signedness,
        SemArea::Lifetime,
        SemArea::Elaboration,
        SemArea::Hierarchy,
        SemArea::Timing,
        SemArea::Xz,
        SemArea::Concurrency,
        SemArea::Assertion,
        SemArea::Constraint,
        SemArea::Coverage,
        SemArea::Class,
        SemArea::Interface,
        SemArea::Package,
        SemArea::Generate,
        SemArea::Dpi,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            SemArea::Scheduler => "scheduler",
            SemArea::TypeSystem => "type_system",
            SemArea::Width => "width",
            SemArea::Signedness => "signedness",
            SemArea::Lifetime => "lifetime",
            SemArea::Elaboration => "elaboration",
            SemArea::Hierarchy => "hierarchy",
            SemArea::Timing => "timing",
            SemArea::Xz => "xz",
            SemArea::Concurrency => "concurrency",
            SemArea::Assertion => "assertion",
            SemArea::Constraint => "constraint",
            SemArea::Coverage => "coverage",
            SemArea::Class => "class",
            SemArea::Interface => "interface",
            SemArea::Package => "package",
            SemArea::Generate => "generate",
            SemArea::Dpi => "dpi",
        }
    }

    pub fn from_name(s: &str) -> Option<SemArea> {
        SemArea::ALL.iter().find(|a| a.name() == s).copied()
    }
}

/// Tekanan semantik — seberapa keras seed menekan tiap area (0..=100).
/// Bukan pilihan template acak: area berbobot tinggi memandu komposisi.
#[derive(Debug, Clone, Default)]
pub struct SemanticPressure {
    weights: HashMap<SemArea, u8>,
}

impl SemanticPressure {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set bobot satu area.
    pub fn set(&mut self, area: SemArea, w: u8) {
        self.weights.insert(area, w.min(100));
    }

    pub fn get(&self, area: SemArea) -> u8 {
        self.weights.get(&area).copied().unwrap_or(0)
    }

    /// Area dengan bobot >= ambang (target aktif utk sesi ini).
    pub fn hot_areas(&self, threshold: u8) -> Vec<SemArea> {
        SemArea::ALL
            .iter()
            .copied()
            .filter(|a| self.get(*a) >= threshold)
            .collect()
    }

    /// Profil awal tekan scheduler/concurrency/timing (P0) — sumber bug
    /// simulator yang paling berbahaya (ordering event salah = output
    /// masuk akal tapi salah timing).
    pub fn scheduler_directed() -> Self {
        let mut p = Self::new();
        p.set(SemArea::Scheduler, 95);
        p.set(SemArea::Concurrency, 90);
        p.set(SemArea::Timing, 80);
        p.set(SemArea::Width, 60);
        p.set(SemArea::Signedness, 60);
        p.set(SemArea::Lifetime, 50);
        p
    }

    /// Profil hirarki/elaborasi — generate, scope, port binding.
    pub fn hierarchy_directed() -> Self {
        let mut p = Self::new();
        p.set(SemArea::Elaboration, 90);
        p.set(SemArea::Hierarchy, 90);
        p.set(SemArea::Generate, 80);
        p.set(SemArea::Package, 60);
        p.set(SemArea::Interface, 60);
        p
    }

    /// Parse profil teks `"scheduler=90,concurrency=95"` (CLI/env/dev).
    pub fn from_profile(s: &str) -> Option<Self> {
        let mut p = Self::new();
        for part in s.split([',', ';']) {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let Some((name, val)) = part.split_once('=') else {
                return None;
            };
            let area = SemArea::from_name(name.trim())?;
            let w: u8 = val.trim().parse().ok()?;
            p.set(area, w);
        }
        Some(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_set_get_and_hot() {
        let mut p = SemanticPressure::new();
        p.set(SemArea::Scheduler, 95);
        p.set(SemArea::Width, 30);
        let hot = p.hot_areas(50);
        assert!(hot.contains(&SemArea::Scheduler));
        assert!(!hot.contains(&SemArea::Width));
    }

    #[test]
    fn profile_parse_roundtrip() {
        let p = SemanticPressure::from_profile("scheduler=90, concurrency=95").unwrap();
        assert_eq!(p.get(SemArea::Scheduler), 90);
        assert_eq!(p.get(SemArea::Concurrency), 95);
        assert!(SemanticPressure::from_profile("bogus=1").is_none());
    }

    #[test]
    fn scheduler_directed_targets_p0() {
        let p = SemanticPressure::scheduler_directed();
        assert!(p.hot_areas(70).contains(&SemArea::Scheduler));
        assert!(p.hot_areas(70).contains(&SemArea::Concurrency));
    }
}