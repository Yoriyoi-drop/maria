//! Optimisasi SIR (SYNTHESIS.md §6–§7).
//!
//! Pass murni `SIR → SIR`, technology-agnostic. Semua rewrite memakai
//! invariant `SirNode.output`: menimpa `values[node.output]` mengubah hasil
//! node UNTUK SEMUA pengguna (ValueId adalah indeks), sehingga folding/alias
//! tidak perlu menelusuri ulang referensi.

mod arith;
mod const_fold;
mod cse;
mod dce;
mod mux;

pub use arith::ArithmeticSimplify;
pub use const_fold::ConstFold;
pub use cse::Cse;
pub use dce::Dce;
pub use mux::MuxSimplify;

use maria_core::LogicVal;
use maria_core::LogicVec;
use maria_sir::{SirModule, SirNodeKind, SirValue, ValueId};

/// Mask u64 untuk lebar `width` bit.
pub(crate) fn mask(width: usize) -> u64 {
    if width >= 64 {
        u64::MAX
    } else if width == 0 {
        0
    } else {
        (1u64 << width) - 1
    }
}

/// Nilai u64 dari LogicVec bila SEMUA bit 2-state (0/1). X/Z → None
/// (folding 4-state aman di-skip, bukan disalah-fold).
pub(crate) fn u64_known(lv: &LogicVec) -> Option<u64> {
    if lv
        .bits
        .iter()
        .all(|b| matches!(b, LogicVal::Zero | LogicVal::One))
    {
        Some(lv.to_u64() & mask(lv.width))
    } else {
        None
    }
}

/// Resolve nilai ke konstanta (transparan lewat `Buffer`).
pub(crate) fn resolve_const(m: &SirModule, vid: ValueId) -> Option<LogicVec> {
    match &m.values[vid] {
        SirValue::Const(lv) => Some(lv.clone()),
        SirValue::Node(nid) if m.nodes[*nid].kind == SirNodeKind::Buffer => {
            resolve_const(m, m.nodes[*nid].inputs[0])
        }
        _ => None,
    }
}

/// Konten slot nilai (alias untuk identity rewrite).
pub(crate) fn alias_content(m: &SirModule, vid: ValueId) -> SirValue {
    m.values[vid].clone()
}

/// Node masih hidup? (`values[output]` masih memuat `Node(nid)`.)
pub(crate) fn node_alive(m: &SirModule, nid: usize) -> bool {
    let out = m.nodes[nid].output;
    matches!(m.values.get(out), Some(SirValue::Node(x)) if *x == nid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_and_u64_known() {
        assert_eq!(mask(8), 0xff);
        assert_eq!(mask(64), u64::MAX);
        let ok = LogicVec::from_u64(0b1010, 4);
        assert_eq!(u64_known(&ok), Some(0b1010));
        let x = LogicVec::fill(LogicVal::X, 4);
        assert_eq!(u64_known(&x), None);
    }
}
