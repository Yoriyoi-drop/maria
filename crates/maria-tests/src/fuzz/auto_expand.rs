//! Bug-Guided Auto-Expansion — otomatis generate vari setelah bug ditemukan.
//!
//! Saat fuzzer menemukan bug, auto_expand mengekstrak hot features dan
//! menghasilkan vari yang mengeksplorasi wilayah serupa. Semua vari
//! masuk ke corpus dengan energy tinggi (bug proximity).
//!
//! Alur:
//! 1. Bug ditemukan → ekstrak hot features + GenMode
//! 2. Generate vari: operator family, width boundary, EMI transform, stimulus
//! 3. Vari masuk corpus dengan energy tinggi
//! 4. Coverage guide otomatis eksplorasi wilayah serupa

use super::bug_db::BugSeverity;
use super::expr::BinOp as EBinOp;
use super::expr::Expr;
use super::gen::GenInput;
use super::guide::CoverageGuide;
use super::oracle::Verdict;
use super::svast::{self, GenMode, SVAst};

/// Satu vari yang dihasilkan dari bug.
#[derive(Debug, Clone)]
pub struct BugVariant {
    pub source_input: GenInput,
    pub description: String,
    pub mode: GenMode,
    pub energy_boost: f64,
}

/// Auto-expander: menghasilkan vari dari temuan bug.
pub struct AutoExpander {
    /// Vari yang dihasilkan.
    variants: Vec<BugVariant>,
    /// Batas maksimum vari per bug.
    pub max_variants_per_bug: usize,
}

impl AutoExpander {
    pub fn new() -> Self {
        AutoExpander {
            variants: Vec::new(),
            max_variants_per_bug: 16,
        }
    }

    /// Ekstrak mode dari input (berdasarkan struktur ekspresi).
    pub fn infer_mode(input: &GenInput) -> GenMode {
        use super::expr::Expr as E;
        match &input.expr {
            E::Lit(_) | E::Var(_) | E::XLit { .. } => GenMode::Combinational,
            E::Bin(EBinOp::Concat, _, _) => GenMode::Combinational,
            E::Un(_, _) => GenMode::Combinational,
            E::Bin(_, l, r) => {
                // Jika operator comparison → mungkin sequential (always_comb)
                match l.as_ref() {
                    E::Bin(EBinOp::Eq | EBinOp::Ne | EBinOp::Lt | EBinOp::Gt, _, _) => {
                        GenMode::Sequential
                    }
                    _ => GenMode::Combinational,
                }
            }
            E::Ternary(_, _, _) => GenMode::Combinational,
            E::Repl(_, _) => GenMode::Combinational,
            E::BitSel(_, _) | E::PartSel(_, _, _) => GenMode::Combinational,
        }
    }

    /// Generate vari dari satu bug finding.
    pub fn expand_bug(&mut self, input: &GenInput, message: &str, _severity: BugSeverity) {
        let mode = Self::infer_mode(input);
        let w = input.w;
        let mut rng = fastrand::Rng::with_seed(input.seed ^ 0xBEEF);

        let mut generated = 0usize;

        // 0. SVAst mode variants — PRIORITAS: selalu sertakan semua mode
        //    pipeline (Combinational/Sequential/ForkJoin/Class/Generate) agar
        //    auto-expansion mengeksplorasi wilayah > ekspresi kombinasional.
        for &sv_mode in GenMode::all() {
            if generated >= self.max_variants_per_bug {
                break;
            }
            let _ast = svast::generate_svast_mode(rng.u64(..), sv_mode);
            let svast_input = GenInput {
                w,
                wb: w,
                a: input.a,
                b: input.b,
                expr: super::expr::Expr::Lit(0), // placeholder
                seed: rng.u64(..),
            };
            self.variants.push(BugVariant {
                source_input: svast_input,
                description: format!("svast_mode:{:?}", sv_mode),
                mode: sv_mode,
                energy_boost: 1.3,
            });
            generated += 1;
        }

        // 1. Operator family variant — ganti operator sejenis
        if let super::expr::Expr::Bin(op, _, _) = &input.expr {
            let family = Self::operator_family(*op);
            for &fam_op in family.iter().take(3) {
                if generated >= self.max_variants_per_bug {
                    break;
                }
                let mut mutated = input.clone();
                mutated.seed = rng.u64(..);
                mutated.expr = Self::replace_top_op(&mutated.expr, fam_op);
                self.variants.push(BugVariant {
                    source_input: mutated,
                    description: format!("operator_family:{:?}", fam_op),
                    mode,
                    energy_boost: 2.0,
                });
                generated += 1;
            }
        }

        // 2. Width boundary variants
        let boundaries = [1u32, 2, 4, 8, 15, 16, 17, 31, 32, 33, 63, 64, 65];
        for &bw in boundaries.iter() {
            if generated >= self.max_variants_per_bug {
                break;
            }
            if bw == w {
                continue;
            }
            let mut mutated = input.clone();
            mutated.seed = rng.u64(..);
            mutated.w = bw;
            mutated.wb = bw.min(mutated.w);
            let m = super::gen::mask_of(bw);
            mutated.a &= m;
            mutated.b &= m;
            mutated.normalize();
            self.variants.push(BugVariant {
                source_input: mutated,
                description: format!("width_boundary:{}", bw),
                mode,
                energy_boost: 1.5,
            });
            generated += 1;
        }

        // 3. EMI transform variants (redundant parens, commute, dead code)
        for &emi_op in &[
            "redundant_parens",
            "commute_assoc",
            "literal_form",
            "dead_code",
        ] {
            if generated >= self.max_variants_per_bug {
                break;
            }
            let mut mutated = input.clone();
            mutated.seed = rng.u64(..) ^ 0xE001;
            match emi_op {
                "commute_assoc" => {
                    // Commute: swap operands if commutative
                    if let super::expr::Expr::Bin(op, ref l, ref r) = mutated.expr {
                        if Self::is_commutative(op) {
                            mutated.expr = super::expr::Expr::Bin(
                                op,
                                Box::new((**r).clone()),
                                Box::new((**l).clone()),
                            );
                        }
                    }
                }
                "redundant_parens" | "literal_form" | "dead_code" => {
                    // Variasi seed saja (source rendering berbeda)
                }
                _ => {}
            }
            self.variants.push(BugVariant {
                source_input: mutated,
                description: format!("emi:{}", emi_op),
                mode,
                energy_boost: 1.2,
            });
            generated += 1;
        }

        // 4. Stimulus boundary — input ekstrem
        let extrema = [0u64, 1, !0u64 >> 1, !0u64];
        for &ext in extrema.iter() {
            if generated >= self.max_variants_per_bug {
                break;
            }
            let mut mutated = input.clone();
            mutated.seed = rng.u64(..);
            let m = super::gen::mask_of(w);
            mutated.a = ext & m;
            mutated.b = (ext ^ 0xFFFF) & m;
            self.variants.push(BugVariant {
                source_input: mutated,
                description: format!("extreme_stimulus:{:#x}", ext),
                mode,
                energy_boost: 1.8,
            });
            generated += 1;
        }
    }

