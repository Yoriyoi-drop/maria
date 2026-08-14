//! Generic Tech Mapping (SYNTHESIS.md §5/§12 — phase 4).
//!
//! Mapping `SirModule` (sudah lewat pass optimizer) → netlist **teknologi**
//! dengan LUT real + carry chain + FF, BUKAN netlist bit-vector generik
//! fase 3. Logika boolean → LUT6 (truth table `init` dihitung nyata), adder →
//! CARRY4, register → FF per-bit.
//!
//! ```text
//! SirModule ──tech_map──► Netlist (Lut{init} / Carry4 / Dff*)
//!    │                          │
//!    └── bit-blast ──► BitFn ──► LUT cut (≤ K input, init 64-bit)
//!                          │
//!                          └── AIG decomposition (> K input → cascade LUT)
//! ```
//!
//! Algoritma:
//! 1. **Bit-blast**: setiap bit output node SIR → fungsi Boolean `BitFn`
//!    (And/Or/Xor/Not/Mux + konstanta). Aritmetika lewat carry chain.
//! 2. **LUT cut**: cone dengan ≤ `K` leaf (K = `lut_inputs()`, LUT6 → 6)
//!    di-pack jadi SATU LUT; `init` = evaluasi fungsi atas 2^K kombinasi.
//!    Leaf konstanta di-fold ke init (bukan pin).
//! 3. **AIG decomposition**: fungsi > K leaf dipecah struktural — tiap
//!    sub-fungsi → LUT, node atas menggabungkan hasil sub-LUT (≤ 2 input).
//! 4. **Carry chain**: Add/Sub → CARRY4 (`{co,s} = ci + a + b`), Sub via
//!    `a + ~b + 1`. Tanpa carry chain → ripple adder LUT (fallback).
//! 5. **Register** → FF per-bit (`Dff`/`DffE`/`DffR`/`DffRE`).
//!
//! Netlist hasil mapping bersifat **bit-level**: setiap bit internal punya
//! net 1-bit sendiri; port/konstanta tetap net vector (bit-select `.a[3]`
//! via `PinConn.bit`). Deterministik — input sama → output identik.

use std::collections::{HashMap, HashSet};

use maria_core::intern::Symbol;
use maria_core::LogicVal;
use maria_netlist::cell::{CellInstance, CellKind, PinConn};
use maria_netlist::net::{Netlist, NetId, PortDir};
use maria_sir::{ResetSpec, SirModule, SirNodeKind, SirRegister, SirValue};
use maria_tech::TechArch;

/// Sumber 1-bit: net + (opsional) indeks bit di net vector.
#[derive(Debug, Clone, Copy, PartialEq)]
struct OneBit {
    net: NetId,
    bit: Option<usize>,
}

/// Fungsi Boolean bit-level — pohon operasi teknologi-agnostik.
///
/// `Leaf(vid, bit)` = bit `bit` dari nilai SIR `vid` (port / konstanta /
/// register / node lain). Konstanta diwakili `Const` — di-fold ke `init`
/// LUT, bukan pin.
#[derive(Debug, Clone, PartialEq)]
enum BitFn {
    Const(bool),
    Leaf(usize, usize),
    Not(Box<BitFn>),
    And(Box<BitFn>, Box<BitFn>),
    Or(Box<BitFn>, Box<BitFn>),
    Xor(Box<BitFn>, Box<BitFn>),
    Mux(Box<BitFn>, Box<BitFn>, Box<BitFn>),
}

impl BitFn {
    /// Fold konstanta & identitas (bottom-up). Mencegah LUT tak perlu:
    /// `a & 1 → a`, `x & 0 → 0`, `~~a → a`, `x ^ x → 0`, dst.
    fn simplify(self) -> BitFn {
        match self {
            BitFn::Const(_) | BitFn::Leaf(_, _) => self,
            BitFn::Not(a) => match a.simplify() {
                BitFn::Const(c) => BitFn::Const(!c),
                BitFn::Not(inner) => *inner,
                a => BitFn::Not(Box::new(a)),
            },
            BitFn::And(a, b) => {
                let a = a.simplify();
                let b = b.simplify();
                match (&a, &b) {
                    (BitFn::Const(false), _) | (_, BitFn::Const(false)) => BitFn::Const(false),
                    (BitFn::Const(true), _) => b,
                    (_, BitFn::Const(true)) => a,
                    _ => {
                        if a == b {
                            a
                        } else {
                            BitFn::And(Box::new(a), Box::new(b))
                        }
                    }
                }
            }
            BitFn::Or(a, b) => {
                let a = a.simplify();
                let b = b.simplify();
                match (&a, &b) {
                    (BitFn::Const(true), _) | (_, BitFn::Const(true)) => BitFn::Const(true),
                    (BitFn::Const(false), _) => b,
                    (_, BitFn::Const(false)) => a,
                    _ => {
                        if a == b {
                            a
                        } else {
                            BitFn::Or(Box::new(a), Box::new(b))
                        }
                    }
                }
            }
            BitFn::Xor(a, b) => {
                let a = a.simplify();
                let b = b.simplify();
                match (&a, &b) {
                    (BitFn::Const(false), _) => b,
                    (_, BitFn::Const(false)) => a,
                    (BitFn::Const(true), _) => BitFn::Not(Box::new(b)),
                    (_, BitFn::Const(true)) => BitFn::Not(Box::new(a)),
                    (BitFn::Not(an), _) if **an == b => BitFn::Const(true),
                    (_, BitFn::Not(bn)) if **bn == a => BitFn::Const(true),
                    _ => {
                        if a == b {
                            BitFn::Const(false)
                        } else {
                            BitFn::Xor(Box::new(a), Box::new(b))
                        }
                    }
                }
            }
            BitFn::Mux(s, t, f) => {
                let s = s.simplify();
                let t = t.simplify();
                let f = f.simplify();
                match &s {
                    BitFn::Const(true) => t,
                    BitFn::Const(false) => f,
                    _ => {
                        if t == f {
                            t
                        } else {
                            BitFn::Mux(Box::new(s), Box::new(t), Box::new(f))
                        }
                    }
                }
            }
        }
    }

    /// Kumpulkan leaf (vid, bit) — sudah di-resolve ke vid kanonik, dedup,
    /// tanpa `Const`. Bila bukan konstanta → masukkan.
    fn collect(&self, leaves: &mut Vec<(usize, usize)>, seen: &mut HashSet<(usize, usize)>, sir: &SirModule) {
        match self {
            BitFn::Const(_) => {}
            BitFn::Leaf(vid, bit) => {
                let rv = resolve_vid(sir, *vid);
                if seen.insert((rv, *bit)) {
                    leaves.push((rv, *bit));
                }
            }
            BitFn::Not(a) => a.collect(leaves, seen, sir),
            BitFn::And(a, b) | BitFn::Or(a, b) | BitFn::Xor(a, b) => {
                a.collect(leaves, seen, sir);
                b.collect(leaves, seen, sir);
            }
            BitFn::Mux(s, t, f) => {
                s.collect(leaves, seen, sir);
                t.collect(leaves, seen, sir);
                f.collect(leaves, seen, sir);
            }
        }
    }

    /// Evaluasi fungsi atas assignment leaf (None = tidak terpakai).
    fn eval(&self, leaves: &[(usize, usize)], assign: &[Option<bool>], sir: &SirModule) -> bool {
        match self {
            BitFn::Const(c) => *c,
            BitFn::Leaf(vid, bit) => {
                let rv = resolve_vid(sir, *vid);
                let pos = leaves
                    .iter()
                    .position(|&(v, b)| v == rv && b == *bit)
                    .expect("leaf sudah dikumpulkan");
                assign[pos].expect("assignment leaf terisi")
            }
            BitFn::Not(a) => !a.eval(leaves, assign, sir),
            BitFn::And(a, b) => a.eval(leaves, assign, sir) && b.eval(leaves, assign, sir),
            BitFn::Or(a, b) => a.eval(leaves, assign, sir) || b.eval(leaves, assign, sir),
            BitFn::Xor(a, b) => a.eval(leaves, assign, sir) ^ b.eval(leaves, assign, sir),
            BitFn::Mux(s, t, f) => {
                if s.eval(leaves, assign, sir) {
                    t.eval(leaves, assign, sir)
                } else {
                    f.eval(leaves, assign, sir)
                }
            }
        }
    }

