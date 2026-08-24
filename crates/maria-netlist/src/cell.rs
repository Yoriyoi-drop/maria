//! Sel netlist generik + instance (SYNTHESIS.md §11).
//!
//! Sel adalah primitif teknologi-agnostik hasil mapping SIR (bukan sel
//! library ASIC/FPGA — itu tanggung jawab `maria-tech`). Satu node SIR →
//! satu sel bit-vector (`width`); bit-slicing ke LUT/gate per-bit menyusul
//! di phase 4/7.

use maria_core::intern::Symbol;

pub type CellId = usize;

/// Referensi pin: sel mana + nama pin apa (dipakai net.driver / net.loads).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinRef {
    pub cell: CellId,
    pub pin: String,
}

/// Koneksi pin → net pada sebuah instance sel.
///
/// `bit: Some(i)` berarti koneksi ke bit `i` dari net (LUT/carry memakai
/// bit-select `.I0(a[3])`); `None` = seluruh net (net scalar/vector utuh).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinConn {
    pub net: crate::net::NetId,
    pub pin: String,
    pub bit: Option<usize>,
}

impl PinConn {
    /// Koneksi seluruh net.
    pub fn whole(net: crate::net::NetId, pin: impl Into<String>) -> Self {
        PinConn {
            net,
            pin: pin.into(),
            bit: None,
        }
    }

    /// Koneksi bit `i` dari net (dipakai LUT/CARRY4).
    pub fn bit(net: crate::net::NetId, pin: impl Into<String>, i: usize) -> Self {
        PinConn {
            net,
            pin: pin.into(),
            bit: Some(i),
        }
    }
}

/// Jenis sel generik (teknologi-agnostik).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellKind {
    // ── Boolean —─
    And,
    Or,
    Xor,
    Not,
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
    Sar,
    // ── Perbandingan (output 1 bit) —─
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    // ── Reduksi (output 1 bit) —─
    ReduceAnd,
    ReduceOr,
    ReduceXor,
    // ── Bit-vector —─
    Concat,
    Slice {
        msb: usize,
        lsb: usize,
    },
    // ── Buffer / I/O —─
    Buffer,
    TriState,
    // ── Register —─
    Dff,
    DffE,
    DffR {
        reset_value: u64,
        polarity: bool,
        r#async: bool,
    },
    DffRE {
        reset_value: u64,
        polarity: bool,
        r#async: bool,
    },
    // ── Teknologi (phase 4, maria-tech) —─
    /// LUT6 dengan truth table `init` (64 bit). Input yang tidak terpakai
    /// di-tie ke `1'b0` (pola FPGA — LUT selalu 6 input).
    Lut {
        init: u64,
    },
    /// Carry chain 4-bit (CARRY4 — adder ripple).
    Carry4,
}

impl CellKind {
    /// Nama sel (dipakai emit Verilog & report).
    pub fn cell_name(&self) -> String {
        match self {
            CellKind::And => "_AND2".into(),
            CellKind::Or => "_OR2".into(),
            CellKind::Xor => "_XOR2".into(),
            CellKind::Not => "_NOT".into(),
            CellKind::Mux => "_MUX2".into(),
            CellKind::Add => "_ADD".into(),
            CellKind::Sub => "_SUB".into(),
            CellKind::Mul => "_MUL".into(),
            CellKind::Div => "_DIV".into(),
            CellKind::Mod => "_MOD".into(),
            CellKind::Shl => "_SHL".into(),
            CellKind::Shr => "_SHR".into(),
            CellKind::Sar => "_SAR".into(),
            CellKind::Eq => "_EQ".into(),
            CellKind::Ne => "_NE".into(),
            CellKind::Lt => "_LT".into(),
            CellKind::Le => "_LE".into(),
            CellKind::Gt => "_GT".into(),
            CellKind::Ge => "_GE".into(),
            CellKind::ReduceAnd => "_REDAND".into(),
            CellKind::ReduceOr => "_REDOR".into(),
            CellKind::ReduceXor => "_REDXOR".into(),
            CellKind::Concat => "_CONCAT".into(),
            CellKind::Slice { .. } => "_SLICE".into(),
            CellKind::Buffer => "_BUF".into(),
            CellKind::TriState => "_TRI".into(),
            CellKind::Dff => "DFF".into(),
            CellKind::DffE => "DFFE".into(),
            CellKind::DffR { .. } => "DFFR".into(),
            CellKind::DffRE { .. } => "DFFRE".into(),
            CellKind::Lut { .. } => "LUT6".into(),
            CellKind::Carry4 => "CARRY4".into(),
        }
    }

    pub fn is_sequential(&self) -> bool {
        matches!(
            self,
            CellKind::Dff | CellKind::DffE | CellKind::DffR { .. } | CellKind::DffRE { .. }
        )
    }

