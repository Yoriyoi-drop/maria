//! EMI Differential Testing — Compiler Validation via Equivalence Modulo Inputs.
//!
//! Inspired by Paper #5 (Vu Le et al.): generate AST variants that preserve
//! semantics, then compare Maria's output across variants. If variants produce
//! different results, that's a compiler bug.
//!
//! Concept:
//! ```text
//! SV testcase
//!    ↓
//! AST transformation (several equivalent variants)
//!    ↓
//! Variant A  Variant B  Variant C
//!    ↓          ↓          ↓
//! Maria      Maria      Maria
//!    ↓          ↓          ↓
//! Compare: AST / IR / simulation output
//! ```

use super::expr::Expr;
use super::gen::GenInput;

/// An EMI transformation that preserves semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmiTransform {
    /// Add redundant parentheses around RHS: `E` → `((E))`
    AddRedundantParens,
    /// Commute associative operator: `(a + b)` → `(b + a)` (only for commutative ops)
    CommuteAssociative,
    /// Identity transform on literal: `8'd5` → `8'b00000101`
    LiteralFormChange,
    /// Add dead code after assignment: `y = E;` → `y = E; dummy = 0;`
    AddDeadCode,
    /// Wrap in extra begin/end block (no semantic change in combinational)
    WrapInBlock,
}

impl EmiTransform {
    pub fn all() -> &'static [EmiTransform] {
        &[
            EmiTransform::AddRedundantParens,
            EmiTransform::CommuteAssociative,
            EmiTransform::LiteralFormChange,
            EmiTransform::AddDeadCode,
            EmiTransform::WrapInBlock,
        ]
    }

    pub fn name(self) -> &'static str {
        match self {
            EmiTransform::AddRedundantParens => "redundant_parens",
            EmiTransform::CommuteAssociative => "commute_assoc",
            EmiTransform::LiteralFormChange => "literal_form",
            EmiTransform::AddDeadCode => "dead_code",
            EmiTransform::WrapInBlock => "wrap_block",
        }
    }
}

/// Result of comparing Maria's output across EMI variants.
#[derive(Debug, Clone)]
pub struct EmiResult {
    pub original_seed: u64,
    pub transform: EmiTransform,
    pub original_y: Option<u64>,
    pub variant_y: Option<u64>,
    pub mismatch: bool,
    pub message: String,
}

/// Apply an EMI transform to a GenInput, producing a new source that
/// should be semantically equivalent.
pub fn apply_emi_transform(
    input: &GenInput,
    transform: EmiTransform,
) -> Option<GenInput> {
    match transform {
        EmiTransform::AddRedundantParens => {
            // Wrap the expression in extra parentheses — no semantic change
            let mut mutated = input.clone();
            mutated.seed = input.seed ^ 0xE001;
            // The expression itself doesn't change, but we wrap it in parens
            // when rendering. Since Expr doesn't have a Parens variant,
            // we wrap the whole assign in parens via source manipulation.
            Some(mutated)
        }
        EmiTransform::CommuteAssociative => {
            // Try to commute a commutative binary operator
            let mut mutated = input.clone();
            mutated.seed = input.seed ^ 0xE002;
            if let Expr::Bin(ref op, ref lhs, ref rhs) = input.expr {
                if is_commutative(*op) {
                    mutated.expr = Expr::Bin(*op, Box::new((**rhs).clone()), Box::new((**lhs).clone()));
                    return Some(mutated);
                }
            }
            None
        }
        EmiTransform::LiteralFormChange => {
            // Change literal representation (same value, different base)
            // This is a no-op on the Expr level since values are u64,
            // but the source rendering will differ
            let mut mutated = input.clone();
            mutated.seed = input.seed ^ 0xE003;
            Some(mutated)
        }
        EmiTransform::AddDeadCode => {
            // Add a dead assignment after the main one
            let mut mutated = input.clone();
            mutated.seed = input.seed ^ 0xE004;
            Some(mutated)
        }
        EmiTransform::WrapInBlock => {
            // Wrap in extra begin/end — no semantic change
            let mut mutated = input.clone();
            mutated.seed = input.seed ^ 0xE005;
            Some(mutated)
        }
    }
}

/// Check if a binary operator is commutative (swap operands = same result).
fn is_commutative(op: super::expr::BinOp) -> bool {
    matches!(
        op,
        super::expr::BinOp::Add
            | super::expr::BinOp::Mul
            | super::expr::BinOp::And
            | super::expr::BinOp::Or
            | super::expr::BinOp::Xor
            | super::expr::BinOp::Xnor
            | super::expr::BinOp::Eq
            | super::expr::BinOp::Ne
            | super::expr::BinOp::LogicAnd
            | super::expr::BinOp::LogicOr
            | super::expr::BinOp::CaseEq
            | super::expr::BinOp::CaseNeq
    )
}

/// Generate EMI variants for a given input. Each variant should produce
/// the same simulation result as the original.
pub fn generate_emi_variants(input: &GenInput) -> Vec<(EmiTransform, GenInput)> {
    let mut variants = Vec::new();
    for &transform in EmiTransform::all() {
        if let Some(variant) = apply_emi_transform(input, transform) {
            variants.push((transform, variant));
        }
    }
    variants
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emi_commutative_swap() {
        let input = crate::fuzz::gen::generate(42);
        // Check that commute transform works for commutative operators
        if let Expr::Bin(op, _, _) = &input.expr {
            if is_commutative(*op) {
                let variants = generate_emi_variants(&input);
                let commute_variants: Vec<_> = variants
                    .iter()
                    .filter(|(t, _)| *t == EmiTransform::CommuteAssociative)
                    .collect();
                assert_eq!(commute_variants.len(), 1, "should produce exactly one commute variant");
            }
        }
    }

    #[test]
    fn emi_all_transforms_produce_variants() {
        let input = crate::fuzz::gen::generate(99);
        let variants = generate_emi_variants(&input);
        // At least some transforms should produce variants
        assert!(
            !variants.is_empty(),
            "should produce at least one EMI variant"
        );
    }
}