    /// Truth table 64-bit: `init[addr]` = f(leaf j := bit j dari addr).
    /// Konvensi Xilinx — leaf 0 → pin i0 (addr bit 0, LSB).
    fn init_table(&self, leaves: &[(usize, usize)], sir: &SirModule) -> u64 {
        let k = leaves.len();
        debug_assert!(k <= 64, "LUT init max 6 input — k={k}");
        let mut init = 0u64;
        let mut assign = vec![None; k];
        for addr in 0usize..(1usize << k.min(6)) {
            for j in 0..k {
                assign[j] = Some(((addr >> j) & 1) == 1);
            }
            if self.eval(leaves, &assign, sir) {
                init |= 1u64 << addr;
            }
        }
        init
    }
}

/// FF yang masih menunggu D-net (wired setelah semua node ter-mapping).
struct PendingFf {
    reg_name: Symbol,
    d: usize,
    clock: usize,
    reset: Option<ResetSpec>,
    enable: Option<usize>,
    width: usize,
    /// Net 1-bit yang di-drive tiap FF bit (w==1 → satu elemen).
    q_bits: Vec<NetId>,
}

/// Hasil tech mapping.
#[derive(Debug, Clone)]
pub struct TechMapResult {
    pub netlist: Netlist,
    /// Jumlah LUT (sel `Lut { init }`).
    pub lut_count: usize,
    /// Jumlah CARRY4 setara 4-bit (`ceil(width/4)` per adder).
    pub carry4_count: usize,
    /// Jumlah FF bit.
    pub ff_count: usize,
    /// Konstruk yang dilewati (jujur, bukan salah-mapping).
    pub skipped: Vec<String>,
}

/// Tech mapper — memegang netlist yang sedang dibangun + register nilai.
struct TechMapper<'a> {
    sir: &'a SirModule,
    nl: Netlist,
    value_net: Vec<Option<NetId>>,
    const_nets: HashMap<(u64, usize), NetId>,
    lut_inputs: usize,
    has_carry: bool,
    lut_counter: usize,
    gen_counter: usize,
    carry_counter: usize,
    lut_count: usize,
    carry_slices: usize,
    pending_ffs: Vec<PendingFf>,
    skipped: Vec<String>,
}

/// Vid → vid kanonik (alias node → output node; alias port → nilai port).
fn resolve_vid(sir: &SirModule, vid: usize) -> usize {
    match &sir.values[vid] {
        SirValue::Node(nid) => sir.nodes[*nid].output,
        SirValue::Port(pid) => {
            if sir
                .inputs
                .iter()
                .chain(sir.outputs.iter())
                .any(|p| p.value == vid)
            {
                return vid;
            }
            for p in sir.inputs.iter().chain(sir.outputs.iter()) {
                if matches!(&sir.values[p.value], SirValue::Port(pp) if *pp == *pid) {
                    return p.value;
                }
            }
            vid
        }
        _ => vid,
    }
}

/// Bit `i` nilai SIR sebagai `BitFn` (konstanta di-fold; di luar lebar → 0).
fn val_bit(sir: &SirModule, vid: usize, i: usize) -> BitFn {
    if let SirValue::Const(lv) = &sir.values[vid] {
        let one = lv.bits.get(i).copied() == Some(LogicVal::One);
        return BitFn::Const(one);
    }
    let w = sir.value_width(vid);
    if i >= w {
        BitFn::Const(false)
    } else {
        BitFn::Leaf(vid, i)
    }
}

/// F40 resubstitution: inline leaf yang merujuk node SEDERHANA menjadi
/// BitFn node tersebut (bit-blast) sehingga cone per-bit bisa di-fuse ke
/// SATU LUT6 (mux chain dari `case` → 1 LUT per bit, bukan 1 LUT per node
/// SIR). Node aritmetika (Add/Sub/Mul/Div/Mod) & shift (Shl/Shr/Sar) TIDAK
/// di-inline — representasinya net (CARRY4 / barrel shift) — begitu juga
/// TriState. Guard depth cegah blow-up eksponensial pada chain dalam.
fn expand_simple(sir: &SirModule, f: &BitFn) -> BitFn {
    expand_simple_depth(sir, f, 8)
}

fn expand_simple_depth(sir: &SirModule, f: &BitFn, depth: usize) -> BitFn {
    if depth == 0 {
        return f.clone();
    }
    match f {
        BitFn::Const(_) => f.clone(),
        BitFn::Leaf(vid, bit) => {
            let rv = resolve_vid(sir, *vid);
            let nid = match &sir.values[rv] {
                SirValue::Node(nid) => *nid,
                _ => return f.clone(),
            };
            let node = &sir.nodes[nid];
            let w = node.width.max(1);
            if *bit >= w {
                return BitFn::Const(false);
            }
            let inl = |vid2: usize, b2: usize, d: usize| {
                expand_simple_depth(sir, &val_bit(sir, vid2, b2), d)
            };
            // Lebar operand untuk compare/reduce (output 1-bit, operand lebar).
            let opw = node
                .inputs
                .iter()
                .map(|&v| sir.value_width(v).max(1))
                .max()
                .unwrap_or(1);
            match &node.kind {
                SirNodeKind::And => BitFn::And(
                    Box::new(inl(node.inputs[0], *bit, depth - 1)),
                    Box::new(inl(node.inputs[1], *bit, depth - 1)),
                )
                .simplify(),
                SirNodeKind::Or => BitFn::Or(
                    Box::new(inl(node.inputs[0], *bit, depth - 1)),
                    Box::new(inl(node.inputs[1], *bit, depth - 1)),
                )
                .simplify(),
                SirNodeKind::Xor => BitFn::Xor(
                    Box::new(inl(node.inputs[0], *bit, depth - 1)),
                    Box::new(inl(node.inputs[1], *bit, depth - 1)),
                )
                .simplify(),
                SirNodeKind::Not => {
                    BitFn::Not(Box::new(inl(node.inputs[0], *bit, depth - 1))).simplify()
                }
                SirNodeKind::Buffer => inl(node.inputs[0], *bit, depth - 1),
                SirNodeKind::Mux => BitFn::Mux(
                    Box::new(inl(node.inputs[0], 0, depth - 1)),
                    Box::new(inl(node.inputs[1], *bit, depth - 1)),
                    Box::new(inl(node.inputs[2], *bit, depth - 1)),
                )
                .simplify(),
                SirNodeKind::Eq => eq_fn(sir, node.inputs[0], node.inputs[1], opw),
                SirNodeKind::Ne => {
                    let mut acc = BitFn::Const(false);
                    for k in 0..opw {
                        let x = BitFn::Xor(
                            Box::new(val_bit(sir, node.inputs[0], k)),
                            Box::new(val_bit(sir, node.inputs[1], k)),
                        );
                        acc = BitFn::Or(Box::new(acc), Box::new(x)).simplify();
                    }
                    acc
                }
                SirNodeKind::Lt => lt_fn(sir, node.inputs[0], node.inputs[1], opw),
                SirNodeKind::Le => BitFn::Or(
                    Box::new(lt_fn(sir, node.inputs[0], node.inputs[1], opw)),
                    Box::new(eq_fn(sir, node.inputs[0], node.inputs[1], opw)),
                )
                .simplify(),
                SirNodeKind::Gt => BitFn::Not(Box::new(BitFn::Or(
                    Box::new(lt_fn(sir, node.inputs[0], node.inputs[1], opw)),
                    Box::new(eq_fn(sir, node.inputs[0], node.inputs[1], opw)),
                )))
                .simplify(),
                SirNodeKind::Ge => {
                    BitFn::Not(Box::new(lt_fn(sir, node.inputs[0], node.inputs[1], opw)))
                        .simplify()
                }
                SirNodeKind::ReduceAnd | SirNodeKind::ReduceOr | SirNodeKind::ReduceXor => {
                    reduce_fn(sir, node.inputs[0], opw, &node.kind)
                }
                SirNodeKind::Concat => {
                    let mut offset = 0usize;
                    for k in (0..node.inputs.len()).rev() {
                        let kw = sir.value_width(node.inputs[k]);
                        if *bit < offset + kw {
                            return inl(node.inputs[k], *bit - offset, depth - 1);
                        }
                        offset += kw;
                    }
                    BitFn::Const(false)
                }
                SirNodeKind::Slice { msb, .. } => {
                    if *msb >= *bit {
                        inl(node.inputs[0], *msb - *bit, depth - 1)
                    } else {
                        BitFn::Const(false)
                    }
                }
                _ => f.clone(), // aritmetika/shift/tristate → net (Leaf)
            }
        }
        BitFn::Not(a) => BitFn::Not(Box::new(expand_simple_depth(sir, a, depth - 1))),
        BitFn::And(a, b) => BitFn::And(
            Box::new(expand_simple_depth(sir, a, depth - 1)),
            Box::new(expand_simple_depth(sir, b, depth - 1)),
        ),
        BitFn::Or(a, b) => BitFn::Or(
            Box::new(expand_simple_depth(sir, a, depth - 1)),
            Box::new(expand_simple_depth(sir, b, depth - 1)),
        ),
        BitFn::Xor(a, b) => BitFn::Xor(
            Box::new(expand_simple_depth(sir, a, depth - 1)),
            Box::new(expand_simple_depth(sir, b, depth - 1)),
        ),
        BitFn::Mux(s, t, f2) => BitFn::Mux(
            Box::new(expand_simple_depth(sir, s, depth - 1)),
            Box::new(expand_simple_depth(sir, t, depth - 1)),
            Box::new(expand_simple_depth(sir, f2, depth - 1)),
        ),
    }
}

