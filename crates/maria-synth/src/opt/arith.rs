//! Pass `ArithmeticSimplify` — aritmetika (SYNTHESIS.md §6).
//!
//! Identity: `a + 0 → a`, `a − 0 → a`, `a * 1 → a`, `a << 0 → a`, dst.
//! Strength reduction: `a * 2^k → a << k` (satu node, tanpa multiplier).

use maria_core::LogicVec;
use maria_sir::{SirModule, SirNodeKind, SirValue, ValueId};

use crate::opt::{alias_content, resolve_const, u64_known};
use crate::pass::{SynthContext, SynthPass};

/// Pass: penyederhanaan aritmetika.
pub struct ArithmeticSimplify;

impl SynthPass for ArithmeticSimplify {
    fn name(&self) -> &'static str {
        "arith"
    }

    fn run(&mut self, ctx: &mut SynthContext) -> Result<usize, maria_core::error::SimError> {
        let mut changed = 0usize;
        let nids: Vec<usize> = (0..ctx.module.nodes.len()).collect();
        for nid in nids {
            if !crate::opt::node_alive(&ctx.module, nid) {
                continue;
            }
            let kind = ctx.module.nodes[nid].kind.clone();
            let out = ctx.module.nodes[nid].output;
            if let Some(content) = arith_identity(&mut ctx.module, nid, &kind) {
                ctx.module.values[out] = content;
                changed += 1;
            }
        }
        Ok(changed)
    }
}

fn arith_identity(m: &mut SirModule, nid: usize, kind: &SirNodeKind) -> Option<SirValue> {
    let inputs = m.nodes[nid].inputs.clone();
    let width = m.nodes[nid].width;
    match kind {
        SirNodeKind::Add | SirNodeKind::Sub => {
            // a + 0 → a ; a - 0 → a (0 di salah satu operand)
            for (i, v) in inputs.iter().enumerate() {
                if let Some(c) = const_of(m, *v) {
                    if c == 0 {
                        let other = inputs[1 - i];
                        return Some(alias_content(m, other));
                    }
                }
            }
            // a - a → 0 ; a + a → a<<1 (dibiarkan — phase 2 minimal)
            if kind == &SirNodeKind::Sub && inputs.len() == 2 && inputs[0] == inputs[1] {
                return Some(SirValue::Const(LogicVec::from_u64(0, width.max(1))));
            }
            None
        }
        SirNodeKind::Mul => {
            for (i, v) in inputs.iter().enumerate() {
                if let Some(c) = const_of(m, *v) {
                    let other = inputs[1 - i];
                    if c == 0 {
                        return Some(SirValue::Const(LogicVec::from_u64(0, width.max(1))));
                    }
                    if c == 1 {
                        return Some(alias_content(m, other));
                    }
                    // Strength reduction: a * 2^k → a << k
                    if let Some(k) = pow2_shift(c) {
                        let kv = m.add_value(SirValue::Const(LogicVec::from_u64(k, width.max(1))));
                        m.nodes[nid].kind = SirNodeKind::Shl;
                        m.nodes[nid].inputs = vec![other, kv];
                        return Some(SirValue::Node(nid));
                    }
                }
            }
            None
        }
        SirNodeKind::Shl | SirNodeKind::Shr | SirNodeKind::Sar => {
            // a << 0 → a ; a >> 0 → a
            if inputs.len() == 2 {
                if let Some(c) = const_of(m, inputs[1]) {
                    if c == 0 {
                        return Some(alias_content(m, inputs[0]));
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn const_of(m: &SirModule, v: ValueId) -> Option<u64> {
    resolve_const(m, v).and_then(|lv| u64_known(&lv))
}

/// Bila `c` pangkat dua → eksponen k (c == 2^k).
fn pow2_shift(c: u64) -> Option<u64> {
    if c != 0 && c.is_power_of_two() {
        Some(c.trailing_zeros() as u64)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maria_core::intern::Symbol;
    use maria_sir::SirModule;
    use crate::pass::SynthPipeline;

    #[test]
    fn mul_by_power_of_two_becomes_shift() {
        let mut m = SirModule::new(Symbol::intern("top"));
        m.add_value(SirValue::Port(0)); // x
        m.add_value(SirValue::Const(LogicVec::from_u64(4, 8))); // 4
        let nid = m.add_node(SirNodeKind::Mul, vec![0, 1], 8);
        let out = m.nodes[nid].output;
        m.add_value(SirValue::Node(nid));
        let mut p = SynthPipeline::new();
        p.add(ArithmeticSimplify);
        let (m2, results) = p.run(m).unwrap();
        assert_eq!(results[0].changed, 1);
        let node = &m2.nodes[nid];
        assert_eq!(node.kind, SirNodeKind::Shl, "a*4 → a<<2");
        assert_eq!(node.inputs[0], 0);
        assert_eq!(m2.values[out], SirValue::Node(nid), "output tetap node (Shl)");
    }

    #[test]
    fn add_zero_aliases() {
        let mut m = SirModule::new(Symbol::intern("top"));
        m.add_value(SirValue::Port(0)); // x
        m.add_value(SirValue::Const(LogicVec::from_u64(0, 8)));
        let nid = m.add_node(SirNodeKind::Add, vec![0, 1], 8);
        let out = m.nodes[nid].output;
        m.add_value(SirValue::Node(nid));
        let mut p = SynthPipeline::new();
        p.add(ArithmeticSimplify);
        let (m2, _) = p.run(m).unwrap();
        assert_eq!(m2.values[out], m2.values[0], "a+0 → a");
    }
}
