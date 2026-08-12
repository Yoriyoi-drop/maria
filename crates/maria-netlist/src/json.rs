//! Emisi `netlist.json` — representasi netlist untuk GUI/CI (SYNTHESIS.md §13).

use serde_json::{Value, json};

use crate::net::{Netlist, PortDir};

/// Netlist → JSON (netlist.json).
pub fn emit_json(nl: &Netlist) -> String {
    let ports: Vec<Value> = nl
        .ports
        .iter()
        .map(|p| {
            json!({
                "name": p.name.as_str(),
                "dir": p.dir.name(),
                "width": p.width,
            })
        })
        .collect();

    let cells: Vec<Value> = nl
        .cells
        .iter()
        .enumerate()
        .map(|(id, c)| {
            let inputs: Vec<Value> = c
                .inputs
                .iter()
                .map(|pin| json!({ "pin": pin.pin, "net": nl.nets[pin.net].name.as_str() }))
                .collect();
            let outputs: Vec<Value> = c
                .outputs
                .iter()
                .map(|pin| json!({ "pin": pin.pin, "net": nl.nets[pin.net].name.as_str() }))
                .collect();
            json!({
                "id": id,
                "name": c.name.as_str(),
                "cell": c.kind.cell_name(),
                "width": c.width,
                "inputs": inputs,
                "outputs": outputs,
            })
        })
        .collect();

    let nets: Vec<Value> = nl
        .nets
        .iter()
        .enumerate()
        .map(|(id, n)| {
            let driver = match &n.driver {
                Some(d) => json!({ "cell": d.cell, "pin": d.pin }),
                None => Value::Null,
            };
            let loads: Vec<Value> = n
                .loads
                .iter()
                .map(|l| json!({ "cell": l.cell, "pin": l.pin }))
                .collect();
            json!({
                "id": id,
                "name": n.name.as_str(),
                "width": n.width,
                "driver": driver,
                "loads": loads,
                "clock": n.is_clock,
                "reset": n.is_reset,
                "io": n.is_io,
                "const": n.const_value,
            })
        })
        .collect();

    let s = crate::graph::stats(nl);
    let doc = json!({
        "name": nl.name.as_str(),
        "device": "generic",
        "stats": {
            "cells": s.cell_count,
            "comb_cells": s.comb_cells,
            "ff_cells": s.ff_cells,
            "nets": s.net_count,
            "max_fanout": s.max_fanout,
            "max_level": s.max_level,
        },
        "ports": ports,
        "cells": cells,
        "nets": nets,
    });
    serde_json::to_string_pretty(&doc).unwrap_or_default()
}

/// Helper untuk test: konversi kecil.
#[allow(dead_code)]
fn port_dir_name(d: &PortDir) -> &'static str {
    d.name()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lower::lower_module;
    use maria_core::{intern::Symbol, LogicVec};
    use maria_sir::{ResetSpec, SirNode, SirNodeKind, SirRegister, SirValue};

    fn counter_sir() -> maria_sir::SirModule {
        let mut m = maria_sir::SirModule::new(Symbol::intern("counter"));
        let _ = m.add_value(SirValue::Port(0));
        let _ = m.add_value(SirValue::Port(1));
        let _ = m.add_value(SirValue::Reg(0));
        let _ = m.add_value(SirValue::Const(LogicVec::from_u64(1, 8)));
        let n = m.add_node(SirNodeKind::Add, vec![2, 3], 8);
        let _ = m.add_value(SirValue::Node(n));
        m.registers.push(SirRegister {
            name: Symbol::intern("count"),
            d: 4,
            q: 2,
            clock: 0,
            reset: Some(ResetSpec {
                signal: 1,
                value: LogicVec::from_u64(0, 8),
                polarity: false,
                r#async: true,
            }),
            enable: None,
            width: 8,
        });
        let mk = |name: &str, dir: maria_sir::PortDir, value: usize, width: usize| maria_sir::SirPort {
            name: Symbol::intern(name),
            dir,
            width,
            value,
        };
        m.inputs.push(mk("clk", maria_sir::PortDir::Input, 0, 1));
        m.inputs.push(mk("rst_n", maria_sir::PortDir::Input, 1, 1));
        m.outputs.push(mk("count", maria_sir::PortDir::Output, 2, 8));
        m
    }

    #[test]
    fn json_has_all_sections() {
        let nl = lower_module(&counter_sir());
        let s = emit_json(&nl);
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["name"], "counter");
        assert_eq!(v["ports"].as_array().unwrap().len(), 3);
        assert_eq!(v["cells"].as_array().unwrap().len(), 2);
        assert_eq!(v["nets"].as_array().unwrap().len(), nl.nets.len());
        assert_eq!(v["stats"]["ff_cells"], 1);
    }
}