/// Equality bit: `a_i == b_i`.
fn eq_bit(sir: &SirModule, a: usize, b: usize, i: usize) -> BitFn {
    BitFn::Not(Box::new(BitFn::Xor(Box::new(val_bit(sir, a, i)), Box::new(val_bit(sir, b, i)))))
        .simplify()
}

/// Ripple less-than (unsigned): `a < b`.
fn lt_fn(sir: &SirModule, a: usize, b: usize, w: usize) -> BitFn {
    let mut acc = BitFn::Const(false);
    for i in (0..w).rev() {
        let ai = val_bit(sir, a, i);
        let bi = val_bit(sir, b, i);
        let lt = BitFn::And(Box::new(BitFn::Not(Box::new(ai.clone()))), Box::new(bi.clone())).simplify();
        let eq = eq_bit(sir, a, b, i);
        acc = BitFn::Mux(Box::new(eq), Box::new(acc), Box::new(lt)).simplify();
    }
    acc
}

/// Equality vector: `a == b` (AND atas semua bit eq).
fn eq_fn(sir: &SirModule, a: usize, b: usize, w: usize) -> BitFn {
    let mut acc = BitFn::Const(true);
    for i in 0..w {
        let e = eq_bit(sir, a, b, i);
        acc = BitFn::And(Box::new(acc), Box::new(e)).simplify();
    }
    acc
}

/// Reduction: chain bitwise.
fn reduce_fn(sir: &SirModule, vid: usize, w: usize, kind: &SirNodeKind) -> BitFn {
    let init = match kind {
        SirNodeKind::ReduceAnd => true,
        _ => false,
    };
    let mut acc = BitFn::Const(init);
    for i in 0..w {
        let b = val_bit(sir, vid, i);
        acc = match kind {
            SirNodeKind::ReduceAnd => BitFn::And(Box::new(acc), Box::new(b)),
            SirNodeKind::ReduceOr => BitFn::Or(Box::new(acc), Box::new(b)),
            _ => BitFn::Xor(Box::new(acc), Box::new(b)),
        }
        .simplify();
    }
    acc
}

impl<'a> TechMapper<'a> {
    fn new(sir: &'a SirModule, arch: &dyn TechArch) -> Self {
        TechMapper {
            sir,
            nl: Netlist::new(sir.name),
            value_net: vec![None; sir.values.len()],
            const_nets: HashMap::new(),
            lut_inputs: arch.lut_inputs().max(1),
            has_carry: arch.has_carry_chain(),
            lut_counter: 0,
            gen_counter: 0,
            carry_counter: 0,
            lut_count: 0,
            carry_slices: 0,
            pending_ffs: Vec::new(),
            skipped: Vec::new(),
        }
    }

    /// Net konstanta `(nilai, lebar)` — dipool supaya dua pemakaian nilai
    /// sama berbagi satu net (CSE-level).
    fn const_net(&mut self, val: u64, w: usize) -> NetId {
        let key = (val, w);
        if let Some(&n) = self.const_nets.get(&key) {
            return n;
        }
        let mask = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let name = Symbol::intern(&format!("tie_{:x}_{}", val & mask, w));
        let net = self.nl.add_net(name, w);
        self.nl.nets[net].const_value = Some(val & mask);
        self.const_nets.insert(key, net);
        net
    }

    fn tie_const(&mut self, val: bool) -> OneBit {
        OneBit { net: self.const_net(val as u64, 1), bit: None }
    }

    /// Net nilai SIR — materialisasi konstanta bila belum ada.
    fn net_of(&mut self, vid: usize) -> NetId {
        let rv = resolve_vid(self.sir, vid);
        if let Some(n) = self.value_net[rv] {
            return n;
        }
        if let SirValue::Const(lv) = &self.sir.values[rv] {
            let n = self.const_net(lv.to_u64(), lv.width);
            self.value_net[rv] = Some(n);
            return n;
        }
        let n = self.tie_const(false).net;
        self.value_net[rv] = Some(n);
        n
    }

    /// Sumber bit nilai SIR untuk koneksi pin (bit-select otomatis).
    fn bit_of(&mut self, vid: usize, bit: usize) -> OneBit {
        let net = self.net_of(vid);
        let w = self.sir.value_width(resolve_vid(self.sir, vid));
        if w <= 1 {
            OneBit { net, bit: None }
        } else {
            OneBit { net, bit: Some(bit) }
        }
    }

    /// Net 1-bit hasil LUT (`init` atas input yang diberikan).
    fn emit_lut(&mut self, init: u64, inputs: Vec<OneBit>) -> OneBit {
        let name = format!("lut{}", self.lut_counter);
        let out = self.nl.add_net(Symbol::intern(&name), 1);
        let mut pins = Vec::with_capacity(self.lut_inputs);
        for (i, inp) in inputs.iter().enumerate() {
            if i >= self.lut_inputs {
                break;
            }
            pins.push(PinConn { net: inp.net, pin: format!("i{}", i), bit: inp.bit });
        }
        let tie = self.const_net(0, 1);
        for i in inputs.len().min(self.lut_inputs)..self.lut_inputs {
            pins.push(PinConn { net: tie, pin: format!("i{}", i), bit: None });
        }
        let mut cell = CellInstance::new(Symbol::intern(&name), CellKind::Lut { init }, 1);
        cell.in_width = 1;
        cell.inputs = pins;
        cell.outputs = vec![PinConn { net: out, pin: "o".into(), bit: None }];
        self.nl.add_cell(cell);
        self.lut_counter += 1;
        self.lut_count += 1;
        OneBit { net: out, bit: None }
    }

    /// Pack satu `BitFn` → sumber 1-bit (net LUT / leaf / konstanta).
    fn pack_bit(&mut self, f: &BitFn) -> OneBit {
        match f {
            BitFn::Const(c) => return self.tie_const(*c),
            BitFn::Leaf(vid, bit) => return self.bit_of(*vid, *bit),
            _ => {}
        }
        // F40 resubstitution: coba fuse cone per-bit — inline leaf yang
        // merujuk node SEDERHANA (And/Or/Xor/Not/Mux/compare/reduce/buffer)
        // sehingga mux chain dari `case` jadi SATU LUT6 per bit. Aritmetika/
        // shift tetap net (CARRY4/barrel). Cone yang membengkak > K input →
        // fallback ke f asli (per-node, AIG di bawah) — menghindari blow-up
        // (mis. `count == 99` 8-bit tetap jadi EQ terpisah, bukan inline).
        let expanded = expand_simple(self.sir, f);
        let mut leaves = Vec::new();
        let mut seen = HashSet::new();
        expanded.collect(&mut leaves, &mut seen, self.sir);
        leaves.sort_unstable();
        if leaves.len() <= self.lut_inputs {
            let init = expanded.init_table(&leaves, self.sir);
            let inputs: Vec<OneBit> = leaves.iter().map(|&(v, b)| self.bit_of(v, b)).collect();
            return self.emit_lut(init, inputs);
        }
        // Fallback: pack f asli (tanpa expand — cone per-node).
        let mut leaves = Vec::new();
        let mut seen = HashSet::new();
        f.collect(&mut leaves, &mut seen, self.sir);
        leaves.sort_unstable();
        if leaves.len() <= self.lut_inputs {
            let init = f.init_table(&leaves, self.sir);
            let inputs: Vec<OneBit> = leaves.iter().map(|&(v, b)| self.bit_of(v, b)).collect();
            return self.emit_lut(init, inputs);
        }
        // AIG decomposition: sub-fungsi → LUT, node atas gabungkan hasilnya
        // (tiap LUT kombinasi ≤ 2 input, kecuali Mux 3).
        match f {
            BitFn::Not(a) => {
                let ai = self.pack_bit(a);
                self.emit_lut(0x1, vec![ai])
            }
            BitFn::And(a, b) => {
                let ai = self.pack_bit(a);
                let bi = self.pack_bit(b);
                self.emit_lut(0x8, vec![ai, bi])
            }
            BitFn::Or(a, b) => {
                let ai = self.pack_bit(a);
                let bi = self.pack_bit(b);
                self.emit_lut(0xE, vec![ai, bi])
            }
            BitFn::Xor(a, b) => {
                let ai = self.pack_bit(a);
                let bi = self.pack_bit(b);
                self.emit_lut(0x6, vec![ai, bi])
            }
            BitFn::Mux(s, t, f2) => {
                let si = self.pack_bit(s);
                let ti = self.pack_bit(t);
                let fi = self.pack_bit(f2);
                // pins i0=s i1=t i2=f; y = s?t:f → init 0xD8
                self.emit_lut(0xD8, vec![si, ti, fi])
            }
            BitFn::Const(_) | BitFn::Leaf(_, _) => unreachable!("konstanta/leaf ditangani di atas"),
        }
    }

