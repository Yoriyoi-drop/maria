//! Model netlist + net (SYNTHESIS.md §11).
//!
//! Aturan netlist Maria:
//! 1. **1 driver, N loads** — acyclic DAG (bukan multi-driver bus).
//! 2. Semua lebar sudah final (dibangun dari `SirModule`, bukan AST).
//! 3. Port top-level → `Port`; koneksi antar sel → `Net`.
//! 4. Traceability: `src_map` dipakai dari SIR.

use maria_core::intern::Symbol;

use crate::cell::{CellId, PinRef};

pub type NetId = usize;

/// Arah port top-level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortDir {
    Input,
    Output,
    Inout,
}

impl PortDir {
    pub fn name(&self) -> &'static str {
        match self {
            PortDir::Input => "in",
            PortDir::Output => "out",
            PortDir::Inout => "inout",
        }
    }
}

/// Port top-level netlist.
#[derive(Debug, Clone, PartialEq)]
pub struct Port {
    pub name: Symbol,
    pub dir: PortDir,
    pub width: usize,
}

/// Net (sinyal listrik): satu driver, banyak loads.
#[derive(Debug, Clone, PartialEq)]
pub struct Net {
    pub name: Symbol,
    pub width: usize,
    /// Driver: pin output sel yang menggerakkan net ini (None = port input
    /// atau net internal tanpa driver — port didefinisikan terpisah).
    pub driver: Option<PinRef>,
    pub loads: Vec<PinRef>,
    pub is_clock: bool,
    pub is_reset: bool,
    pub is_io: bool,
    /// Konstanta yang menggerakkan net ini (None = net biasa). Net konstanta
    /// TIDAK punya driver sel — nilainya disuntikkan saat emit Verilog.
    pub const_value: Option<u64>,
}

/// Netlist gate-level hasil mapping SIR.
#[derive(Debug, Clone, PartialEq)]
pub struct Netlist {
    pub name: Symbol,
    pub ports: Vec<Port>,
    pub cells: Vec<crate::cell::CellInstance>,
    pub nets: Vec<Net>,
}

impl Netlist {
    pub fn new(name: Symbol) -> Self {
        Netlist {
            name,
            ports: Vec::new(),
            cells: Vec::new(),
            nets: Vec::new(),
        }
    }

    pub fn add_port(&mut self, name: Symbol, dir: PortDir, width: usize) {
        self.ports.push(Port { name, dir, width });
    }

    pub fn add_net(&mut self, name: Symbol, width: usize) -> NetId {
        let id = self.nets.len();
        self.nets.push(Net {
            name,
            width,
            driver: None,
            loads: Vec::new(),
            is_clock: false,
            is_reset: false,
            is_io: false,
            const_value: None,
        });
        id
    }

    /// Tambah sel, koneksikan driver/load otomatis dari `inputs`/`outputs`.
    pub fn add_cell(&mut self, cell: crate::cell::CellInstance) -> CellId {
        let id = self.cells.len();
        for pin in cell.inputs.iter() {
            self.nets[pin.net].loads.push(PinRef {
                cell: id,
                pin: pin.pin.clone(),
            });
        }
        for pin in cell.outputs.iter() {
            debug_assert!(
                self.nets[pin.net].driver.is_none(),
                "net {} punya 2 driver ({} & {})",
                self.nets[pin.net].name.as_str(),
                self.nets[pin.net]
                    .driver
                    .as_ref()
                    .map(|p| p.cell.to_string())
                    .unwrap_or_default(),
                id
            );
            self.nets[pin.net].driver = Some(PinRef {
                cell: id,
                pin: pin.pin.clone(),
            });
        }
        self.cells.push(cell);
        id
    }

    /// Fanout (jumlah loads) sebuah net.
    pub fn net_fanout(&self, net: NetId) -> usize {
        self.nets[net].loads.len()
    }

    /// Jumlah sel sequential.
    pub fn ff_count(&self) -> usize {
        self.cells.iter().filter(|c| c.kind.is_sequential()).count()
    }

    /// Nama net (untuk dump/report).
    pub fn net_name(&self, id: NetId) -> &Symbol {
        &self.nets[id].name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellInstance;

    fn one_ff_netlist() -> Netlist {
        let mut nl = Netlist::new(Symbol::intern("top"));
        nl.add_port(Symbol::intern("clk"), PortDir::Input, 1);
        nl.add_port(Symbol::intern("q"), PortDir::Output, 8);
        let clk = nl.add_net(Symbol::intern("clk"), 1);
        let d = nl.add_net(Symbol::intern("d"), 8);
        let q = nl.add_net(Symbol::intern("q"), 8);
        nl.nets[clk].is_clock = true;
        nl.nets[q].is_io = true;

        let mut ff = CellInstance::new(Symbol::intern("u0"), crate::cell::CellKind::Dff, 8);
        ff.inputs = vec![
            crate::cell::PinConn { net: clk, pin: "c".into(), bit: None },
            crate::cell::PinConn { net: d, pin: "d".into(), bit: None },
        ];
        ff.outputs = vec![crate::cell::PinConn { net: q, pin: "q".into(), bit: None }];
        nl.add_cell(ff);
        nl
    }

    #[test]
    fn one_driver_n_loads() {
        let nl = one_ff_netlist();
        assert_eq!(nl.ff_count(), 1);
        assert_eq!(nl.net_fanout(1), 1, "net d punya 1 load");
        assert_eq!(nl.nets[2].driver.as_ref().map(|p| p.cell), Some(0));
        assert!(nl.nets.iter().any(|n| n.is_clock));
        assert!(nl.nets.iter().any(|n| n.is_io));
    }

    #[test]
    #[should_panic(expected = "2 driver")]
    fn add_cell_detects_double_driver() {
        // Invariant netlist: 1 driver per net. Dua sel menggerakkan net sama
        // harus di-reject (debug_assert di add_cell).
        let mut nl = Netlist::new(Symbol::intern("top"));
        let n = nl.add_net(Symbol::intern("x"), 1);
        let mut c1 = CellInstance::new(Symbol::intern("u0"), crate::cell::CellKind::Buffer, 1);
        c1.outputs = vec![crate::cell::PinConn { net: n, pin: "y".into(), bit: None }];
        let mut c2 = CellInstance::new(Symbol::intern("u1"), crate::cell::CellKind::Buffer, 1);
        c2.outputs = vec![crate::cell::PinConn { net: n, pin: "y".into(), bit: None }];
        nl.add_cell(c1);
        nl.add_cell(c2); // panic: net x punya 2 driver
    }
}