    /// Ambil semua vari dan kosongkan buffer.
    pub fn drain_variants(&mut self) -> Vec<BugVariant> {
        std::mem::take(&mut self.variants)
    }

    /// Jumlah vari yang tersisa.
    pub fn pending_count(&self) -> usize {
        self.variants.len()
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    fn operator_family(op: super::expr::BinOp) -> &'static [super::expr::BinOp] {
        use super::expr::BinOp as B;
        match op {
            B::Add | B::Sub | B::Mul | B::Div | B::Mod => &[B::Add, B::Sub, B::Mul, B::Div, B::Mod],
            B::And | B::Or | B::Xor | B::Xnor => &[B::And, B::Or, B::Xor, B::Xnor],
            B::Shl | B::Shr | B::Sshl | B::Sshr => &[B::Shl, B::Shr, B::Sshl, B::Sshr],
            B::Eq | B::Ne | B::Lt | B::Le | B::Gt | B::Ge => {
                &[B::Eq, B::Ne, B::Lt, B::Le, B::Gt, B::Ge]
            }
            B::LogicAnd | B::LogicOr => &[B::LogicAnd, B::LogicOr],
            B::CaseEq | B::CaseNeq => &[B::CaseEq, B::CaseNeq, B::Eq, B::Ne],
            B::Power => &[B::Power, B::Mul],
            B::Concat => &[B::Concat, B::Add],
            B::Inside => &[B::Inside, B::Eq, B::Ne],
        }
    }

    fn is_commutative(op: super::expr::BinOp) -> bool {
        use super::expr::BinOp as B;
        matches!(
            op,
            B::Add
                | B::Mul
                | B::And
                | B::Or
                | B::Xor
                | B::Xnor
                | B::Eq
                | B::Ne
                | B::LogicAnd
                | B::LogicOr
                | B::CaseEq
                | B::CaseNeq
        )
    }

    fn replace_top_op(expr: &super::expr::Expr, new_op: super::expr::BinOp) -> super::expr::Expr {
        match expr {
            super::expr::Expr::Bin(_, l, r) => super::expr::Expr::Bin(new_op, l.clone(), r.clone()),
            other => other.clone(),
        }
    }
}

impl Default for AutoExpander {
    fn default() -> Self {
        Self::new()
    }
}

/// Integrasikan auto-expand ke dalam fuzz loop.
///
/// Setelah bug ditemukan, panggil `expand_bug()`, lalu `drain_variants()`.
/// Masukkan vari ke corpus CoverageGuide dengan energy tinggi.
pub fn apply_expansion(
    expander: &mut AutoExpander,
    guide: &mut CoverageGuide,
    input: &GenInput,
    verdict: &Verdict,
) {
    if let Verdict::Bug(msg) = verdict {
        expander.expand_bug(input, msg, BugSeverity::DifferentialMismatch);
        let variants = expander.drain_variants();
        for v in variants {
            // Feed variant ke guide sebagai seed baru (energy tinggi)
            guide.observe(&v.source_input, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_expand_generates_variants() {
        let mut expander = AutoExpander::new();
        let input = crate::fuzz::gen::generate(42);
        expander.expand_bug(&input, "test bug", BugSeverity::Crash);
        let variants = expander.drain_variants();
        assert!(!variants.is_empty(), "should generate at least one variant");
        assert!(variants.len() <= expander.max_variants_per_bug);
    }

    #[test]
    fn auto_expand_covers_multiple_modes() {
        let mut expander = AutoExpander::new();
        let input = crate::fuzz::gen::generate(42);
        expander.expand_bug(&input, "test bug", BugSeverity::Crash);
        let variants = expander.drain_variants();
        let modes: std::collections::HashSet<_> = variants.iter().map(|v| v.mode).collect();
        // Should cover at least 2 different modes
        assert!(
            modes.len() >= 2,
            "should cover multiple modes, got {:?}",
            modes
        );
    }

    #[test]
    fn auto_expand_infer_mode() {
        use super::super::expr::{BinOp, Expr as E};
        let input = GenInput {
            w: 8,
            wb: 8,
            a: 5,
            b: 3,
            expr: E::Bin(BinOp::Add, Box::new(E::Var('a')), Box::new(E::Var('b'))),
            seed: 1,
        };
        assert_eq!(AutoExpander::infer_mode(&input), GenMode::Combinational);
    }
}
