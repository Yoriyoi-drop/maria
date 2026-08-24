//! Model netlist gate-level Maria (S1) — struktur data murni, technology-agnostic.
//!
//! Aturan (SYNTHESIS.md §3):
//! 1. 1 driver, N loads — netlist acyclic (DAG).
//! 2. Semua lebar sudah di-resolve (dibangun dari IrDesign, bukan AST).
//! 3. Traceability — `src_map` memetakan net → signal RTL + lokasi source.

use maria_core::intern::Symbol;
use std::collections::HashMap;

pub type NetId = usize;
pub type InstId = usize;

/// Jenis device target (S1: hanya fpga-x7 & generic — asic menyusul di S4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceKind {
    FpgaX7,
    Generic,
}

impl DeviceKind {
    pub fn name(&self) -> &'static str {
        match self {
            DeviceKind::FpgaX7 => "fpga-x7",
            DeviceKind::Generic => "generic",
        }
    }
}

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

/// Jenis sel (primitif). S1 meng-emit sel struktural dasar + FF hasil inferensi.
#[derive(Debug, Clone, PartialEq)]
pub enum CellKind {
    /// k-LUT dengan truth table `init` (FPGA). S2 mengisi init nyata;
    /// S1 memakai init 0 sebagai placeholder struktural.
    Lut {
        k: usize,
        init: u64,
    },
    /// D-flip-flop polos.
    Dff,
    /// FF dengan reset (nilai reset dari `ResetInfo.value`).
    DffR {
        reset_value: u64,
    },
    /// FF + clock enable (`Process::Sequential.iff`).
    DffE,
    /// FF + reset + clock enable.
    DffRE {
        reset_value: u64,
    },
    /// Half/full adder (primitif carry chain).
    AddHalf,
    AddFull,
    /// RAM/ROM (S1: placeholder estimasi — inferensi penuh di S3).
    Brams,
    Rom,
    /// Multiplier (S1: estimasi — inferensi DSP di S3).
    DspMul,
    /// Buffer I/O (FPGA pad).
    Ibuf,
    Obuf,
    Iobuf,
    /// Global clock buffer.
    Bufg,
    /// Buffer identitas (koneksi langsung).
    PassThrough,
}

impl CellKind {
    pub fn is_sequential(&self) -> bool {
        matches!(
            self,
            CellKind::Dff | CellKind::DffR { .. } | CellKind::DffE | CellKind::DffRE { .. }
        )
    }

    pub fn is_mem(&self) -> bool {
        matches!(self, CellKind::Brams | CellKind::Rom)
    }

    /// Nama sel untuk output netlist/report.
    pub fn cell_name(&self) -> String {
        match self {
            CellKind::Lut { k, .. } => format!("LUT{}", k),
            CellKind::Dff => "FDRE".into(),
            CellKind::DffR { .. } => "FDRE".into(),
            CellKind::DffE => "FDE".into(),
            CellKind::DffRE { .. } => "FDRE".into(),
            CellKind::AddHalf => "HALFADDER".into(),
            CellKind::AddFull => "FULLADDER".into(),
            CellKind::Brams => "BRAM36".into(),
            CellKind::Rom => "ROM36".into(),
            CellKind::DspMul => "DSP48".into(),
            CellKind::Ibuf => "IBUF".into(),
            CellKind::Obuf => "OBUF".into(),
            CellKind::Iobuf => "IOBUF".into(),
            CellKind::Bufg => "BUFG".into(),
            CellKind::PassThrough => "BUF".into(),
        }
    }
}

/// Referensi pin: (net_id, nama pin).
#[derive(Debug, Clone, PartialEq)]
pub struct PinRef {
    pub net: NetId,
    pub pin: &'static str,
}

/// Instance sel dalam netlist.
#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    pub name: Symbol,
    pub kind: CellKind,
    pub inputs: Vec<PinRef>,
    pub outputs: Vec<PinRef>,
    /// Lokasi fisik saat place (S5) — None sebelum place.
    pub loc: Option<(usize, usize, usize)>, // (baris, kolom, slot slice)
}

/// Net (sinyal) — satu driver, banyak loads.
#[derive(Debug, Clone, PartialEq)]
pub struct Net {
    pub name: Symbol,
    pub width: usize,
    pub driver: Option<InstId>,
    pub loads: Vec<InstId>,
    pub is_clock: bool,
    pub is_reset: bool,
    pub is_io: bool,
    /// Diisi saat route (S5) — delay wire dalam ps.
    pub wire_delay_ps: Option<u32>,
}

