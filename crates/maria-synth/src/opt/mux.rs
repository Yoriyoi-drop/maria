//! Pass `MuxSimplify` — penyederhanaan MUX (SYNTHESIS.md §6).
//!
//! Di luar yang sudah ditangani `ConstFold` (cond konstanta, t==f):
//! - `Mux(Not(c), t, f)` → `Mux(c, f, t)` (swap branch, hilangkan inverter).

use maria_sir::{SirNodeKind, SirValue};

use crate::pass::{SynthContext, SynthPass};

/// Pass: penyederhanaan MUX.
pub struct MuxSimplify;

impl SynthPass for MuxSimplify {
    fn name(&self) -> &'static str {
        "mux"
    }

    fn run(&mut self, ctx: &mut SynthContext) -> Result<usize, maria_core::error::SimError> {
        let mut changed = 0usize;
        let nids: Vec<usize> = (0..ctx.module.nodes.len()).collect();
        for nid in nids {
            if !crate::opt::node_alive(&ctx.module, nid) {
                continue;
            }
            if ctx.module.nodes[nid].kind != SirNodeKind::Mux {
                continue;
            }
            let inputs = ctx.module.nodes[nid].inputs.clone();
            if inputs.len() != 3 {
                continue;
            }
            // Mux(Not(c), t, f) → Mux(c, f, t)
            let sel = inputs[0];
            if let Some(SirValue::Node(inner)) = ctx.module.values.get(sel) {
                if ctx.module.nodes[*inner].kind == SirNodeKind::Not {
                    let c = ctx.module.nodes[*inner].inputs[0];
                    ctx.module.nodes[nid].inputs = vec![c, inputs[2], inputs[1]];
                    changed += 1;
                }
            }
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maria_core::intern::Symbol;
    use maria_sir::SirModule;
    use crate::pass::SynthPipeline;

    #[test]
    fn mux_with_not_sel_swaps_branches() {
        let mut m = SirModule::new(Symbol::intern("top"));
        m.add_value(SirValue::Port(0)); // c
        m.add_value(SirValue::Port(1)); // t
        m.add_value(SirValue::Port(2)); // f
        let nn = m.add_node(SirNodeKind::Not, vec![0], 1);
        m.add_value(SirValue::Node(nn)); // vid 3 = Not(c)
        let nid = m.add_node(SirNodeKind::Mux, vec![3, 1, 2], 8);
        m.add_value(SirValue::Node(nid));
        let mut p = SynthPipeline::new();
        p.add(MuxSimplify);
        let (m2, results) = p.run(m).unwrap();
        assert_eq!(results[0].changed, 1);
        let node = &m2.nodes[nid];
        assert_eq!(node.inputs, vec![0, 2, 1], "Mux(Not(c),t,f) → Mux(c,f,t)");
    }
}