    /// Kunci modul unik untuk emit library — DFFR/DFFRE menyandikan spesifikasi
    /// reset (nilai/polaritas/async) agar dua FF dengan reset berbeda tidak
    /// bertabrakan pada satu nama modul.
    pub fn module_key(&self) -> String {
        match self {
            CellKind::DffR {
                reset_value,
                polarity,
                r#async,
            } => format!(
                "DFFR_r{:x}_{}_{}",
                reset_value,
                if *polarity { "ah" } else { "al" },
                if *r#async { "a" } else { "s" }
            ),
            CellKind::DffRE {
                reset_value,
                polarity,
                r#async,
            } => format!(
                "DFFRE_r{:x}_{}_{}",
                reset_value,
                if *polarity { "ah" } else { "al" },
                if *r#async { "a" } else { "s" }
            ),
            CellKind::Slice { msb, lsb } => format!("SLICE_{}_{}", msb, lsb),
            // Init disandikan di nama modul: dua LUT dengan truth table berbeda
            // tidak boleh bertabrakan pada satu definisi modul.
            CellKind::Lut { init } => format!("LUT6_{:x}", init),
            _ => self.cell_name(),
        }
    }

    /// Nama pin input dalam urutan koneksi.
    pub fn input_pins(&self) -> Vec<&'static str> {
        match self {
            CellKind::And | CellKind::Or | CellKind::Xor => vec!["a", "b"],
            CellKind::Not | CellKind::ReduceAnd | CellKind::ReduceOr | CellKind::ReduceXor => {
                vec!["a"]
            }
            CellKind::Mux => vec!["s", "a", "b"],
            CellKind::Add
            | CellKind::Sub
            | CellKind::Mul
            | CellKind::Div
            | CellKind::Mod
            | CellKind::Shl
            | CellKind::Shr
            | CellKind::Sar
            | CellKind::Eq
            | CellKind::Ne
            | CellKind::Lt
            | CellKind::Le
            | CellKind::Gt
            | CellKind::Ge => vec!["a", "b"],
            CellKind::Slice { .. } | CellKind::Buffer => vec!["a"],
            CellKind::Concat => vec!["p"],
            CellKind::TriState => vec!["en", "a"],
            CellKind::Dff => vec!["c", "d"],
            CellKind::DffE => vec!["c", "ce", "d"],
            CellKind::DffR { .. } => vec!["c", "r", "d"],
            CellKind::DffRE { .. } => vec!["c", "r", "ce", "d"],
            // LUT6: 6 input pin; unused di-tie ke 0 oleh mapper.
            CellKind::Lut { .. } => vec!["i0", "i1", "i2", "i3", "i4", "i5"],
            CellKind::Carry4 => vec!["ci", "a", "b"],
        }
    }

    /// Nama pin output.
    pub fn output_pins(&self) -> Vec<&'static str> {
        match self {
            CellKind::Lut { .. } => vec!["o"],
            CellKind::Carry4 => vec!["s", "co"],
            _ => {
                if self.is_sequential() {
                    vec!["q"]
                } else {
                    vec!["y"]
                }
            }
        }
    }
}

/// Instance sel dalam netlist.
#[derive(Debug, Clone, PartialEq)]
pub struct CellInstance {
    pub name: Symbol,
    pub kind: CellKind,
    /// Lebar bit-vector OUTPUT sel ini.
    pub width: usize,
    /// Lebar operand data utama (untuk parameter `W` modul generik — untuk
    /// EQ/Reduce, output 1 bit tapi operand bisa lebar; untuk FF = lebar D).
    pub in_width: usize,
    pub inputs: Vec<PinConn>,
    pub outputs: Vec<PinConn>,
}

impl CellInstance {
    pub fn new(name: Symbol, kind: CellKind, width: usize) -> Self {
        CellInstance {
            name,
            kind,
            width,
            in_width: width,
            inputs: Vec::new(),
            outputs: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_kind_flags_and_pins() {
        assert!(CellKind::DffR {
            reset_value: 0,
            polarity: true,
            r#async: true
        }
        .is_sequential());
        assert!(!CellKind::Add.is_sequential());
        assert_eq!(CellKind::Mux.input_pins(), vec!["s", "a", "b"]);
        assert_eq!(CellKind::Mux.output_pins(), vec!["y"]);
        assert_eq!(
            CellKind::DffRE {
                reset_value: 0,
                polarity: false,
                r#async: true
            }
            .input_pins(),
            vec!["c", "r", "ce", "d"]
        );
        assert_eq!(CellKind::DffE.output_pins(), vec!["q"]);
        assert_eq!(CellKind::Add.cell_name(), "_ADD");
    }
}