/// Statistik synthesis (S1: estimasi utilisasi).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SynthStats {
    pub ff_count: usize,
    pub lut_count: usize,
    pub carry4_count: usize,
    pub bram_count: usize,
    pub rom_count: usize,
    pub dsp_count: usize,
    pub io_count: usize,
    pub bufg_count: usize,
    pub logic_nodes: usize,
    pub process_count: usize,
    pub fsm_count: usize,
    pub mem_bits: usize,
}

/// Netlist lengkap hasil synthesis.
#[derive(Debug, Clone, PartialEq)]
pub struct Netlist {
    pub name: Symbol,
    pub device: DeviceKind,
    pub ports: Vec<Port>,
    pub instances: Vec<Instance>,
    pub nets: Vec<Net>,
    /// Traceability: signal RTL → (file, line, col) di mana di-assign.
    pub src_map: HashMap<Symbol, (String, usize, usize)>,
    pub stats: SynthStats,
}

impl Netlist {
    pub fn new(name: Symbol, device: DeviceKind) -> Self {
        Netlist {
            name,
            device,
            ports: Vec::new(),
            instances: Vec::new(),
            nets: Vec::new(),
            src_map: HashMap::new(),
            stats: SynthStats::default(),
        }
    }

    /// Tambah net, kembalikan id-nya.
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
            wire_delay_ps: None,
        });
        id
    }

    /// Tambah instance, catat driver/load ke net.
    pub fn add_instance(&mut self, inst: Instance) -> InstId {
        let id = self.instances.len();
        for pin in inst.inputs.iter().chain(inst.outputs.iter()) {
            let net = &mut self.nets[pin.net];
            if pin.pin.starts_with("q") || pin.pin.starts_with("o") {
                // Output pin: driver
                net.driver = Some(id);
            } else {
                net.loads.push(id);
            }
        }
        self.instances.push(inst);
        id
    }

    /// Jumlah pin net (untuk debug/statistik).
    pub fn net_fanout(&self, net: NetId) -> usize {
        self.nets[net].loads.len()
    }

    /// Iterasi semua instance sequential (FF).
    pub fn ffs(&self) -> impl Iterator<Item = &Instance> {
        self.instances.iter().filter(|i| i.kind.is_sequential())
    }
}

/// Konstruktor instance FF (dipakai infer.rs + test).
pub fn ff_instance(
    name: Symbol,
    kind: CellKind,
    clk_net: NetId,
    d_net: NetId,
    q_net: NetId,
    reset: Option<(NetId, u64)>,
    enable: Option<NetId>,
) -> Instance {
    let mut inputs = vec![
        PinRef {
            net: clk_net,
            pin: "c",
        },
        PinRef {
            net: d_net,
            pin: "d",
        },
    ];
    if let Some((r, _v)) = reset {
        inputs.push(PinRef { net: r, pin: "r" });
    }
    if let Some(e) = enable {
        inputs.push(PinRef { net: e, pin: "ce" });
    }
    let _ = kind;
    Instance {
        name,
        kind,
        inputs,
        outputs: vec![PinRef {
            net: q_net,
            pin: "q",
        }],
        loc: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netlist_add_net_and_instance_links_driver_load() {
        let mut nl = Netlist::new(Symbol::intern("top"), DeviceKind::FpgaX7);
        let clk = nl.add_net(Symbol::intern("clk"), 1);
        let d = nl.add_net(Symbol::intern("d"), 1);
        let q = nl.add_net(Symbol::intern("q"), 1);
        let ff = ff_instance(
            Symbol::intern("u0"),
            CellKind::DffR { reset_value: 0 },
            clk,
            d,
            q,
            None,
            None,
        );
        nl.add_instance(ff);
        assert_eq!(nl.net_fanout(d), 1);
        assert_eq!(nl.nets[q].driver, Some(0));
        assert_eq!(nl.ffs().count(), 1);
        assert_eq!(nl.nets[clk].is_clock, false); // set terpisah oleh infer
    }

    #[test]
    fn cell_kind_sequential_flags() {
        assert!(CellKind::Dff.is_sequential());
        assert!(CellKind::DffRE { reset_value: 1 }.is_sequential());
        assert!(!CellKind::Lut { k: 6, init: 0 }.is_sequential());
        assert!(CellKind::Brams.is_mem());
    }
}
