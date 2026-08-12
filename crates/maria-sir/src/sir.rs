//! Model SIR — Synthesis Intermediate Representation (SYNTHESIS.md §3).
//!
//! SIR adalah graf nilai **node-based** (SSA-like), bukan pohon statement.
//! Synthesis tidak peduli struktur syntax (`always_ff`/`if`/`assign`) — ia
//! hanya peduli *siapa menghitung apa*:
//!
//! ```text
//! assign y = (a & b) | c;
//!
//!   Node 0: AND  input0 = a  input1 = b
//!   Node 1: OR   input0 = Node0 input1 = c
//!   output y = Node1
//! ```
//!
//! Aturan:
//! 1. Satu `SirModule` = satu modul (dari `IrDesign` hasil elaborator —
//!    width/kind sudah final, bukan dari AST).
//! 2. Semua nilai adalah bit-vector; lebar tercatat per node/register.
//! 3. Traceability: `src_map` memetakan signal → `file:line:col` RTL.
//! 4. JANGAN menyimpan bentuk Verilog (`always_ff`/`if`/`else`) — register
//!    diekspresikan sebagai `SirRegister { clock, reset, enable, d, q }`.

use maria_core::intern::Symbol;
use maria_core::LogicVec;
use std::collections::HashMap;

pub type PortId = usize;
pub type WireId = usize;
pub type NodeId = usize;
pub type RegisterId = usize;
pub type ValueId = usize;

/// Arah port top-level SIR.
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

/// Port top-level modul SIR.
#[derive(Debug, Clone, PartialEq)]
pub struct SirPort {
    pub name: Symbol,
    pub dir: PortDir,
    pub width: usize,
    /// Nilai yang dibawa port (input: slot-nya sendiri; output: slot signal).
    pub value: ValueId,
}

/// Wire bernama (net) — koneksi antar node/register yang punya nama RTL.
#[derive(Debug, Clone, PartialEq)]
pub struct SirWire {
    pub name: Symbol,
    pub width: usize,
    /// Nilai yang di-drive wire ini (Const / Node / Reg / Port).
    pub value: ValueId,
    pub is_clock: bool,
    pub is_reset: bool,
    pub is_io: bool,
}

/// Jenis node logika SIR (primitif teknologi-agnostik).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SirNodeKind {
    // ── Boolean —─
    And,
    Or,
    Xor,
    Not,
    // ── Seleksi —─
    Mux,
    // ── Aritmetika —─
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    // ── Shift —─
    Shl,
    Shr,
    /// Shift kanan aritmetik (mempertahankan sign).
    Sar,
    // ── Perbandingan —─
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    // ── Reduksi —─
    ReduceAnd,
    ReduceOr,
    ReduceXor,
    // ── Bit-vector —─
    Concat,
    Slice { msb: usize, lsb: usize },
    // ── Buffer / I/O —─
    Buffer,
    TriState,
}

impl SirNodeKind {
    pub fn name(&self) -> &'static str {
        match self {
            SirNodeKind::And => "AND",
            SirNodeKind::Or => "OR",
            SirNodeKind::Xor => "XOR",
            SirNodeKind::Not => "NOT",
            SirNodeKind::Mux => "MUX",
            SirNodeKind::Add => "ADD",
            SirNodeKind::Sub => "SUB",
            SirNodeKind::Mul => "MUL",
            SirNodeKind::Div => "DIV",
            SirNodeKind::Mod => "MOD",
            SirNodeKind::Shl => "SHL",
            SirNodeKind::Shr => "SHR",
            SirNodeKind::Sar => "SAR",
            SirNodeKind::Eq => "EQ",
            SirNodeKind::Ne => "NE",
            SirNodeKind::Lt => "LT",
            SirNodeKind::Le => "LE",
            SirNodeKind::Gt => "GT",
            SirNodeKind::Ge => "GE",
            SirNodeKind::ReduceAnd => "REDAND",
            SirNodeKind::ReduceOr => "REDOR",
            SirNodeKind::ReduceXor => "REDXOR",
            SirNodeKind::Concat => "CONCAT",
            SirNodeKind::Slice { .. } => "SLICE",
            SirNodeKind::Buffer => "BUF",
            SirNodeKind::TriState => "TRI",
        }
    }
}

/// Node logika SIR — hasilnya adalah satu bit-vector (lebar `width`).
#[derive(Debug, Clone, PartialEq)]
pub struct SirNode {
    pub kind: SirNodeKind,
    /// Operand (ValueId). Mux: [sel, t, f]. Slice: [base].
    pub inputs: Vec<ValueId>,
    pub width: usize,
    /// Slot nilai OUTPUT node ini (`values[output] == SirValue::Node(self)`
    /// selama node hidup). Dipakai pass optimizer untuk menulis ulang hasil
    /// node (fold/alias) tanpa mengubah referensi lain.
    pub output: ValueId,
    /// Traceability: `file:line:col` di RTL (bila tersedia).
    pub src: Option<(String, usize, usize)>,
}

/// Spesifikasi reset register.
#[derive(Debug, Clone, PartialEq)]
pub struct ResetSpec {
    /// Nilai (ValueId) dari signal reset.
    pub signal: ValueId,
    /// Nilai saat reset (bit-vector).
    pub value: LogicVec,
    /// true = active-high, false = active-low.
    pub polarity: bool,
    /// true = async, false = sync.
    pub r#async: bool,
}

