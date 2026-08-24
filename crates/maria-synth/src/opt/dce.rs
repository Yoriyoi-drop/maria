//! Pass `Dce` — Dead Code Elimination (SYNTHESIS.md §6).
//!
//! Node yang tidak lagi direferensikan dari "akar" (port, register, wire)
//! dihapus. Node yang output-nya sudah diganti konstanta/alias oleh pass
//! sebelumnya (fold/identity/cse) otomatis mati di sini.

use std::collections::HashMap;

use maria_core::LogicVec;
use maria_sir::{SirValue, ValueId};

use crate::pass::{SynthContext, SynthPass};

/// Pass: buang node mati + remap index node.
pub struct Dce;

impl SynthPass for Dce {
    fn name(&self) -> &'static str {
        "dce"
    }

    fn run(&mut self, ctx: &mut SynthContext) -> Result<usize, maria_core::error::SimError> {
        let m = &mut ctx.module;
        let n = m.nodes.len();
        if n == 0 {
            return Ok(0);
        }

        // Akar: semua nilai yang dirujuk langsung oleh port/register/wire.
        let mut roots: Vec<ValueId> = Vec::new();
        for p in m.inputs.iter().chain(m.outputs.iter()) {
            roots.push(p.value);
        }
        for w in &m.wires {
            roots.push(w.value);
        }
        for r in &m.registers {
            roots.push(r.d);
            roots.push(r.q);
            roots.push(r.clock);
            if let Some(rs) = &r.reset {
                roots.push(rs.signal);
            }
            if let Some(e) = r.enable {
                roots.push(e);
            }
        }

        // BFS reachability.
        let mut alive = vec![false; n];
        let mut stack = roots;
        while let Some(vid) = stack.pop() {
            if let Some(SirValue::Node(nid)) = m.values.get(vid) {
                if *nid < n && !alive[*nid] {
                    alive[*nid] = true;
                    stack.extend(m.nodes[*nid].inputs.iter().copied());
                }
            }
        }

        let removed = n - alive.iter().filter(|a| **a).count();
        if removed == 0 {
            return Ok(0);
        }

        // Remap node hidup.
        let mut remap: HashMap<usize, usize> = HashMap::new();
        let mut kept: Vec<usize> = Vec::new();
        for (old, a) in alive.iter().enumerate() {
            if *a {
                remap.insert(old, kept.len());
                kept.push(old);
            }
        }
        let new_nodes: Vec<_> = kept.iter().map(|old| m.nodes[*old].clone()).collect();
        m.nodes = new_nodes;

        // Rewrite referensi nilai: Node(old) → Node(new) / Const(0) bila mati.
        for v in m.values.iter_mut() {
            if let SirValue::Node(old) = v {
                if let Some(nid) = remap.get(old) {
                    *v = SirValue::Node(*nid);
                } else {
                    *v = SirValue::Const(LogicVec::from_u64(0, 1));
                }
            }
        }

        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pass::SynthPipeline;
    use maria_core::intern::Symbol;
    use maria_sir::{SirModule, SirNodeKind};

    #[test]
    fn orphan_node_is_removed() {
        let mut m = SirModule::new(Symbol::intern("top"));
        m.add_value(SirValue::Port(0)); // 0 — port (akar)
        let n1 = m.add_node(SirNodeKind::Not, vec![0], 1);
        let o1 = m.nodes[n1].output;
        m.add_value(SirValue::Node(n1)); // vid 1
                                         // Wire merujuk vid 1 → n1 HIDUP (akar).
        m.add_wire(Symbol::intern("y"), 1, 1);
        let n2 = m.add_node(SirNodeKind::Not, vec![0], 1);
        let _o2 = m.nodes[n2].output;
        m.add_value(SirValue::Node(n2)); // vid 3 — n2 mati (tidak dirujuk)
        let mut p = SynthPipeline::new();
        p.add(Dce);
        let (m2, results) = p.run(m).unwrap();
        assert_eq!(results[0].changed, 1);
        assert_eq!(m2.nodes.len(), 1, "satu node mati dibuang");
        // n1 hidup → output masih Node(remap 0).
        assert_eq!(m2.values[o1], SirValue::Node(0));
    }

    #[test]
    fn dead_node_output_becomes_const() {
        let mut m = SirModule::new(Symbol::intern("top"));
        m.add_value(SirValue::Port(0));
        let nid = m.add_node(SirNodeKind::Not, vec![0], 1);
        let out = m.nodes[nid].output;
        m.add_value(SirValue::Node(nid));
        // Tanpa akar lain, node mati → output jadi Const(0).
        let mut p = SynthPipeline::new();
        p.add(Dce);
        let (m2, _) = p.run(m).unwrap();
        assert_eq!(m2.nodes.len(), 0);
        assert!(matches!(m2.values[out], SirValue::Const(_)));
    }
}
