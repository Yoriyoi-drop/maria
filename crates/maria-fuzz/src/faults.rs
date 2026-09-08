//! Fault-injection validation (GAP-5 — paradigma mutation-testing):
//!
//! 1. **Soundness sweep** — jalankan SELURUH baterai oracle (determinism, EMI,
//!    metamorphic-identity) pada N seed generated BERSIH; setiap mismatch =
//!    FALSE POSITIVE oracle (bukan bug engine). Target: 0 FP.
//! 2. **Fault observability** — dua fault semantik klasik disimulasikan di
//!    level source (op-swap `&`→`|`, `+`→`&`): fingerprint source harus beda
//!    dari source ber-fault. Kalau fault tak mengubah fingerprint, fault tak
//!    terobservasi di sinyal top → oracle mana pun tak bisa menangkapnya
//!    (batas fundamental fingerprint top-signals).
//!
//! Hasil = baseline eksperimental (bukan klaim): precision oracle pada input
//! bersih + sensitivity bound.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::differential;
use crate::harness;
use crate::FuzzConfig;
use crate::gen::Generator;

/// Swap kemunculan `from` → `to` pertama (fault semantik deterministik).
pub fn op_swap_fault(source: &str, from: &str, to: &str) -> Option<String> {
    let pos = source.find(from)?;
    let mut s = source.to_string();
    s.replace_range(pos..pos + from.len(), to);
    Some(s)
}

/// Observability: fingerprint source ≠ fingerprint ber-fault → fault benar-
/// benar mengubah perilaku sinyal. Fingerprint standar SUDAH memuat sinyal
/// flatten child module — hierarki tak menyembunyikan fault (terverifikasi).
pub fn fault_observable(source: &str, from: &str, to: &str, cfg: &FuzzConfig) -> bool {
    let Some(faulted) = op_swap_fault(source, from, to) else {
        return false;
    };
    let f1 = harness::fingerprint_isolated(source, cfg.max_time, cfg.hang_ms);
    let f2 = harness::fingerprint_isolated(&faulted, cfg.max_time, cfg.hang_ms);
    matches!((f1, f2), (Some(a), Some(b)) if a != b)
}

/// Ringkasan sweep.
#[derive(Debug, Clone, Default)]
pub struct SweepResult {
    pub seeds_total: u64,
    /// Seed bersih yang berhasil dieksekusi (fingerprint valid).
    pub seeds_clean: u64,
    /// False positive per oracle pada seed bersih.
    pub fp_determinism: u64,
    pub fp_emi: u64,
    pub fp_meta: u64,
    /// Observability op-swap fault: (terlihat, total dicoba).
    pub fault_observable: u64,
    pub fault_total: u64,
}

/// Sweep N modul generated (Paper #14/#15) — baterai oracle utuh wajib
/// bersih; fault op-swap wajib terlihat di fingerprint.
pub fn sweep(n: usize, base: &FuzzConfig) -> SweepResult {
    let mut rng = StdRng::seed_from_u64(0xF00D);
    let gen = Generator::new(0xF00D);
    let mut res = SweepResult::default();
    let cfg = FuzzConfig {
        hang_ms: 3_000,
        ..base.clone()
    };
    for _ in 0..n {
        let src = gen.random_module(&mut rng);
        res.seeds_total += 1;
        // Basa: harus bisa dieksekusi.
        if harness::fingerprint_isolated(&src, cfg.max_time, cfg.hang_ms).is_none() {
            continue;
        }
        res.seeds_clean += 1;
        // Baterai oracle pada seed bersih — Mismatch = false positive.
        if matches!(
            differential::determinism_check(&src, &cfg),
            differential::DiffVerdict::Mismatch(_)
        ) {
            res.fp_determinism += 1;
        }
        if matches!(
            differential::emi_check(&src, &cfg),
            differential::DiffVerdict::Mismatch(_)
        ) {
            res.fp_emi += 1;
        }
        if matches!(
            differential::meta_identity_check(&src, &cfg),
            differential::DiffVerdict::Mismatch(_)
        ) {
            res.fp_meta += 1;
        }
        // Observability fault op-swap (pasangan umum; swap kemunculan pertama).
        for (f, t) in [("&", "|"), ("+", "&"), ("^", "|"), ("-", "+")] {
            if !src.contains(f) {
                continue;
            }
            res.fault_total += 1;
            if fault_observable(&src, f, t, &cfg) {
                res.fault_observable += 1;
            }
        }
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_swap_changes_source() {
        let s = op_swap_fault("assign y = a & b;", "&", "|").unwrap();
        assert!(s.contains("|"));
        assert!(!s.contains("& b"));
    }

    #[test]
    fn op_swap_none_when_absent() {
        assert!(op_swap_fault("assign y = a;", "&", "|").is_none());
    }

    #[test]
    fn flattened_fingerprint_observes_child_internal_fault() {
        // Flatten elaborator mengangkat sinyal child ke top.signals (nama
        // hier `u.t`) → fingerprint standar SUDAH melihat fault internal
        // child. Mengoreksi hipotesis audit "hierarki buta": berlaku utk
        // input yang tak di-drive (X semuanya), bukan utk hierarki.
        let cfg = FuzzConfig {
            max_time: 40,
            hang_ms: 2000,
            ..FuzzConfig::default()
        };
        let src = "module child(input logic [3:0] x);\n  logic [3:0] t;\n  assign t = x & 4'b1010;\nendmodule\nmodule top(input logic [3:0] a);\n  child u(.x(a));\n  initial a = 4'd5;\nendmodule\n";
        let faulted = op_swap_fault(src, "&", "|").expect("harus ada & di child");
        let f_top = harness::fingerprint_isolated(&src, cfg.max_time, cfg.hang_ms);
        let f_top_f = harness::fingerprint_isolated(&faulted, cfg.max_time, cfg.hang_ms);
        assert!(
            f_top.as_deref().map_or(false, |f| f.contains("u.t=")),
            "fingerprint berisi sinyal flatten child: {:?}",
            f_top
        );
        assert_ne!(
            f_top, f_top_f,
            "fault child-internal TERlihat di fingerprint standar (flatten)"
        );
    }
}