    /// Gabung bit sources MSB-first → net vector lebar `w` (Concat).
    fn concat_bits(&mut self, name: &str, w: usize, bits: Vec<OneBit>) -> NetId {
        let net = self.nl.add_net(Symbol::intern(name), w);
        let mut cell = CellInstance::new(Symbol::intern(name), CellKind::Concat, w);
        cell.in_width = w;
        cell.inputs = (0..w)
            .map(|i| {
                let b = &bits[w - 1 - i];
                PinConn { net: b.net, pin: format!("p{}", i), bit: b.bit }
            })
            .collect();
        cell.outputs = vec![PinConn { net, pin: "y".into(), bit: None }];
        self.nl.add_cell(cell);
        net
    }

    /// Zero-extend nilai ke lebar `w` (slice bila nilai lebih lebar).
    fn extend_to(&mut self, vid: usize, w: usize) -> NetId {
        let rv = resolve_vid(self.sir, vid);
        let cur = self.net_of(rv);
        let cw = self.sir.value_width(rv);
        if cw == w {
            return cur;
        }
        let name = Symbol::intern(&format!("g{}", self.gen_counter));
        self.gen_counter += 1;
        if cw < w {
            let pad = self.const_net(0, w - cw);
            let ext = self.nl.add_net(name, w);
            let mut cell = CellInstance::new(name, CellKind::Concat, w);
            cell.in_width = w;
            cell.inputs = vec![
                PinConn { net: pad, pin: "p0".into(), bit: None },
                PinConn { net: cur, pin: "p1".into(), bit: None },
            ];
            cell.outputs = vec![PinConn { net: ext, pin: "y".into(), bit: None }];
            self.nl.add_cell(cell);
            ext
        } else {
            let slice = self.nl.add_net(name, w);
            let mut cell = CellInstance::new(
                name,
                CellKind::Slice { msb: w - 1, lsb: 0 },
                w,
            );
            cell.in_width = cw;
            cell.inputs = vec![PinConn { net: cur, pin: "a".into(), bit: None }];
            cell.outputs = vec![PinConn { net: slice, pin: "y".into(), bit: None }];
            self.nl.add_cell(cell);
            slice
        }
    }

    /// Shift kiri net lebar `w` sejauh `j` (bit di bawah → 0).
    fn shift_net(&mut self, src: NetId, j: usize, w: usize) -> NetId {
        if j == 0 {
            return src;
        }
        let name = Symbol::intern(&format!("g{}", self.gen_counter));
        self.gen_counter += 1;
        let tie = self.const_net(0, 1);
        let bits: Vec<OneBit> = (0..w)
            .map(|i| {
                if i >= j {
                    OneBit { net: src, bit: Some(i - j) }
                } else {
                    OneBit { net: tie, bit: None }
                }
            })
            .collect();
        self.concat_bits(name.as_str(), w, bits)
    }

    /// Net CARRY4 `a + b` (ci=0), lebar `w`.
    fn add_nets(&mut self, a: NetId, b: NetId, w: usize) -> NetId {
        let name = format!("carry{}", self.carry_counter);
        self.carry_counter += 1;
        let s = self.nl.add_net(Symbol::intern(&format!("{}_s", name)), w);
        let co = self.nl.add_net(Symbol::intern(&format!("{}_co", name)), 1);
        let mut cell = CellInstance::new(Symbol::intern(&name), CellKind::Carry4, w);
        cell.in_width = w;
        cell.inputs = vec![
            PinConn { net: self.const_net(0, 1), pin: "ci".into(), bit: None },
            PinConn { net: a, pin: "a".into(), bit: None },
            PinConn { net: b, pin: "b".into(), bit: None },
        ];
        cell.outputs = vec![
            PinConn { net: s, pin: "s".into(), bit: None },
            PinConn { net: co, pin: "co".into(), bit: None },
        ];
        self.nl.add_cell(cell);
        self.carry_slices += (w + 3) / 4;
        s
    }

    /// Add/Sub via carry chain CARRY4 (Sub = `a + ~b + 1`).
    fn map_carry_add(&mut self, nid: usize, sub: bool) {
        let node = &self.sir.nodes[nid];
        let w = node.width.max(1);
        let a_vid = node.inputs[0];
        let b_vid = node.inputs[1];
        let a_ext = self.extend_to(a_vid, w);
        let b_in = if sub {
            self.invert_ext(b_vid, w)
        } else {
            self.extend_to(b_vid, w)
        };
        let name = format!("carry{}", self.carry_counter);
        self.carry_counter += 1;
        let s = self.nl.add_net(Symbol::intern(&format!("n{}", nid)), w);
        let co = self.nl.add_net(Symbol::intern(&format!("n{}_co", nid)), 1);
        let mut cell = CellInstance::new(Symbol::intern(&name), CellKind::Carry4, w);
        cell.in_width = w;
        cell.inputs = vec![
            PinConn { net: self.const_net(sub as u64, 1), pin: "ci".into(), bit: None },
            PinConn { net: a_ext, pin: "a".into(), bit: None },
            PinConn { net: b_in, pin: "b".into(), bit: None },
        ];
        cell.outputs = vec![
            PinConn { net: s, pin: "s".into(), bit: None },
            PinConn { net: co, pin: "co".into(), bit: None },
        ];
        self.nl.add_cell(cell);
        self.carry_slices += (w + 3) / 4;
        self.value_net[node.output] = Some(s);
    }

    /// Inversi per-bit (`~x`) lebar `w` → net vector (via LUT NOT).
    fn invert_ext(&mut self, vid: usize, w: usize) -> NetId {
        let ext = self.extend_to(vid, w);
        let name = Symbol::intern(&format!("g{}", self.gen_counter));
        self.gen_counter += 1;
        let mut bits = Vec::with_capacity(w);
        for i in 0..w {
            let out = self.emit_lut(0x1, vec![OneBit { net: ext, bit: Some(i) }]);
            bits.push(out);
        }
        self.concat_bits(name.as_str(), w, bits)
    }

    /// Add/Sub via ripple adder LUT (device tanpa carry chain).
    fn map_ripple_add(&mut self, nid: usize, sub: bool) {
        let node = &self.sir.nodes[nid];
        let w = node.width.max(1);
        let a = node.inputs[0];
        let b = node.inputs[1];
        let mut carry = BitFn::Const(sub);
        let mut sum_bits = Vec::with_capacity(w);
        for i in 0..w {
            let ai = val_bit(self.sir, a, i);
            let bi0 = val_bit(self.sir, b, i);
            let bi = if sub {
                BitFn::Not(Box::new(bi0)).simplify()
            } else {
                bi0
            };
            let axorb = BitFn::Xor(Box::new(ai.clone()), Box::new(bi.clone())).simplify();
            let s = BitFn::Xor(Box::new(axorb.clone()), Box::new(carry.clone())).simplify();
            let c_and = BitFn::And(Box::new(ai), Box::new(bi)).simplify();
            let c_car = BitFn::And(Box::new(carry), Box::new(axorb)).simplify();
            carry = BitFn::Or(Box::new(c_and), Box::new(c_car)).simplify();
            sum_bits.push(s);
        }
        self.map_node_bits(nid, sum_bits);
    }

    /// Multiplikasi via shift-add (dekomposisi AIG): `a*b = Σ b[j]·(a<<j)`.
    fn map_mul(&mut self, nid: usize) {
        let node = &self.sir.nodes[nid];
        let w = node.width.max(1);
        let a_vid = node.inputs[0];
        let b_vid = node.inputs[1];
        let b_rv = resolve_vid(self.sir, b_vid);
        let a_ext = self.extend_to(a_vid, w);
        let b_const = match &self.sir.values[b_rv] {
            SirValue::Const(lv) => Some(lv.bits.clone()),
            _ => None,
        };
        let mut acc: Option<NetId> = None;
        for j in 0..w {
            let bj = b_const
                .as_ref()
                .map(|bits| bits.get(j).copied() == Some(LogicVal::One));
            if bj == Some(false) {
                continue;
            }
            let shifted = self.shift_net(a_ext, j, w);
            let partial = if bj == Some(true) {
                shifted
            } else {
                // partial bit i = b[j] & (a<<j)[i] → 1 LUT AND per bit.
                let sel = self.bit_of(b_vid, j);
                let mut bits = Vec::with_capacity(w);
                for i in 0..w {
                    let out = self.emit_lut(0x8, vec![sel, OneBit { net: shifted, bit: Some(i) }]);
                    bits.push(out);
                }
                self.concat_bits(&format!("g{}", self.gen_counter), w, bits)
            };
            self.gen_counter += 1;
            acc = Some(match acc {
                None => partial,
                Some(prev) => self.add_nets(prev, partial, w),
            });
        }
        let out = acc.unwrap_or_else(|| self.const_net(0, w));
        self.value_net[node.output] = Some(out);
    }

