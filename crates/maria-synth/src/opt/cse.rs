//! Pass `Cse` — Common Subexpression Elimination (SYNTHESIS.md §6).
//!
//! Dua node identik (kind + operand ter-resolve) → hasil kedua di-alias ke
//! hasil pertama. Operand di-resolve: konstanta memakai (width, value) — dua
//! konstanta senilai dari slot berbeda tetap dianggap sama.

use std::collections::HashMap;

use maria_sir::{SirModule, SirNodeKind, SirValue, ValueId};

use crate::opt::{alias_content, u64_known};
use crate::pass::{SynthContext, SynthPass};

/// Pass: eliminasi sub-ekspresi identik.
pub struct Cse;

impl SynthPass for Cse {
    fn name(&self) -> &'static str {
        "cse"
    }

    fn run(&mut self, ctx: &mut SynthContext) -> Result<usize, maria_core::error::SimError> {
        let mut seen: HashMap<String, ValueId> = HashMap::new();
        let mut changed = 0usize;
        let nids: Vec<usize> = (0..ctx.module.nodes.len()).collect();
        for nid in nids {
            if !crate::opt::node_alive(&ctx.module, nid) {
                continue;
            }
            let key = node_key(&ctx.module, nid);
            let out = ctx.module.nodes[nid].output;
            match seen.get(&key) {
                Some(&first) => {
                    ctx.module.values[out] = alias_content(&ctx.module, first);
                    changed += 1;
                }
                None => {
                    seen.insert(key, out);
                }
            }
        }
        Ok(changed)
    }
}

/// Kunci struktural node dengan operand ter-resolve.
fn node_key(m: &SirModule, nid: usize) -> String {
    let n = &m.nodes[nid];
    let inputs: Vec<String> = n.inputs.iter().map(|v| value_key(m, *v)).collect();
    format!("{}|{}|{}", kind_key(&n.kind), n.width, inputs.join(","))
}

fn kind_key(k: &SirNodeKind) -> String {
    format!("{:?}", k)
}

/// Resolusi operand untuk key: konstanta dinormalisasi (width,value).
fn value_key(m: &SirModule, vid: ValueId) -> String {
    match &m.values[vid] {
        SirValue::Const(lv) => match u64_known(lv) {
            Some(u) => format!("c{}:{:x}", lv.width, u),
            None => format!("cx{}:{:?}", lv.width, lv.bits),
        },
        SirValue::Port(p) => format!("p{p}"),
        SirValue::Reg(r) => format!("r{r}"),
        SirValue::Node(n) => format!("n{n}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pass::SynthPipeline;
    use maria_core::intern::Symbol;
    use maria_core::LogicVec;
    use maria_sir::{SirModule, SirNodeKind};

    #[test]
    fn identical_nodes_are_deduped() {
        let mut m = SirModule::new(Symbol::intern("top"));
        m.add_value(SirValue::Port(0)); // a
        m.add_value(SirValue::Port(1)); // b
        let n1 = m.add_node(SirNodeKind::And, vec![0, 1], 8);
        let o1 = m.nodes[n1].output;
        m.add_value(SirValue::Node(n1)); // vid 2
        let n2 = m.add_node(SirNodeKind::And, vec![0, 1], 8);
        let o2 = m.nodes[n2].output;
        m.add_value(SirValue::Node(n2)); // vid 3
        let mut p = SynthPipeline::new();
        p.add(Cse);
        let (m2, results) = p.run(m).unwrap();
        assert_eq!(results[0].changed, 1);
        assert_eq!(m2.values[o2], m2.values[o1], "y harus alias x (a&b dedup)");
    }

    #[test]
    fn equivalent_consts_in_different_slots_dedup() {
        // And(c1, x) dengan c1 di slot berbeda tapi senilai → dedup.
        let mut m = SirModule::new(Symbol::intern("top"));
        m.add_value(SirValue::Port(0)); // x
        m.add_value(SirValue::Const(LogicVec::from_u64(0xAA, 8))); // c1
        m.add_value(SirValue::Const(LogicVec::from_u64(0xAA, 8))); // c2 (slot beda)
        let n1 = m.add_node(SirNodeKind::Or, vec![0, 1], 8);
        let o1 = m.nodes[n1].output;
        m.add_value(SirValue::Node(n1));
        let n2 = m.add_node(SirNodeKind::Or, vec![0, 2], 8);
        let o2 = m.nodes[n2].output;
        m.add_value(SirValue::Node(n2));
        let mut p = SynthPipeline::new();
        p.add(Cse);
        let (m2, results) = p.run(m).unwrap();
        assert_eq!(results[0].changed, 1);
        assert_eq!(m2.values[o2], m2.values[o1]);
    }
}
