//! Lowering structural: `SirModule` → `Netlist` gate-level generik
//! (SYNTHESIS.md §11 — SIR → generic netlist, phase 3).
//!
//! Aturan mapping:
//! - Setiap **node** SIR → satu sel (`CellKind` sesuai `SirNodeKind`),
//!   input pin → net operand, output pin → net hasil node.
//! - Setiap **register** SIR → satu sel `Dff`/`DffE`/`DffR`/`DffRE`
//!   (clock → pin `c`, reset → `r`, enable → `ce`, d/q → net).
//! - Setiap **nilai** SIR yang hidup (dipakai) → satu net. Konstanta →
//!   net dengan `const_value` (tanpa driver sel).
//! - Node mati (output tak dipakai) TIDAK di-lower — DAG yang dihasilkan
//!   tetap 1-driver/N-loads, acyclic.

use maria_core::intern::Symbol;
use maria_sir::{SirModule, SirNodeKind, SirValue};

use crate::cell::{CellInstance, CellKind, PinConn};
use crate::net::{NetId, Netlist, PortDir};

/// Lower seluruh modul SIR → netlist.
pub fn lower_module(sir: &SirModule) -> Netlist {
    let mut nl = Netlist::new(sir.name);

    // ── Liveness: nilai yang dipakai (roots: port output + register) ──
    //
    // Catatan penting: port output / signal internal bisa berupa ALIAS —
    // slot yang isinya `SirValue::Node(nid)` padahal node output-nya vid
    // kanonik lain (lower SIR menyalin `values[v]` ke slot signal). Jadi
    // propagasi harus lewat ISI nilai, bukan sekadar `used[node.output]`.
    let mut used = vec![false; sir.values.len()];
    for p in &sir.outputs {
        used[p.value] = true;
    }
    for r in &sir.registers {
        used[r.d] = true;
        used[r.q] = true;
        used[r.clock] = true;
        if let Some(e) = r.enable {
            used[e] = true;
        }
        if let Some(rs) = &r.reset {
            used[rs.signal] = true;
        }
    }
    // Propagasi: vid yang dipakai dan berisi Node → hidupkan vid output
    // kanonik node + semua operand; berulang sampai stabil (DAG acyclic).
    loop {
        let mut changed = false;
        for (vid, v) in sir.values.iter().enumerate() {
            if !used[vid] {
                continue;
            }
            if let SirValue::Node(nid) = v {
                let node = &sir.nodes[*nid];
                if !used[node.output] {
                    used[node.output] = true;
                    changed = true;
                }
                for &i in &node.inputs {
                    if !used[i] {
                        used[i] = true;
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    // Port input tetap dibuat walau tak dipakai (kontrak modul).
    for p in &sir.inputs {
        used[p.value] = true;
    }

    // ── Value → Net ──
    //
    // Node yang sama bisa muncul di BANYAK vid (port output / signal internal
    // adalah ALIAS — salinan `SirValue::Node` di slot terpisah). Semua alias
    // harus menunjuk ke net KANONIK yang sama (satu sinyal listrik = satu net,
    // invariant 1-driver). Pass 1: net untuk vid kanonik node + lainnya;
    // pass 2: alias node → net kanonik.
    let mut value_net: Vec<Option<NetId>> = vec![None; sir.values.len()];

    // Pass 1: buat net untuk semua vid yang dipakai, KECUALI alias node
    // (vid berisi Node tapi bukan output kanonik node) dan alias port
    // (vid berisi Port tapi bukan `port.value` — port input punya slot sendiri).
    for (vid, v) in sir.values.iter().enumerate() {
        if !used[vid] {
            continue;
        }
        let canonical = match v {
            SirValue::Node(nid) => sir.nodes[*nid].output == vid,
            SirValue::Port(pid) => {
                // vid kanonik port = `port.value` dari port input/inout.
                let port_value = sir.inputs.iter().chain(sir.outputs.iter()).find(|p| {
                    p.value == vid
                        && matches!(&sir.values[p.value], SirValue::Port(pp) if *pp == *pid)
                });
                port_value.is_some()
            }
            _ => true,
        };
        if !canonical {
            continue; // alias → pass 2
        }
        let net = make_value_net(&mut nl, sir, &used, vid, v);
        value_net[vid] = Some(net);
    }

    // Pass 2: alias node → net node kanonik; alias port → net port kanonik.
    for (vid, v) in sir.values.iter().enumerate() {
        if !used[vid] || value_net[vid].is_some() {
            continue;
        }
        match v {
            SirValue::Node(nid) => {
                if let Some(net) = value_net[sir.nodes[*nid].output] {
                    value_net[vid] = Some(net);
                }
            }
            SirValue::Port(pid) => {
                // Cari port input/inout yang berisi Port(pid) yang SAMA.
                let canon = sir.inputs.iter().chain(sir.outputs.iter()).find(|p| {
                    p.value != vid
                        && matches!(&sir.values[p.value], SirValue::Port(pp) if *pp == *pid)
                });
                if let Some(p) = canon {
                    if let Some(net) = value_net[p.value] {
                        value_net[vid] = Some(net);
                    }
                }
            }
            _ => {}
        }
    }
    // Alias yang masih kosong (defensive) → net sendiri.
    for (vid, v) in sir.values.iter().enumerate() {
        if used[vid] && value_net[vid].is_none() {
            let net = make_value_net(&mut nl, sir, &used, vid, v);
            value_net[vid] = Some(net);
        }
    }

    // ── Port top-level ──
    for p in &sir.inputs {
        let dir = match p.dir {
            maria_sir::PortDir::Input => PortDir::Input,
            maria_sir::PortDir::Inout => PortDir::Inout,
            maria_sir::PortDir::Output => PortDir::Output,
        };
        nl.add_port(p.name, dir, p.width);
        if let Some(net) = value_net[p.value] {
            nl.nets[net].is_io = true;
        }
    }
    for p in &sir.outputs {
        nl.add_port(p.name, PortDir::Output, p.width);
        if let Some(net) = value_net[p.value] {
            nl.nets[net].is_io = true;
        }
    }

    // ── Node → sel ──
    for (nid, n) in sir.nodes.iter().enumerate() {
        if !used[n.output] {
            continue;
        }
        let kind = map_kind(&n.kind);
        let mut cell =
            CellInstance::new(Symbol::intern(&format!("u{}", nid)), kind.clone(), n.width);
        // in_width: operand data utama (Mux: t/f; lainnya: input pertama).
        let data_input = if matches!(kind, CellKind::Mux) { 1 } else { 0 };
        if let Some(&di) = n.inputs.get(data_input) {
            cell.in_width = sir.value_width(di);
        } else if let Some(&di) = n.inputs.first() {
            cell.in_width = sir.value_width(di);
        }
        for (i, &inp) in n.inputs.iter().enumerate() {
            cell.inputs.push(PinConn {
                net: value_net[inp].expect("input node hidup"),
                pin: input_pin_name(&cell.kind, i),
                bit: None,
            });
        }
        cell.outputs.push(PinConn {
            net: value_net[n.output].expect("output node hidup"),
            pin: "y".into(),
            bit: None,
        });
        nl.add_cell(cell);
    }

    // ── Register → sel FF ──
    for (rid, r) in sir.registers.iter().enumerate() {
        let kind = ff_kind(r);
        let mut cell = CellInstance::new(
            Symbol::intern(&format!("ff_{}", r.name.as_str())),
            kind,
            r.width,
        );
        cell.in_width = r.width;
        cell.inputs.push(PinConn {
            net: value_net[r.clock].expect("clock hidup"),
            pin: "c".into(),
            bit: None,
        });
        if let Some(rs) = &r.reset {
            cell.inputs.push(PinConn {
                net: value_net[rs.signal].expect("reset hidup"),
                pin: "r".into(),
                bit: None,
            });
        }
        if let Some(e) = r.enable {
            cell.inputs.push(PinConn {
                net: value_net[e].expect("enable hidup"),
                pin: "ce".into(),
                bit: None,
            });
        }
        cell.inputs.push(PinConn {
            net: value_net[r.d].expect("d hidup"),
            pin: "d".into(),
            bit: None,
        });
        cell.outputs.push(PinConn {
            net: value_net[r.q].expect("q hidup"),
            pin: "q".into(),
            bit: None,
        });
        nl.add_cell(cell);
        let _ = rid;
    }

    nl
}

/// Nama pin input ke-i (Concat: `p0, p1, ...`; lainnya dari schema).
fn input_pin_name(kind: &CellKind, i: usize) -> String {
    if matches!(kind, CellKind::Concat) {
        format!("p{}", i)
    } else {
        kind.input_pins()[i].to_string()
    }
}

/// Jenis sel FF dari spesifikasi register SIR.
fn ff_kind(r: &maria_sir::SirRegister) -> CellKind {
    match (&r.reset, r.enable) {
        (Some(rs), Some(_)) => CellKind::DffRE {
            reset_value: clamp_u64(&rs.value),
            polarity: rs.polarity,
            r#async: rs.r#async,
        },
        (Some(rs), None) => CellKind::DffR {
            reset_value: clamp_u64(&rs.value),
            polarity: rs.polarity,
            r#async: rs.r#async,
        },
        (None, Some(_)) => CellKind::DffE,
        (None, None) => CellKind::Dff,
    }
}

/// Konstanta SIR → u64 (clamp lebar > 64 bit).
fn clamp_u64(lv: &maria_core::LogicVec) -> u64 {
    lv.to_u64()
}

/// Buat satu net untuk sebuah vid (bukan alias node).
fn make_value_net(
    nl: &mut Netlist,
    sir: &SirModule,
    _used: &[bool],
    vid: usize,
    v: &SirValue,
) -> NetId {
    // Nama net: wire bernama (signal RTL) → nama wire; port → nama port;
    // sisanya fallback `n{nid}` / `c{vid}` / `q_{reg}`.
    //
    // Node bisa punya ALIAS vid (port output / wire signal menunjuk vid yang
    // berisi salinan `SirValue::Node(nid)` — bukan vid kanonik node). Nama
    // net kanonik diambil dari wire/port yang menunjuk ke vid MANA PUN yang
    // berisi node yang sama, agar `y` (bukan `n2`) yang muncul di netlist.
    let node_nid = match v {
        SirValue::Node(nid) => Some(*nid),
        _ => None,
    };
    let same_node = |target: usize| matches!(&sir.values[target], SirValue::Node(nid) if node_nid == Some(*nid));
    let wire = sir
        .wires
        .iter()
        .find(|w| w.value == vid || same_node(w.value))
        .or_else(|| {
            // Bila vid ini kanonik node (belum ada wire alias), cari wire
            // yang menunjuk vid lain yang berisi node yang sama.
            sir.wires
                .iter()
                .find(|w| w.value != vid && same_node(w.value))
        });
    let port = sir
        .inputs
        .iter()
        .chain(sir.outputs.iter())
        .find(|p| p.value == vid || same_node(p.value))
        .or_else(|| {
            sir.inputs
                .iter()
                .chain(sir.outputs.iter())
                .find(|p| p.value != vid && same_node(p.value))
        });
    let (name, is_clock, is_reset, is_io) = match (wire, port) {
        (Some(w), _) => (w.name, w.is_clock, w.is_reset, w.is_io),
        (None, Some(p)) => (p.name, false, false, true),
        (None, None) => {
            let fallback = match v {
                SirValue::Node(nid) => Symbol::intern(&format!("n{}", nid)),
                SirValue::Reg(rid) => {
                    let rn = sir.registers[*rid].name.as_str();
                    Symbol::intern(&format!("q_{}", rn))
                }
                SirValue::Const(_) => Symbol::intern(&format!("c{}", vid)),
                SirValue::Port(pid) => Symbol::intern(&format!("p{}", pid)),
            };
            (fallback, false, false, false)
        }
    };
    let width = sir.value_width(vid);
    let net = nl.add_net(name, width);
    nl.nets[net].is_clock = is_clock;
    nl.nets[net].is_reset = is_reset;
    nl.nets[net].is_io = is_io;
    if let SirValue::Const(lv) = v {
        nl.nets[net].const_value = Some(clamp_u64(lv));
    }
    net
}

/// Map node SIR → sel generik.
fn map_kind(k: &SirNodeKind) -> CellKind {
    match k {
        SirNodeKind::And => CellKind::And,
        SirNodeKind::Or => CellKind::Or,
        SirNodeKind::Xor => CellKind::Xor,
        SirNodeKind::Not => CellKind::Not,
        SirNodeKind::Mux => CellKind::Mux,
        SirNodeKind::Add => CellKind::Add,
        SirNodeKind::Sub => CellKind::Sub,
        SirNodeKind::Mul => CellKind::Mul,
        SirNodeKind::Div => CellKind::Div,
        SirNodeKind::Mod => CellKind::Mod,
        SirNodeKind::Shl => CellKind::Shl,
        SirNodeKind::Shr => CellKind::Shr,
        SirNodeKind::Sar => CellKind::Sar,
        SirNodeKind::Eq => CellKind::Eq,
        SirNodeKind::Ne => CellKind::Ne,
        SirNodeKind::Lt => CellKind::Lt,
        SirNodeKind::Le => CellKind::Le,
        SirNodeKind::Gt => CellKind::Gt,
        SirNodeKind::Ge => CellKind::Ge,
        SirNodeKind::ReduceAnd => CellKind::ReduceAnd,
        SirNodeKind::ReduceOr => CellKind::ReduceOr,
        SirNodeKind::ReduceXor => CellKind::ReduceXor,
        SirNodeKind::Concat => CellKind::Concat,
        SirNodeKind::Slice { msb, lsb } => CellKind::Slice {
            msb: *msb,
            lsb: *lsb,
        },
        SirNodeKind::Buffer => CellKind::Buffer,
        SirNodeKind::TriState => CellKind::TriState,
    }
}

/// Uji: lowering SirModule buatan tangan (register + ADD) → netlist.
#[cfg(test)]
mod tests {
    use super::*;
    use maria_core::{intern::Symbol, LogicVec};
    use maria_sir::{ResetSpec, SirNode, SirRegister, SirValue, ValueId};

    fn counter_sir() -> SirModule {
        let mut m = SirModule::new(Symbol::intern("counter"));
        // values: 0=clk(port in), 1=rst_n(port in), 2=count(reg), 3=const1
        let _ = m.add_value(SirValue::Port(0)); // 0
        let _ = m.add_value(SirValue::Port(1)); // 1
        let _ = m.add_value(SirValue::Reg(0)); // 2
        let _ = m.add_value(SirValue::Const(LogicVec::from_u64(1, 8))); // 3
                                                                        // node ADD(count, 1) → output vid 4
        let n = m.add_node(SirNodeKind::Add, vec![2, 3], 8);
        let _ = m.add_value(SirValue::Node(n)); // 4
                                                // register count: d=4, q=2, clk=0, rst=1
        let clk = 0;
        let d = 4;
        let q = 2;
        let rst = 1;
        m.registers.push(SirRegister {
            name: Symbol::intern("count"),
            d,
            q,
            clock: clk,
            reset: Some(ResetSpec {
                signal: rst,
                value: LogicVec::from_u64(0, 8),
                polarity: false,
                r#async: true,
            }),
            enable: None,
            width: 8,
        });
        // ports
        let mk = |name: &str, dir: maria_sir::PortDir, value: ValueId, width: usize| {
            maria_sir::SirPort {
                name: Symbol::intern(name),
                dir,
                width,
                value,
            }
        };
        m.inputs.push(mk("clk", maria_sir::PortDir::Input, 0, 1));
        m.inputs.push(mk("rst_n", maria_sir::PortDir::Input, 1, 1));
        m.outputs
            .push(mk("count", maria_sir::PortDir::Output, 2, 8));
        m
    }

    #[test]
    fn lower_counter_netlist() {
        let sir = counter_sir();
        let nl = lower_module(&sir);
        assert_eq!(nl.ports.len(), 3);
        assert_eq!(nl.cells.len(), 2, "ADD + FF");
        assert_eq!(nl.ff_count(), 1);
        // 1 driver per net — tidak ada yang di-drive dua sel.
        for net in &nl.nets {
            if !net.is_io {
                // konstanta tidak punya driver; internal harus di-drive.
                if net.const_value.is_none() {
                    assert!(
                        net.driver.is_some(),
                        "net {} harus punya driver",
                        net.name.as_str()
                    );
                }
            }
        }
        // ADD input: count (reg q) + konstanta 1.
        let add = nl
            .cells
            .iter()
            .find(|c| c.kind == CellKind::Add)
            .expect("sel ADD");
        assert_eq!(add.inputs.len(), 2);
        let c1_net = nl
            .nets
            .iter()
            .find(|n| n.const_value == Some(1))
            .expect("konstanta 1");
        assert_eq!(c1_net.width, 8, "konstanta harus di-fit ke lebar operand");
    }

    #[test]
    fn netlist_names_are_stable() {
        let a = lower_module(&counter_sir());
        let b = lower_module(&counter_sir());
        assert_eq!(a, b, "lowering harus deterministik");
    }

    #[test]
    fn comb_port_input_gets_net_with_name() {
        // Regresi e2e: port input `a` (vid berisi SirValue::Port) harus
        // mendapatkan net ber-nama `a` (bukan `p0` mengambang). Ini menjamin
        // wire SIR yang menunjuk vid Port tetap terhubung ke net yang sama.
        let mut m = SirModule::new(Symbol::intern("g"));
        let _ = m.add_value(SirValue::Port(0)); // 0 — input a
        let _ = m.add_value(SirValue::Port(1)); // 1 — input b
        let n = m.add_node(SirNodeKind::And, vec![0, 1], 8);
        let out = m.nodes[n].output;
        let _ = m.add_value(SirValue::Node(n)); // out — kanonik
                                                // Port output y → ALIAS node (slot berbeda berisi Node(n)).
        let _ = m.add_value(SirValue::Node(n)); // alias
        let _ = m.add_wire(Symbol::intern("a"), 8, 0);
        let _ = m.add_wire(Symbol::intern("b"), 8, 1);
        let _ = m.add_wire(Symbol::intern("y"), 8, 3);
        let mk = |name: &str, dir: maria_sir::PortDir, value: ValueId, width: usize| {
            maria_sir::SirPort {
                name: Symbol::intern(name),
                dir,
                width,
                value,
            }
        };
        m.inputs.push(mk("a", maria_sir::PortDir::Input, 0, 8));
        m.inputs.push(mk("b", maria_sir::PortDir::Input, 1, 8));
        m.outputs.push(mk("y", maria_sir::PortDir::Output, 3, 8));
        let nl = lower_module(&m);
        // Port a & b punya net bernama a/b, dan node output y juga net y.
        let net_a = nl
            .nets
            .iter()
            .find(|n| n.name.as_str() == "a")
            .expect("net a");
        assert!(!net_a.loads.is_empty(), "net a harus punya load (AND.a)");
        let net_y = nl
            .nets
            .iter()
            .find(|n| n.name.as_str() == "y")
            .expect("net y");
        assert!(net_y.driver.is_some(), "net y harus di-drive sel AND");
        assert!(
            nl.cells.iter().any(|c| c.kind == CellKind::And),
            "ada sel AND"
        );
    }
}