    /// Map node biasa (boolean/seleksi/compare/reduce/shift) → net nilai.
    fn map_regular(&mut self, nid: usize) {
        let node = &self.sir.nodes[nid];
        let w = node.width.max(1);
        let fns: Vec<BitFn> = (0..w)
            .map(|i| self.build_node_bit(node, i))
            .map(BitFn::simplify)
            .collect();
        self.map_node_bits(nid, fns);
    }

    /// `BitFn` untuk bit `i` output node (per-bit ops) / seluruh fungsi
    /// (compare/reduce output 1-bit).
    fn build_node_bit(&self, node: &maria_sir::SirNode, i: usize) -> BitFn {
        let sir = self.sir;
        let w = node.width.max(1);
        // Compare/reduce output 1-bit — lebar operand (bukan lebar output).
        let opw = sir
            .value_width(node.inputs[0])
            .max(sir.value_width(node.inputs.get(1).copied().unwrap_or(node.inputs[0])));
        let a = |i: usize| val_bit(sir, node.inputs[0], i);
        let b = |i: usize| val_bit(sir, node.inputs[1], i);
        match &node.kind {
            SirNodeKind::And => BitFn::And(Box::new(a(i)), Box::new(b(i))),
            SirNodeKind::Or => BitFn::Or(Box::new(a(i)), Box::new(b(i))),
            SirNodeKind::Xor => BitFn::Xor(Box::new(a(i)), Box::new(b(i))),
            SirNodeKind::Not => BitFn::Not(Box::new(a(i))),
            SirNodeKind::Mux => {
                let s = val_bit(sir, node.inputs[0], 0);
                let t = val_bit(sir, node.inputs[1], i);
                let f = val_bit(sir, node.inputs[2], i);
                BitFn::Mux(Box::new(s), Box::new(t), Box::new(f))
            }
            SirNodeKind::Eq => eq_fn(sir, node.inputs[0], node.inputs[1], opw),
            SirNodeKind::Ne => {
                let mut acc = BitFn::Const(false);
                for k in 0..opw {
                    let x = BitFn::Xor(Box::new(val_bit(sir, node.inputs[0], k)), Box::new(val_bit(sir, node.inputs[1], k)));
                    acc = BitFn::Or(Box::new(acc), Box::new(x)).simplify();
                }
                acc
            }
            SirNodeKind::Lt => lt_fn(sir, node.inputs[0], node.inputs[1], opw),
            SirNodeKind::Le => {
                BitFn::Or(Box::new(lt_fn(sir, node.inputs[0], node.inputs[1], opw)), Box::new(eq_fn(sir, node.inputs[0], node.inputs[1], opw)))
                    .simplify()
            }
            SirNodeKind::Gt => BitFn::Not(Box::new(BitFn::Or(
                Box::new(lt_fn(sir, node.inputs[0], node.inputs[1], opw)),
                Box::new(eq_fn(sir, node.inputs[0], node.inputs[1], opw)),
            )))
            .simplify(),
            SirNodeKind::Ge => BitFn::Not(Box::new(lt_fn(sir, node.inputs[0], node.inputs[1], opw))).simplify(),
            SirNodeKind::ReduceAnd | SirNodeKind::ReduceOr | SirNodeKind::ReduceXor => {
                reduce_fn(sir, node.inputs[0], opw, &node.kind)
            }
            SirNodeKind::Shl | SirNodeKind::Shr | SirNodeKind::Sar => {
                let bits = self.barrel_shift(node);
                bits[i].clone()
            }
            SirNodeKind::Concat => {
                let mut offset = 0usize;
                for k in (0..node.inputs.len()).rev() {
                    let kw = sir.value_width(node.inputs[k]);
                    if i < offset + kw {
                        return val_bit(sir, node.inputs[k], i - offset);
                    }
                    offset += kw;
                }
                BitFn::Const(false)
            }
            SirNodeKind::Slice { msb, .. } => {
                let src = node.inputs[0];
                if *msb >= i {
                    val_bit(sir, src, *msb - i)
                } else {
                    BitFn::Const(false)
                }
            }
            SirNodeKind::Buffer => val_bit(sir, node.inputs[0], i),
            SirNodeKind::Add | SirNodeKind::Sub | SirNodeKind::Mul | SirNodeKind::Div | SirNodeKind::Mod | SirNodeKind::TriState => {
                BitFn::Const(false)
            }
        }
    }

    /// Barrel shift (variable amount): Mux tree per bit, amount bit j → shift
    /// 2^j. Amount konstanta di-fold → wiring murni (tanpa LUT).
    fn barrel_shift(&self, node: &maria_sir::SirNode) -> Vec<BitFn> {
        let w = node.width.max(1);
        let src = node.inputs[0];
        let amt = node.inputs[1];
        let amt_w = self.sir.value_width(amt);
        let mut bits: Vec<BitFn> = (0..w).map(|i| val_bit(self.sir, src, i)).collect();
        for j in 0..amt_w {
            let sh = 1usize << j;
            let sel = val_bit(self.sir, amt, j);
            let is_const = matches!(&sel, BitFn::Const(_));
            // Snapshot stage sebelumnya — pembacaan `bits[i±sh]` harus dari
            // stage SEBELUM, bukan hasil overwrite stage berjalan.
            let prev = bits.clone();
            let shifted_at = |i: usize| -> BitFn {
                match &node.kind {
                    SirNodeKind::Shl => {
                        if i >= sh {
                            prev[i - sh].clone()
                        } else {
                            BitFn::Const(false)
                        }
                    }
                    SirNodeKind::Shr => {
                        if i + sh < w {
                            prev[i + sh].clone()
                        } else {
                            BitFn::Const(false)
                        }
                    }
                    _ => {
                        if i + sh < w {
                            prev[i + sh].clone()
                        } else {
                            prev[w - 1].clone()
                        }
                    }
                }
            };
            if is_const {
                if matches!(&sel, BitFn::Const(true)) {
                    for i in 0..w {
                        bits[i] = shifted_at(i);
                    }
                }
                continue;
            }
            for i in 0..w {
                bits[i] = BitFn::Mux(Box::new(sel.clone()), Box::new(shifted_at(i)), Box::new(bits[i].clone()));
            }
        }
        bits
    }

    /// Hasilkan net nilai node dari per-bit `BitFn` (concat bit net bila lebar).
    fn map_node_bits(&mut self, nid: usize, fns: Vec<BitFn>) -> NetId {
        let out_vid = self.sir.nodes[nid].output;
        if fns.len() == 1 {
            let packed = self.pack_bit(&fns[0]);
            let net = if let Some(b) = packed.bit {
                // Leaf berupa bit net vector → ekstrak via Buffer.
                let name = Symbol::intern(&format!("g{}", self.gen_counter));
                self.gen_counter += 1;
                let out = self.nl.add_net(name, 1);
                let mut cell = CellInstance::new(name, CellKind::Buffer, 1);
                cell.in_width = 1;
                cell.inputs = vec![PinConn { net: packed.net, pin: "a".into(), bit: Some(b) }];
                cell.outputs = vec![PinConn { net: out, pin: "y".into(), bit: None }];
                self.nl.add_cell(cell);
                out
            } else {
                packed.net
            };
            self.value_net[out_vid] = Some(net);
            return net;
        }
        let bits: Vec<OneBit> = fns.iter().map(|f| self.pack_bit(f)).collect();
        let name = Symbol::intern(&format!("n{}", nid));
        let net = self.concat_bits(name.as_str(), bits.len(), bits);
        self.value_net[out_vid] = Some(net);
        net
    }