/// Register (FF) — satu D → satu Q, dengan clock/reset/enable eksplisit.
///
/// Bukan bentuk `always_ff begin if ... end` — synthesis hanya peduli:
/// clock, reset(+nilai), enable, D, Q.
#[derive(Debug, Clone, PartialEq)]
pub struct SirRegister {
    pub name: Symbol,
    /// Nilai data (dari D-logic: MUX/ADD/... hasil lowering).
    pub d: ValueId,
    /// Nilai output Q — selalu `SirValue::Reg(register_id)`.
    pub q: ValueId,
    /// Clock.
    pub clock: ValueId,
    pub reset: Option<ResetSpec>,
    /// Clock enable (`iff` — hanya signal tunggal; kompleks → None di fase 1).
    pub enable: Option<ValueId>,
    pub width: usize,
}

/// Sumber nilai (isi slot `values[ValueId]`).
///
/// Catatan: wire bernama (`SirWire`) TIDAK punya varian nilai sendiri —
/// `SirWire.value` menunjuk ke slot ini (bisa berisi Port/Const/Node/Reg).
/// Ini menjaga satu sumber kebenaran untuk nilai.
#[derive(Debug, Clone, PartialEq)]
pub enum SirValue {
    Port(PortId),
    Const(LogicVec),
    Node(NodeId),
    Reg(RegisterId),
}

/// Modul SIR hasil lowering.
#[derive(Debug, Clone, PartialEq)]
pub struct SirModule {
    pub name: Symbol,
    pub inputs: Vec<SirPort>,
    pub outputs: Vec<SirPort>,
    /// Nilai-nilai (SSA-like): port, konstanta, hasil node, output register.
    /// Semua referensi (`d`, `q`, node inputs, port) adalah indeks ke tabel ini.
    pub values: Vec<SirValue>,
    pub wires: Vec<SirWire>,
    pub nodes: Vec<SirNode>,
    pub registers: Vec<SirRegister>,
    /// Traceability: signal RTL → (file, line, col).
    pub src_map: HashMap<Symbol, (String, usize, usize)>,
}

impl SirModule {
    pub fn new(name: Symbol) -> Self {
        SirModule {
            name,
            inputs: Vec::new(),
            outputs: Vec::new(),
            values: Vec::new(),
            wires: Vec::new(),
            nodes: Vec::new(),
            registers: Vec::new(),
            src_map: HashMap::new(),
        }
    }

    pub fn add_value(&mut self, v: SirValue) -> ValueId {
        let id = self.values.len();
        self.values.push(v);
        id
    }

    pub fn add_wire(&mut self, name: Symbol, width: usize, value: ValueId) -> WireId {
        let id = self.wires.len();
        self.wires.push(SirWire {
            name,
            width,
            value,
            is_clock: false,
            is_reset: false,
            is_io: false,
        });
        id
    }

    pub fn add_node(&mut self, kind: SirNodeKind, inputs: Vec<ValueId>, width: usize) -> NodeId {
        let id = self.nodes.len();
        // Output slot = index value BERIKUTNYA (caller wajib menambah value
        // `SirValue::Node(id)` tepat setelah ini).
        let output = self.values.len();
        self.nodes.push(SirNode {
            kind,
            inputs,
            width,
            output,
            src: None,
        });
        id
    }

    /// Lebar nilai (untuk laporan/estimasi).
    pub fn value_width(&self, v: ValueId) -> usize {
        match &self.values[v] {
            SirValue::Port(p) => self.ports()[*p].width,
            SirValue::Const(lv) => lv.width,
            SirValue::Node(n) => self.nodes[*n].width,
            SirValue::Reg(r) => self.registers[*r].width,
        }
    }

    fn ports(&self) -> Vec<&SirPort> {
        // Helper kecil — port dipisah input/output di model; akses langsung
        // lewat `self.inputs`/`self.outputs` untuk keperluan biasa.
        self.inputs.iter().chain(self.outputs.iter()).collect()
    }

    /// Jumlah node logika kombinasi.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Jumlah register.
    pub fn register_count(&self) -> usize {
        self.registers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_width_resolves_across_kinds() {
        let mut m = SirModule::new(Symbol::intern("top"));
        let c = m.add_value(SirValue::Const(LogicVec::from_u64(7, 3)));
        let n = m.add_node(SirNodeKind::Add, vec![c, c], 4);
        let v = m.add_value(SirValue::Node(n));
        assert_eq!(m.value_width(c), 3);
        assert_eq!(m.value_width(v), 4);
        assert_eq!(m.node_count(), 1);
    }

    #[test]
    fn register_model_is_flat_not_verilog() {
        // SirRegister hanya menyimpan clock/reset/enable/d/q — bukan
        // always_ff/if/else (aturan SIR: jauh dari Verilog).
        let mut m = SirModule::new(Symbol::intern("top"));
        let clk = m.add_value(SirValue::Const(LogicVec::from_u64(0, 1)));
        let d = m.add_value(SirValue::Const(LogicVec::from_u64(0, 8)));
        let q = m.add_value(SirValue::Reg(0));
        m.registers.push(SirRegister {
            name: Symbol::intern("count"),
            d,
            q,
            clock: clk,
            reset: Some(ResetSpec {
                signal: clk,
                value: LogicVec::from_u64(0, 8),
                polarity: false,
                r#async: true,
            }),
            enable: None,
            width: 8,
        });
        assert_eq!(m.register_count(), 1);
        assert_eq!(m.value_width(q), 8);
    }
}