    /// Map satu node (dispatch by kind).
    fn map_one_node(&mut self, nid: usize) {
        let node = &self.sir.nodes[nid];
        match &node.kind {
            SirNodeKind::Add => {
                if self.has_carry {
                    self.map_carry_add(nid, false);
                } else {
                    self.map_ripple_add(nid, false);
                }
            }
            SirNodeKind::Sub => {
                if self.has_carry {
                    self.map_carry_add(nid, true);
                } else {
                    self.map_ripple_add(nid, true);
                }
            }
            SirNodeKind::Mul => self.map_mul(nid),
            SirNodeKind::Div | SirNodeKind::Mod | SirNodeKind::TriState => {
                let w = node.width.max(1);
                self.skipped.push(format!(
                    "node n{} ({}) — belum di-map fase 4; nilai di-tie 0",
                    nid,
                    node.kind.name()
                ));
                let tie = self.const_net(0, w);
                self.value_net[node.output] = Some(tie);
            }
            _ => self.map_regular(nid),
        }
    }

    /// Net register Q + FF bit nets (Q = leaf untuk konsumen, dibuat dulu).
    fn make_reg_q(&mut self, r: &SirRegister, w: usize) -> (NetId, Vec<NetId>) {
        if w == 1 {
            let q = self.nl.add_net(Symbol::intern(&format!("{}_q", r.name.as_str())), 1);
            return (q, vec![q]);
        }
        let name = Symbol::intern(&format!("{}_q", r.name.as_str()));
        let mut bits = Vec::with_capacity(w);
        for i in 0..w {
            let b = self.nl.add_net(Symbol::intern(&format!("{}_q{}", r.name.as_str(), i)), 1);
            bits.push(OneBit { net: b, bit: None });
        }
        let bit_nets: Vec<NetId> = bits.iter().map(|b| b.net).collect();
        let q = self.concat_bits(name.as_str(), w, bits);
        (q, bit_nets)
    }

    /// Wire semua FF bit setelah D-net tersedia.
    fn wire_ffs(&mut self) {
        let ffs: Vec<PendingFf> = std::mem::take(&mut self.pending_ffs);
        for ff in ffs {
            let d = self.net_of(ff.d);
            let clk = self.net_of(ff.clock);
            self.nl.nets[clk].is_clock = true;
            let rst = ff.reset.as_ref().map(|rs| self.net_of(rs.signal));
            if let Some(n) = rst {
                self.nl.nets[n].is_reset = true;
            }
            let en = ff.enable.map(|e| self.net_of(e));
            for i in 0..ff.width {
                let kind = ff_kind(&ff, i);
                let name = Symbol::intern(&format!("ff_{}_{}", ff.reg_name.as_str(), i));
                let mut cell = CellInstance::new(name, kind, 1);
                cell.in_width = 1;
                cell.inputs.push(PinConn { net: clk, pin: "c".into(), bit: None });
                if let Some(n) = rst {
                    cell.inputs.push(PinConn { net: n, pin: "r".into(), bit: None });
                }
                if let Some(n) = en {
                    cell.inputs.push(PinConn { net: n, pin: "ce".into(), bit: None });
                }
                let d_pin = if ff.width > 1 {
                    PinConn { net: d, pin: "d".into(), bit: Some(i) }
                } else {
                    PinConn { net: d, pin: "d".into(), bit: None }
                };
                cell.inputs.push(d_pin);
                cell.outputs.push(PinConn {
                    net: ff.q_bits[i],
                    pin: "q".into(),
                    bit: None,
                });
                self.nl.add_cell(cell);
            }
        }
    }

    /// Pipeline mapping lengkap.
    fn map_all(&mut self) {
        // 1. Port input (net vector + slot nilai).
        for p in &self.sir.inputs {
            let net = self.nl.add_net(p.name, p.width);
            self.nl.nets[net].is_io = true;
            self.value_net[resolve_vid(self.sir, p.value)] = Some(net);
            let dir = match p.dir {
                maria_sir::PortDir::Input => PortDir::Input,
                maria_sir::PortDir::Inout => PortDir::Inout,
                maria_sir::PortDir::Output => PortDir::Output,
            };
            self.nl.add_port(p.name, dir, p.width);
        }

        // 2. Register: Q-net (leaf) + pending FF.
        for (rid, r) in self.sir.registers.iter().enumerate() {
            let w = r.width.max(1);
            let (q, q_bits) = self.make_reg_q(r, w);
            self.value_net[r.q] = Some(q);
            self.pending_ffs.push(PendingFf {
                reg_name: r.name,
                d: r.d,
                clock: r.clock,
                reset: r.reset.clone(),
                enable: r.enable,
                width: w,
                q_bits,
            });
            let _ = rid;
        }

        // 3. Node — topological (input nilai harus sudah punya net).
        let mut mapped = vec![false; self.sir.nodes.len()];
        loop {
            let mut progress = false;
            for (nid, node) in self.sir.nodes.iter().enumerate() {
                if mapped[nid] {
                    continue;
                }
                let ready = node.inputs.iter().all(|&v| {
                    let rv = resolve_vid(self.sir, v);
                    matches!(&self.sir.values[rv], SirValue::Const(_)) || self.value_net[rv].is_some()
                });
                if ready {
                    self.map_one_node(nid);
                    mapped[nid] = true;
                    progress = true;
                }
            }
            if !progress {
                break;
            }
        }
        for (nid, node) in self.sir.nodes.iter().enumerate() {
            if !mapped[nid] {
                self.skipped.push(format!(
                    "node n{} ({}) — input tidak ter-mapping; nilai di-tie 0",
                    nid,
                    node.kind.name()
                ));
                let tie = self.const_net(0, node.width.max(1));
                self.value_net[node.output] = Some(tie);
            }
        }

        // 4. Port output: Buffer dari net nilai sumber.
        for p in &self.sir.outputs {
            let src = self.net_of(p.value);
            let port_net = self.nl.add_net(p.name, p.width);
            self.nl.nets[port_net].is_io = true;
            let mut cell = CellInstance::new(
                Symbol::intern(&format!("out_{}", p.name.as_str())),
                CellKind::Buffer,
                p.width.max(1),
            );
            cell.in_width = p.width.max(1);
            cell.inputs = vec![PinConn { net: src, pin: "a".into(), bit: None }];
            cell.outputs = vec![PinConn { net: port_net, pin: "y".into(), bit: None }];
            self.nl.add_cell(cell);
            self.nl.add_port(p.name, PortDir::Output, p.width);
        }

        // 5. FF wiring (butuh D-net hasil node mapping).
        self.wire_ffs();

        // 6. F40 DCE: buang sel kombinasional yang output net-nya tak
        //    terpakai (resubstitution meng-inline node intermediate → net
        //    intermediate dead). Net tak terhubung ikut dibuang agar
        //    verify_dag tetap bersih.
        self.dce_unused();
    }

    /// F40: dead code elimination post-mapping — hapus sel kombinasional
    /// yang SEMUA output net-nya tidak punya load (dan bukan io). Terjadi
    /// karena resubstitution (`expand_simple`) meng-inline node sederhana
    /// (And/Or/Xor/Mux/compare) ke dalam LUT konsumen → net intermediate
    /// tidak pernah dipakai. Sel sequential (FF) tidak pernah dihapus.
    /// Membangun netlist baru (port + net terpakai + sel live) supaya
    /// index sel/net tetap konsisten (1 driver / N loads, verify_dag OK).
    fn dce_unused(&mut self) {
        let cells = std::mem::take(&mut self.nl.cells);
        let nets = std::mem::take(&mut self.nl.nets);
        let mut live_cell = vec![true; cells.len()];
        loop {
            let mut loads = vec![0usize; nets.len()];
            for (cid, c) in cells.iter().enumerate() {
                if !live_cell[cid] {
                    continue;
                }
                for pin in &c.inputs {
                    loads[pin.net] += 1;
                }
            }
            let mut changed = false;
            for (cid, c) in cells.iter().enumerate() {
                if !live_cell[cid] || c.kind.is_sequential() {
                    continue;
                }
                let dead = c.outputs.iter().all(|o| loads[o.net] == 0 && !nets[o.net].is_io);
                if dead {
                    live_cell[cid] = false;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        // Bangun netlist baru: port + net terpakai + sel live (index di-remap).
        let name = self.nl.name;
        let ports = std::mem::take(&mut self.nl.ports);
        let mut new_nl = Netlist::new(name);
        new_nl.ports = ports;
        let mut net_map = vec![usize::MAX; nets.len()];
        for (id, n) in nets.iter().enumerate() {
            let used = live_cell.iter().enumerate().any(|(cid, &l)| {
                l && (cells[cid].inputs.iter().any(|p| p.net == id)
                    || cells[cid].outputs.iter().any(|p| p.net == id))
            });
            if n.is_io || used {
                let nn = new_nl.add_net(n.name, n.width);
                {
                    let nn_ref = &mut new_nl.nets[nn];
                    nn_ref.const_value = n.const_value;
                    nn_ref.is_clock = n.is_clock;
                    nn_ref.is_reset = n.is_reset;
                    nn_ref.is_io = n.is_io;
                }
                net_map[id] = nn;
            }
        }
        for (cid, c) in cells.iter().enumerate() {
            if !live_cell[cid] {
                continue;
            }
            let mut nc = c.clone();
            nc.inputs = c
                .inputs
                .iter()
                .map(|p| PinConn { net: net_map[p.net], pin: p.pin.clone(), bit: p.bit })
                .collect();
            nc.outputs = c
                .outputs
                .iter()
                .map(|p| PinConn { net: net_map[p.net], pin: p.pin.clone(), bit: p.bit })
                .collect();
            new_nl.add_cell(nc);
        }
        self.nl = new_nl;
    }
}

/// Jenis sel FF untuk bit ke-`i` (per-bit FF, reset value dari spec).
fn ff_kind(ff: &PendingFf, _i: usize) -> CellKind {
    let reset_value = ff
        .reset
        .as_ref()
        .map(|rs| rs.value.to_u64())
        .unwrap_or(0);
    match (&ff.reset, ff.enable) {
        (Some(rs), Some(_)) => CellKind::DffRE {
            reset_value,
            polarity: rs.polarity,
            r#async: rs.r#async,
        },
        (Some(rs), None) => CellKind::DffR {
            reset_value,
            polarity: rs.polarity,
            r#async: rs.r#async,
        },
        (None, Some(_)) => CellKind::DffE,
        (None, None) => CellKind::Dff,
    }
}

/// Tech mapping `SirModule` → netlist teknologi (LUT/CARRY4/FF).
pub fn tech_map(sir: &SirModule, arch: &dyn TechArch) -> TechMapResult {
    let mut m = TechMapper::new(sir, arch);
    m.map_all();
    let ff_count = m.nl.cells.iter().filter(|c| c.kind.is_sequential()).count();
    // Hitung ulang dari netlist final (setelah DCE) — counter lama bisa
    // kelebihan (LUT intermediate dihapus).
    let lut_count = m
        .nl
        .cells
        .iter()
        .filter(|c| matches!(c.kind, CellKind::Lut { .. }))
        .count();
    let carry4_count = m
        .nl
        .cells
        .iter()
        .filter(|c| matches!(c.kind, CellKind::Carry4))
        .map(|c| (c.width.max(1) + 3) / 4)
        .sum();
    TechMapResult {
        netlist: m.nl,
        lut_count,
        carry4_count,
        ff_count,
        skipped: m.skipped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maria_core::intern::Symbol;
    use maria_core::LogicVec;
    use maria_netlist::graph::verify_dag;
    use maria_sir::{SirNode, SirRegister, ValueId};
    use maria_tech::GenericArch;

    /// SIR minimal untuk menguji init/leaf — dua slot nilai konstanta.
    fn tiny_sir() -> SirModule {
        let mut m = SirModule::new(Symbol::intern("tiny"));
        let _ = m.add_value(SirValue::Const(LogicVec::from_u64(0, 1)));
        let _ = m.add_value(SirValue::Const(LogicVec::from_u64(1, 1)));
        m
    }

    #[test]
    fn bitfn_init_table_and() {
        let sir = tiny_sir();
        let f = BitFn::And(Box::new(BitFn::Leaf(0, 0)), Box::new(BitFn::Leaf(1, 0)));
        let leaves = vec![(0, 0), (1, 0)];
        assert_eq!(f.init_table(&leaves, &sir), 0x8, "a&b → init 0x8");
    }

    #[test]
    fn bitfn_init_table_or_not() {
        let sir = tiny_sir();
        let o = BitFn::Or(Box::new(BitFn::Leaf(0, 0)), Box::new(BitFn::Leaf(1, 0)));
        assert_eq!(o.init_table(&vec![(0, 0), (1, 0)], &sir), 0xE, "a|b → init 0xE");
        let n = BitFn::Not(Box::new(BitFn::Leaf(0, 0)));
        assert_eq!(n.init_table(&vec![(0, 0)], &sir), 0x1, "~a → init bit0 (input di i0, high tie 0)");
    }

    #[test]
    fn bitfn_init_table_mux() {
        let sir = tiny_sir();
        // Mux(s,t,f) pins i0=s i1=t i2=f; init = s?t:f
        let m = BitFn::Mux(
            Box::new(BitFn::Leaf(0, 0)),
            Box::new(BitFn::Leaf(1, 0)),
            Box::new(BitFn::Const(false)),
        );
        // f=0 → y = s & t → init 0x8
        assert_eq!(m.init_table(&vec![(0, 0), (1, 0)], &sir), 0x8);
    }

    #[test]
    fn bitfn_simplify_folds() {
        let a = BitFn::Leaf(0, 0);
        let t = BitFn::Const(true);
        let f = BitFn::Const(false);
        assert_eq!(BitFn::And(Box::new(a.clone()), Box::new(f.clone())).simplify(), f);
        assert_eq!(BitFn::And(Box::new(a.clone()), Box::new(t.clone())).simplify(), a);
        let nn = BitFn::Not(Box::new(BitFn::Not(Box::new(a.clone())))).simplify();
        assert_eq!(nn, a);
        assert_eq!(BitFn::Xor(Box::new(a.clone()), Box::new(a.clone())).simplify(), f);
        assert_eq!(BitFn::Or(Box::new(a.clone()), Box::new(t.clone())).simplify(), t);
        // a ^ ~a → 1
        assert_eq!(
            BitFn::Xor(Box::new(a.clone()), Box::new(BitFn::Not(Box::new(a.clone())))).simplify(),
            t.clone()
        );
        // mux(t, a, a) → a
        assert_eq!(
            BitFn::Mux(Box::new(t), Box::new(a.clone()), Box::new(a.clone())).simplify(),
            a
        );
    }

    /// SIR counter 8-bit (same shape as maria-netlist tests) — clk, rst_n,
    /// ADD(count,1), FF.
    fn counter_sir() -> SirModule {
        let mut m = SirModule::new(Symbol::intern("counter"));
        let _ = m.add_value(SirValue::Port(0)); // 0 clk
        let _ = m.add_value(SirValue::Port(1)); // 1 rst_n
        let _ = m.add_value(SirValue::Reg(0)); // 2 count q
        let _ = m.add_value(SirValue::Const(LogicVec::from_u64(1, 8))); // 3
        let n = m.add_node(SirNodeKind::Add, vec![2, 3], 8);
        let _ = m.add_value(SirValue::Node(n)); // 4
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
        m.outputs.push(mk("count", maria_sir::PortDir::Output, 2, 8));
        m
    }

    #[test]
    fn tech_map_counter_adder_ffs() {
        let sir = counter_sir();
        let arch = GenericArch;
        let res = tech_map(&sir, &arch);
        assert_eq!(res.ff_count, 8, "FF per-bit — counter 8 bit");
        assert_eq!(res.carry4_count, 2, "8-bit adder → ceil(8/4)=2 CARRY4");
        assert_eq!(res.lut_count, 0, "add polos via CARRY4 — tanpa LUT");
        assert!(res.skipped.is_empty());
        let check = verify_dag(&res.netlist);
        assert!(check.ok, "netlist mapped harus DAG bersih: {check:?}");
        // Semua net internal di-drive; net konstanta punya const_value.
        for net in &res.netlist.nets {
            if net.driver.is_none() && net.const_value.is_none() && !net.is_io {
                panic!("net {} mengambang (tanpa driver/konstanta)", net.name.as_str());
            }
        }
    }

    /// 8-input AND (chain 7 node binary) → tiap node = 1 LUT2 (cone terpotong
    /// di batas node — node perantara jadi leaf, CSE alami).
    fn wide_and_sir() -> SirModule {
        let mut m = SirModule::new(Symbol::intern("wide"));
        let mut ports = Vec::new();
        for i in 0..8 {
            let v = m.add_value(SirValue::Port(i));
            let _ = m.add_wire(Symbol::intern(&format!("a{}", i)), 1, v);
            ports.push(v);
        }
        let mut acc = ports[0];
        for i in 1..8 {
            let n = m.add_node(SirNodeKind::And, vec![acc, ports[i]], 1);
            acc = m.add_value(SirValue::Node(n));
        }
        let mk = |name: &str, value: usize, width: usize| maria_sir::SirPort {
            name: Symbol::intern(name),
            dir: maria_sir::PortDir::Input,
            width,
            value,
        };
        for i in 0..8 {
            m.inputs.push(mk(&format!("a{}", i), i, 1));
        }
        m.outputs.push(mk("y", acc, 1));
        m
    }

    #[test]
    fn tech_map_aig_decomposes_wide_and() {
        let sir = wide_and_sir();
        let arch = GenericArch;
        let res = tech_map(&sir, &arch);
        // F40 resubstitution: chain AND di-inline → cone 8 leaf > K=6 →
        // AIG decompose: 1 LUT6 (6 leaf) + 2 LUT2 = 3 LUT (bukan 1 LUT per
        // node = 7). Fungsi tetap benar — cone di-fuse sebelum LUT cut.
        assert_eq!(res.lut_count, 3, "8-input AND → 1 LUT6 + 2 LUT2 = 3 LUT");
        assert_eq!(res.carry4_count, 0);
        let check = verify_dag(&res.netlist);
        assert!(check.ok, "{check:?}");
    }

    /// SIR ReduceAnd 8-bit — SATU node dengan cone 8 leaf (> K=6) → AIG
    /// decomposition: 1 LUT6 (6 leaf) + 2 LUT2 = 3 LUT.
    fn reduce_and_sir() -> SirModule {
        let mut m = SirModule::new(Symbol::intern("red"));
        let v = m.add_value(SirValue::Port(0));
        let _ = m.add_wire(Symbol::intern("a"), 8, v);
        let n = m.add_node(SirNodeKind::ReduceAnd, vec![v], 1);
        let out = m.nodes[n].output;
        let _ = m.add_value(SirValue::Node(n));
        m.inputs.push(maria_sir::SirPort {
            name: Symbol::intern("a"),
            dir: maria_sir::PortDir::Input,
            width: 8,
            value: v,
        });
        m.outputs.push(maria_sir::SirPort {
            name: Symbol::intern("y"),
            dir: maria_sir::PortDir::Output,
            width: 1,
            value: out,
        });
        m
    }

    #[test]
    fn tech_map_reduce_and_decomposes_wide_cone() {
        let sir = reduce_and_sir();
        let arch = GenericArch;
        let res = tech_map(&sir, &arch);
        assert_eq!(res.lut_count, 3, "8-leaf ReduceAnd → 1 LUT6 + 2 LUT2");
        let check = verify_dag(&res.netlist);
        assert!(check.ok, "{check:?}");
    }

    #[test]
    fn emit_mapped_netlist_has_lut_and_carry_modules() {
        let arch = GenericArch;
        // counter → CARRY4 + FF; or → LUT. Emisi gabungan: dua top di satu
        // string hanya untuk memeriksa keberadaan definisi modul.
        let mut v = maria_netlist::emit_verilog(&tech_map(&counter_sir(), &arch).netlist);
        v.push_str(&maria_netlist::emit_verilog(&tech_map(&or_sir(), &arch).netlist));
        assert!(v.contains("module CARRY4 #(parameter W = 4)"), "modul CARRY4");
        assert!(v.contains("always_comb"), "LUT pakai always_comb case");
        assert!(v.contains("module DFFR_"), "modul FF reset");
        assert!(v.contains(".ci("), "koneksi carry-in");
        assert!(v.contains("6'd3: o = 1'b1;"), "OR init 0xE → addr 3 true");
        assert!(v.contains("module LUT6_"), "modul LUT (init di module_key)");
    }

    /// `a|b` 8-bit — dipakai tes emit LUT.
    fn or_sir() -> SirModule {
        let mut m = SirModule::new(Symbol::intern("g"));
        let _ = m.add_value(SirValue::Port(0)); // a
        let _ = m.add_value(SirValue::Port(1)); // b
        let n = m.add_node(SirNodeKind::Or, vec![0, 1], 8);
        let out = m.nodes[n].output;
        let _ = m.add_value(SirValue::Node(n));
        let _ = m.add_wire(Symbol::intern("y"), 8, out);
        let mk = |name: &str, value: usize| maria_sir::SirPort {
            name: Symbol::intern(name),
            dir: maria_sir::PortDir::Input,
            width: 8,
            value,
        };
        m.inputs.push(mk("a", 0));
        m.inputs.push(mk("b", 1));
        m.outputs.push(mk("y", out));
        m
    }

    #[test]
    fn tech_map_or_uses_lut_init() {
        let m = or_sir();
        let arch = GenericArch;
        let res = tech_map(&m, &arch);
        assert_eq!(res.lut_count, 8, "a|b 8-bit → 8 LUT (1 per bit)");
        // init tiap LUT = 0xE (a|b).
        for c in &res.netlist.cells {
            if let CellKind::Lut { init } = c.kind {
                assert_eq!(init, 0xE, "init OR harus 0xE");
            }
        }
        assert!(verify_dag(&res.netlist).ok);
    }

    /// SIR ALU 8-bit — `case (op) {0: a&b, 1: a|b, 2: a+b, default: a^b}`
    /// di-lower menjadi mux chain bertingkat: n9 = mux(n8, n7, n6),
    /// n6 = mux(n5, n4, n3), n3 = mux(n2, n1, n0) (sama dengan dump SIR
    /// `examples/synth/alu.sv`).
    fn alu_sir() -> SirModule {
        let mut m = SirModule::new(Symbol::intern("alu"));
        let _ = m.add_value(SirValue::Port(0)); // a
        let _ = m.add_value(SirValue::Port(1)); // b
        let _ = m.add_value(SirValue::Port(2)); // op
        let c2 = m.add_value(SirValue::Const(LogicVec::from_u64(2, 32)));
        let c1 = m.add_value(SirValue::Const(LogicVec::from_u64(1, 32)));
        let c0 = m.add_value(SirValue::Const(LogicVec::from_u64(0, 32)));
        let n0 = m.add_node(SirNodeKind::Xor, vec![0, 1], 8);
        let v0 = m.add_value(SirValue::Node(n0));
        let n1 = m.add_node(SirNodeKind::Add, vec![0, 1], 8);
        let v1 = m.add_value(SirValue::Node(n1));
        let n2 = m.add_node(SirNodeKind::Eq, vec![2, c2], 1);
        let v2 = m.add_value(SirValue::Node(n2));
        let n3 = m.add_node(SirNodeKind::Mux, vec![v2, v1, v0], 8);
        let v3 = m.add_value(SirValue::Node(n3));
        let n4 = m.add_node(SirNodeKind::Or, vec![0, 1], 8);
        let v4 = m.add_value(SirValue::Node(n4));
        let n5 = m.add_node(SirNodeKind::Eq, vec![2, c1], 1);
        let v5 = m.add_value(SirValue::Node(n5));
        let n6 = m.add_node(SirNodeKind::Mux, vec![v5, v4, v3], 8);
        let v6 = m.add_value(SirValue::Node(n6));
        let n7 = m.add_node(SirNodeKind::And, vec![0, 1], 8);
        let v7 = m.add_value(SirValue::Node(n7));
        let n8 = m.add_node(SirNodeKind::Eq, vec![2, c0], 1);
        let v8 = m.add_value(SirValue::Node(n8));
        let n9 = m.add_node(SirNodeKind::Mux, vec![v8, v7, v6], 8);
        let v9 = m.add_value(SirValue::Node(n9));
        let _ = m.add_wire(Symbol::intern("y"), 8, v9);
        let mk = |name: &str, value: usize, width: usize| maria_sir::SirPort {
            name: Symbol::intern(name),
            dir: maria_sir::PortDir::Input,
            width,
            value,
        };
        m.inputs.push(mk("a", 0, 8));
        m.inputs.push(mk("b", 1, 8));
        m.inputs.push(mk("op", 2, 2));
        m.outputs.push(mk("y", v9, 8));
        m
    }

    #[test]
    fn tech_map_alu_case_fuses_to_8_luts() {
        // Kriteria phase 4: `alu.sv` → LUT count sesuai ekspektasi.
        // Resubstitution menggabungkan mux chain (case) + and/or/xor + eq
        // menjadi SATU cone per bit: leaf a[i], b[i], op[0], op[1], sum-bit
        // (net CARRY4) = 5 ≤ K=6 → 1 LUT6 per bit → 8 LUT. a+b → 2 CARRY4.
        let sir = alu_sir();
        let arch = GenericArch;
        let res = tech_map(&sir, &arch);
        assert_eq!(res.lut_count, 8, "alu case → 1 LUT6 per bit (cone 5 leaf)");
        assert_eq!(res.carry4_count, 2, "a+b 8-bit → ceil(8/4)=2 CARRY4");
        assert_eq!(res.ff_count, 0);
        let check = verify_dag(&res.netlist);
        assert!(check.ok, "{check:?}");
        // Semua net ter-drive/konstanta/io — tidak ada floating.
        for net in &res.netlist.nets {
            if net.driver.is_none() && net.const_value.is_none() && !net.is_io {
                panic!("net {} mengambang", net.name.as_str());
            }
        }
    }
}
