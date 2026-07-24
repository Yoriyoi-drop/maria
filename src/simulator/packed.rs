//! Packed 4-state logic representation — SIMD-ready bit-packing.
//!
//! # Encoding
//!
//! Each 4-state cell (X, Z, 0, 1) dikodekan dalam **2 bit** menggunakan
//! dua bitmask `u64`: `known` dan `value`.
//!
//! | State | known | value | Keterangan               |
//! |-------|-------|-------|--------------------------|
//! |   X   |   0   |   0   | Unknown                  |
//! |   Z   |   0   |   1   | High-impedance           |
//! |   0   |   1   |   0   | Logic 0                  |
//! |   1   |   1   |   1   | Logic 1                  |
//!
//! # SIMD Benefit
//!
//! Operasi bitwise (AND, OR, XOR, NOT) menggunakan **2-3 instruksi CPU**
//! per chunk, tanpa branching, tanpa loop per-bit. Untuk sinyal ≤64 bit,
//! semua operasi adalah u64 bitwise langsung — 100% CPU pipeline friendly.
//!
//! Catatan: AND dan OR membutuhkan logika 4-state khusus (0/1 dominance),
//! sehingga sedikit lebih kompleks dari XOR/NOT yang bisa bitmask langsung.

use crate::ir::{BinaryIrOp, LogicVal, LogicVec, UnaryIrOp};

/// Number of cells per chunk (u64 = 64 bits).
const CELLS_PER_CHUNK: usize = 64;

/// Packed 4-state logic vector.
///
/// Setiap chunk adalah `(known_mask: u64, value_mask: u64)`.
/// - `known_mask`: bit ke-i = 1 jika nilai diketahui (0/1), 0 jika X/Z.
/// - `value_mask`: bit ke-i = nilai aktual (hanya valid jika known=1).
///
/// Untuk sinyal >64 bit, chunks berisi Vec<(u64, u64)>.
/// Untuk sinyal ≤64 bit, hanya chunks[0] yang digunakan.
///
/// # 4-State Truth Tables
///
/// AND: 0 mendominasi (a&0=0 regardless of b). X/Z with anything non-zero → X.
/// OR:  1 mendominasi (a|1=1 regardless of b). X/Z with anything non-zero → X.
/// XOR: X/Z jika salah satu X/Z. Otherwise normal XOR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedLogicVec {
    /// Chunks of (known, value) — each chunk covers 64 cells.
    chunks: Vec<(u64, u64)>,
    /// Total number of cells (bits).
    width: usize,
}

impl PackedLogicVec {
    // ─── Construction ───

    /// Create a new packed vector initialized to X (unknown).
    pub fn new(width: usize) -> Self {
        let w = width.max(1);
        let num_chunks = (w + CELLS_PER_CHUNK - 1) / CELLS_PER_CHUNK;
        // X = known:0, value:0
        let chunks = vec![(0u64, 0u64); num_chunks];
        PackedLogicVec { chunks, width: w }
    }

    /// Create from a single bit value with given width.
    pub fn fill(val: LogicVal, width: usize) -> Self {
        let w = width.max(1);
        let num_chunks = (w + CELLS_PER_CHUNK - 1) / CELLS_PER_CHUNK;
        let mut chunks = Vec::with_capacity(num_chunks);
        for c in 0..num_chunks {
            let chunk_width = (w - c * CELLS_PER_CHUNK).min(CELLS_PER_CHUNK);
            let mask = if chunk_width >= 64 { !0u64 } else { (1u64 << chunk_width) - 1 };
            let (known, value) = match val {
                LogicVal::X => (0u64, 0u64),
                LogicVal::Z => (0u64, mask),       // all bits = Z
                LogicVal::Zero => (mask, 0u64),     // all bits = 0 (known)
                LogicVal::One => (mask, mask),      // all bits = 1 (known)
            };
            chunks.push((known, value));
        }
        PackedLogicVec { chunks, width: w }
    }

    /// Create from a u64 value with given width (0/1 bits only).
    /// Untuk sinyal >64 bit, hanya lower 64 bit yang diisi; sisanya X.
    pub fn from_u64(val: u64, width: usize) -> Self {
        let w = width.max(1);
        let num_chunks = (w + CELLS_PER_CHUNK - 1) / CELLS_PER_CHUNK;
        let mut chunks = Vec::with_capacity(num_chunks);
        for c in 0..num_chunks {
            let bit_offset = c * CELLS_PER_CHUNK;
            let chunk_width = (w - bit_offset).min(CELLS_PER_CHUNK);
            let mask = if chunk_width >= 64 { !0u64 } else { (1u64 << chunk_width) - 1 };
            let shifted = if bit_offset < 64 { val >> bit_offset } else { 0 };
            let value = shifted & mask;
            let known = mask; // All bits are known 0/1
            chunks.push((known, value));
        }
        PackedLogicVec { chunks, width: w }
    }

    /// Convert from a traditional LogicVec (Vec<LogicVal> based).
    pub fn from_logicvec(lv: &LogicVec) -> Self {
        let w = lv.width.max(1);
        let num_chunks = (w + CELLS_PER_CHUNK - 1) / CELLS_PER_CHUNK;
        let mut chunks = vec![(0u64, 0u64); num_chunks];
        for i in 0..lv.width {
            let (k, v) = match lv.bits[i] {
                LogicVal::X => (0u64, 0u64),
                LogicVal::Z => (0u64, 1u64),
                LogicVal::Zero => (1u64, 0u64),
                LogicVal::One => (1u64, 1u64),
            };
            let (chunk_idx, bit) = (i / CELLS_PER_CHUNK, i % CELLS_PER_CHUNK);
            chunks[chunk_idx].0 |= k << bit;
            chunks[chunk_idx].1 |= v << bit;
        }
        PackedLogicVec { chunks, width: w }
    }

    /// Convert back to a traditional LogicVec.
    pub fn to_logicvec(&self) -> LogicVec {
        let mut bits = Vec::with_capacity(self.width);
        for i in 0..self.width {
            let (chunk_idx, bit) = (i / CELLS_PER_CHUNK, i % CELLS_PER_CHUNK);
            let (known, value) = self.chunks[chunk_idx];
            let k = (known >> bit) & 1;
            let v = (value >> bit) & 1;
            bits.push(match (k, v) {
                (0, 0) => LogicVal::X,
                (0, 1) => LogicVal::Z,
                (1, 0) => LogicVal::Zero,
                (1, 1) => LogicVal::One,
                _ => LogicVal::X,
            });
        }
        let width = self.width;
        LogicVec { bits, width }
    }

    // ─── Mask Helpers ───

    /// Get the active bit mask for the last chunk.
    fn last_chunk_mask(&self) -> u64 {
        if self.width == 0 {
            return 0;
        }
        let last_bit = self.width % CELLS_PER_CHUNK;
        if last_bit == 0 { !0u64 } else { (1u64 << last_bit) - 1 }
    }

    /// Apply width mask to all chunks.
    fn apply_mask(&self) -> Vec<(u64, u64)> {
        let last_idx = self.chunks.len().saturating_sub(1);
        let mask = self.last_chunk_mask();
        self.chunks.iter().enumerate().map(|(i, &(k, v))| {
            if i == last_idx && mask != !0u64 {
                (k & mask, v & mask)
            } else {
                (k, v)
            }
        }).collect()
    }

    /// Get a single cell as LogicVal.
    fn get_cell(&self, i: usize) -> LogicVal {
        if i >= self.width {
            return LogicVal::X;
        }
        let (chunk_idx, bit) = (i / CELLS_PER_CHUNK, i % CELLS_PER_CHUNK);
        let (known, value) = self.chunks[chunk_idx];
        let k = (known >> bit) & 1;
        let v = (value >> bit) & 1;
        match (k, v) {
            (0, 0) => LogicVal::X,
            (0, 1) => LogicVal::Z,
            (1, 0) => LogicVal::Zero,
            (1, 1) => LogicVal::One,
            _ => LogicVal::X,
        }
    }

    // ─── Accessors ───

    /// Get the width in cells (bits).
    pub fn width(&self) -> usize {
        self.width
    }

    /// Get the number of chunks.
    pub fn num_chunks(&self) -> usize {
        self.chunks.len()
    }

    /// Get raw chunk data: slice of (known, value) pairs.
    pub fn chunks(&self) -> &[(u64, u64)] {
        &self.chunks
    }

    /// Check if all cells are X.
    pub fn all_x(&self) -> bool {
        let masked = self.apply_mask();
        masked.iter().all(|&(known, value)| known == 0 && value == 0)
    }

    /// Check if all cells are Z.
    pub fn all_z(&self) -> bool {
        let masked = self.apply_mask();
        if masked.is_empty() {
            return true;
        }
        // For Z, known=0 and value=1 for each cell
        // So known must be 0, and value must have all 1s within the active width
        let last_idx = masked.len() - 1;
        masked.iter().enumerate().all(|(i, &(known, value))| {
            if i == last_idx {
                // Last chunk: all bits must be Z within the active width
                // Z = known:0, value:1 for each cell
                known == 0 && value == self.last_chunk_mask()
            } else {
                known == 0 && value == !0u64
            }
        })
    }

    /// Convert to u64 (only known bits; X/Z bits become 0).
    pub fn to_u64(&self) -> u64 {
        if self.chunks.is_empty() {
            return 0;
        }
        let (known, value) = self.chunks[0];
        // Only return known 0/1 bits, masked to active width
        let chunk_width = self.width.min(CELLS_PER_CHUNK);
        let width_mask = if chunk_width >= 64 { !0u64 } else { (1u64 << chunk_width) - 1 };
        (value & known) & width_mask
    }

    /// Convert to bool (1-bit interpretation).
    pub fn to_bool(&self) -> Option<bool> {
        if self.width == 0 {
            return Some(false);
        }
        let (known0, value0) = self.chunks[0];
        let chunk_width = self.width.min(CELLS_PER_CHUNK);
        let width_mask = if chunk_width >= 64 { !0u64 } else { (1u64 << chunk_width) - 1 };
        let known = known0 & width_mask;
        let value = value0 & width_mask;

        // All X or Z → return None
        if known == 0 {
            return None;
        }
        // Any bit is 1 (and known) → true
        let any_one = (value & known) != 0;
        Some(any_one)
    }

    /// Resize to new width (truncate or zero-extend).
    pub fn resize(&self, new_width: usize) -> Self {
        if new_width == self.width {
            return self.clone();
        }
        let w = new_width.max(1);
        let new_chunks = (w + CELLS_PER_CHUNK - 1) / CELLS_PER_CHUNK;
        let mut chunks = self.chunks.clone();
        chunks.resize(new_chunks, (0u64, 0u64));
        // Truncate the last chunk if needed
        let last_bit = w % CELLS_PER_CHUNK;
        if last_bit > 0 {
            let last_idx = chunks.len() - 1;
            let mask = (1u64 << last_bit) - 1;
            chunks[last_idx].0 &= mask;
            chunks[last_idx].1 &= mask;
        }
        PackedLogicVec { chunks, width: w }
    }

    /// Concatenate two packed vectors.
    pub fn extend(&self, other: &PackedLogicVec) -> Self {
        // Fallback: convert to LogicVec, concat, convert back
        // Optimization: chunk-level bit manipulation bisa ditambahkan nanti
        let mut lv_self = self.to_logicvec();
        let lv_other = other.to_logicvec();
        lv_self.bits.extend(lv_other.bits);
        lv_self.width = self.width + other.width;
        PackedLogicVec::from_logicvec(&lv_self)
    }

    // ─── Bitwise Operations (SIMD-friendly) ───

    /// Bitwise AND — 4-state correct.
    ///
    /// Truth table: 0 mendominasi (a&0=0). X/Z dengan non-zero → X.
    /// Formula per bit:
    /// - result = 0 jika a=0 ATAU b=0
    /// - result = 1 jika a=1 DAN b=1
    /// - result = X otherwise
    pub fn bitwise_and(&self, other: &PackedLogicVec) -> PackedLogicVec {
        let max_width = self.width.max(other.width);
        let chunks = crate::simulator::simd_packed::simd_and(&self.chunks, &other.chunks);
        PackedLogicVec { chunks, width: max_width }
    }

    /// Bitwise OR — 4-state correct.
    ///
    /// Truth table: 1 mendominasi (a|1=1). X/Z dengan non-zero → X.
    /// Formula per bit:
    /// - result = 1 jika a=1 ATAU b=1
    /// - result = 0 jika a=0 DAN b=0
    /// - result = X otherwise
    pub fn bitwise_or(&self, other: &PackedLogicVec) -> PackedLogicVec {
        let max_width = self.width.max(other.width);
        let chunks = crate::simulator::simd_packed::simd_or(&self.chunks, &other.chunks);
        PackedLogicVec { chunks, width: max_width }
    }

    /// Bitwise XOR — 4-state correct.
    ///
    /// Truth table: X/Z if either input is X/Z, normal XOR otherwise.
    /// Formula per bit:
    /// - known = a.known & b.known
    /// - value = (a.value ^ b.value) & known
    pub fn bitwise_xor(&self, other: &PackedLogicVec) -> PackedLogicVec {
        let max_width = self.width.max(other.width);
        let chunks = crate::simulator::simd_packed::simd_xor(&self.chunks, &other.chunks);
        PackedLogicVec { chunks, width: max_width }
    }

    /// Bitwise XNOR = NOT(XOR).
    pub fn bitwise_xnor(&self, other: &PackedLogicVec) -> PackedLogicVec {
        let xor = self.bitwise_xor(other);
        xor.bitwise_not()
    }

    /// Bitwise NOT — 2 operasi per chunk.
    ///
    /// Formula: known unchanged, value flipped where known.
    /// X→X, Z→Z, 0→1, 1→0
    pub fn bitwise_not(&self) -> PackedLogicVec {
        let chunks = crate::simulator::simd_packed::simd_not(&self.chunks);
        PackedLogicVec { chunks, width: self.width }
    }

    // ─── Reduction Operations ───

    /// Reduction AND: returns 1 if all bits are 1, 0 if any bit is 0, X otherwise.
    pub fn red_and(&self) -> PackedLogicVec {
        let last_idx = self.chunks.len().saturating_sub(1);
        let last_chunk_full_mask = self.last_chunk_mask();
        // Iterate over original chunks, applying mask for last chunk
        for (i, &(known, value)) in self.chunks.iter().enumerate() {
            // Determine the full mask for this chunk
            let full_mask = if i == last_idx && last_chunk_full_mask != !0u64 {
                last_chunk_full_mask
            } else {
                !0u64
            };
            let km = known & full_mask;
            let vm = value & full_mask;
            // Any known 0 bit → result is 0
            let known_zero = km & !vm;
            if known_zero != 0 {
                return PackedLogicVec::fill(LogicVal::Zero, 1);
            }
            // Any unknown (X/Z) bit in this chunk
            if km != full_mask {
                return PackedLogicVec::fill(LogicVal::X, 1);
            }
        }
        // All bits are known 1
        PackedLogicVec::fill(LogicVal::One, 1)
    }

    /// Reduction OR: returns 1 if any bit is 1, 0 if all bits are 0, X otherwise.
    pub fn red_or(&self) -> PackedLogicVec {
        let last_idx = self.chunks.len().saturating_sub(1);
        let last_chunk_full_mask = self.last_chunk_mask();
        let mut all_known = true;
        for (i, &(known, value)) in self.chunks.iter().enumerate() {
            let full_mask = if i == last_idx && last_chunk_full_mask != !0u64 {
                last_chunk_full_mask
            } else {
                !0u64
            };
            let km = known & full_mask;
            let vm = value & full_mask;
            // Any known 1 bit → result is 1
            let known_one = km & vm;
            if known_one != 0 {
                return PackedLogicVec::fill(LogicVal::One, 1);
            }
            // Any unknown (X/Z) bit
            if km != full_mask {
                all_known = false;
            }
        }
        if all_known {
            PackedLogicVec::fill(LogicVal::Zero, 1)
        } else {
            PackedLogicVec::fill(LogicVal::X, 1)
        }
    }

    /// Reduction XOR: returns 1 if odd number of 1s, 0 if even, X if any X/Z.
    pub fn red_xor(&self) -> PackedLogicVec {
        let last_idx = self.chunks.len().saturating_sub(1);
        let last_chunk_full_mask = self.last_chunk_mask();
        let mut xor_acc = 0u64;
        let mut all_known = true;
        for (i, &(known, value)) in self.chunks.iter().enumerate() {
            let full_mask = if i == last_idx && last_chunk_full_mask != !0u64 {
                last_chunk_full_mask
            } else {
                !0u64
            };
            let km = known & full_mask;
            let vm = value & full_mask;
            if km != full_mask {
                all_known = false;
            }
            xor_acc ^= (vm & km);
        }
        if !all_known {
            PackedLogicVec::fill(LogicVal::X, 1)
        } else {
            let parity = xor_acc.count_ones() & 1;
            PackedLogicVec::from_u64(parity as u64, 1)
        }
    }

    /// Reduction NAND.
    pub fn red_nand(&self) -> PackedLogicVec {
        let and = self.red_and();
        and.bitwise_not()
    }

    /// Reduction NOR.
    pub fn red_nor(&self) -> PackedLogicVec {
        let or = self.red_or();
        or.bitwise_not()
    }

    /// Reduction XNOR.
    pub fn red_xnor(&self) -> PackedLogicVec {
        let xor = self.red_xor();
        xor.bitwise_not()
    }

    // ─── Shift Operations ───

    /// Logical shift left.
    pub fn shl(&self, shift: usize) -> PackedLogicVec {
        if shift == 0 || self.width == 0 {
            return self.clone();
        }
        if shift >= self.width {
            return PackedLogicVec::fill(LogicVal::Zero, self.width);
        }
        // Fallback ke LogicVec untuk initial implementation
        let mut lv = self.to_logicvec();
        for i in (shift..self.width).rev() {
            lv.bits[i] = lv.bits[i - shift];
        }
        for i in 0..shift.min(self.width) {
            lv.bits[i] = LogicVal::Zero;
        }
        PackedLogicVec::from_logicvec(&lv)
    }

    /// Logical shift right.
    pub fn shr(&self, shift: usize) -> PackedLogicVec {
        if shift == 0 || self.width == 0 {
            return self.clone();
        }
        if shift >= self.width {
            return PackedLogicVec::fill(LogicVal::Zero, self.width);
        }
        let mut lv = self.to_logicvec();
        for i in 0..(self.width - shift) {
            lv.bits[i] = lv.bits[i + shift];
        }
        for i in (self.width - shift)..self.width {
            lv.bits[i] = LogicVal::Zero;
        }
        PackedLogicVec::from_logicvec(&lv)
    }

    /// Arithmetic shift left (same as logical for unsigned).
    pub fn sshl(&self, shift: usize) -> PackedLogicVec {
        self.shl(shift)
    }

    /// Arithmetic shift right (sign-extend).
    pub fn sshr(&self, shift: usize) -> PackedLogicVec {
        if shift == 0 || self.width == 0 {
            return self.clone();
        }
        if shift >= self.width {
            let msb = self.get_cell(self.width - 1);
            return PackedLogicVec::fill(msb, self.width);
        }
        let msb = self.get_cell(self.width - 1);
        let mut lv = self.to_logicvec();
        for i in 0..(self.width - shift) {
            lv.bits[i] = lv.bits[i + shift];
        }
        for i in (self.width - shift)..self.width {
            lv.bits[i] = msb;
        }
        PackedLogicVec::from_logicvec(&lv)
    }

    // ─── Comparison Operations ───

    /// Equality comparison (Verilog === semantics: X/Z must match exactly).
    pub fn eq(&self, other: &PackedLogicVec) -> PackedLogicVec {
        let max_chunks = self.chunks.len().max(other.chunks.len());
        for i in 0..max_chunks {
            let (ak, av) = self.chunks.get(i).copied().unwrap_or((0, 0));
            let (bk, bv) = other.chunks.get(i).copied().unwrap_or((0, 0));
            // For ===: ALL bits (including X/Z) must match exactly
            if ak != bk || av != bv {
                return PackedLogicVec::from_u64(0, 1);
            }
        }
        PackedLogicVec::from_u64(1, 1)
    }

    /// Casex equality: X/Z in the pattern are don't-care.
    pub fn casex_eq(&self, pattern: &PackedLogicVec) -> PackedLogicVec {
        // Gunakan LogicVec fallback untuk correctness
        let val_lv = self.to_logicvec();
        let pat_lv = pattern.to_logicvec();
        let result = val_lv.casex_eq(&pat_lv);
        PackedLogicVec::from_u64(if result { 1 } else { 0 }, 1)
    }

    /// Casez equality: Z in the pattern is don't-care.
    pub fn casez_eq(&self, pattern: &PackedLogicVec) -> PackedLogicVec {
        let val_lv = self.to_logicvec();
        let pat_lv = pattern.to_logicvec();
        let result = val_lv.casez_eq(&pat_lv);
        PackedLogicVec::from_u64(if result { 1 } else { 0 }, 1)
    }

    /// Not-equal.
    pub fn neq(&self, other: &PackedLogicVec) -> PackedLogicVec {
        let eq = self.eq(other);
        eq.bitwise_not()
    }

    // ─── Arithmetic Operations ───

    /// Addition — fallback ke LogicVec karena kompleksitas carry.
    pub fn add(&self, other: &PackedLogicVec) -> PackedLogicVec {
        let lv_self = self.to_logicvec();
        let lv_other = other.to_logicvec();
        let result = crate::simulator::value::eval_binary(BinaryIrOp::Add, &lv_self, &lv_other);
        PackedLogicVec::from_logicvec(&result)
    }

    /// Subtraction.
    pub fn sub(&self, other: &PackedLogicVec) -> PackedLogicVec {
        let lv_self = self.to_logicvec();
        let lv_other = other.to_logicvec();
        let result = crate::simulator::value::eval_binary(BinaryIrOp::Sub, &lv_self, &lv_other);
        PackedLogicVec::from_logicvec(&result)
    }
}

impl std::fmt::Display for PackedLogicVec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for i in (0..self.width).rev() {
            let (chunk_idx, bit) = (i / CELLS_PER_CHUNK, i % CELLS_PER_CHUNK);
            let (known, value) = self.chunks[chunk_idx];
            let k = (known >> bit) & 1;
            let v = (value >> bit) & 1;
            let c = match (k, v) {
                (0, 0) => 'x',
                (0, 1) => 'z',
                (1, 0) => '0',
                (1, 1) => '1',
                _ => '?',
            };
            write!(f, "{}", c)?;
        }
        Ok(())
    }
}

// ─── Packed Binary/Unary Evaluators ───

/// Evaluate a unary operation using packed representation.
/// Returns None if the operation cannot be evaluated in packed form.
pub fn eval_unary_packed(op: &UnaryIrOp, val: &PackedLogicVec) -> Option<PackedLogicVec> {
    match op {
        UnaryIrOp::Plus => Some(val.clone()),
        UnaryIrOp::BitNot => Some(val.bitwise_not()),
        UnaryIrOp::Not => {
            let b = val.to_bool()?;
            Some(PackedLogicVec::from_u64(if b { 0 } else { 1 }, 1))
        }
        UnaryIrOp::Minus => {
            // Two's complement: ~val + 1
            let not = val.bitwise_not();
            let one = PackedLogicVec::from_u64(1, val.width());
            Some(not.add(&one))
        }
        UnaryIrOp::RedAnd => Some(val.red_and()),
        UnaryIrOp::RedNand => Some(val.red_nand()),
        UnaryIrOp::RedOr => Some(val.red_or()),
        UnaryIrOp::RedNor => Some(val.red_nor()),
        UnaryIrOp::RedXor => Some(val.red_xor()),
        UnaryIrOp::RedXnor => Some(val.red_xnor()),
    }
}

/// Evaluate a binary operation using packed representation.
/// Returns None if the operation cannot be evaluated in packed form.
pub fn eval_binary_packed(op: &BinaryIrOp, lhs: &PackedLogicVec, rhs: &PackedLogicVec) -> Option<PackedLogicVec> {
    match op {
        BinaryIrOp::BitAnd => Some(lhs.bitwise_and(rhs)),
        BinaryIrOp::BitOr => Some(lhs.bitwise_or(rhs)),
        BinaryIrOp::BitXor => Some(lhs.bitwise_xor(rhs)),
        BinaryIrOp::BitXnor => Some(lhs.bitwise_xnor(rhs)),
        // Comparison ops (Eq, Neq, CaseEq, etc.) are handled by JIT or interpreted
        // for correct 1-bit width handling and X/Z semantics.
        _ => None,
    }
}    /// Check if a binary operation can be accelerated by packed eval.
    /// Only true bitwise ops benefit from SIMD bitmask acceleration.
    /// Comparison ops are handled by JIT or interpreted for correct width/XZ semantics.
    pub fn is_packable_binary_op(op: &BinaryIrOp) -> bool {
        matches!(
            op,
            BinaryIrOp::BitAnd | BinaryIrOp::BitOr | BinaryIrOp::BitXor | BinaryIrOp::BitXnor
        )
    }

/// Extended packed binary evaluator dengan width extension.
/// Fallback ke LogicVec untuk operasi non-bitwise.
pub fn eval_binary_packed_extended(op: &BinaryIrOp, lhs: &PackedLogicVec, rhs: &PackedLogicVec) -> PackedLogicVec {
    if let Some(result) = eval_binary_packed(op, lhs, rhs) {
        return result;
    }
    let lv_lhs = lhs.to_logicvec();
    let lv_rhs = rhs.to_logicvec();
    let result = crate::simulator::value::eval_binary(op.clone(), &lv_lhs, &lv_rhs);
    PackedLogicVec::from_logicvec(&result)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Construction Tests ───

    #[test]
    fn test_new_default_x() {
        let pv = PackedLogicVec::new(8);
        assert_eq!(pv.width(), 8);
        assert!(pv.all_x(), "new PackedLogicVec should be all X");
        assert_eq!(format!("{}", pv), "xxxxxxxx");
    }

    #[test]
    fn test_from_u64() {
        let pv = PackedLogicVec::from_u64(0b1010, 4);
        assert_eq!(format!("{}", pv), "1010");
        assert_eq!(pv.to_u64(), 0b1010);
    }

    #[test]
    fn test_from_u64_wide() {
        // >64 bit signal — hanya lower 64 bit yang terisi
        let pv = PackedLogicVec::from_u64(!0u64, 128);
        assert_eq!(pv.width(), 128);
        assert_eq!(pv.num_chunks(), 2);
        // 64 bit pertama = 1, 64 bit kedua = 0
        let lv = pv.to_logicvec();
        for i in 0..64 {
            assert_eq!(lv.bits[i], LogicVal::One, "bit {} should be 1", i);
        }
        for i in 64..128 {
            assert_eq!(lv.bits[i], LogicVal::Zero, "bit {} should be 0", i);
        }
    }

    #[test]
    fn test_fill_one() {
        let pv = PackedLogicVec::fill(LogicVal::One, 8);
        assert_eq!(format!("{}", pv), "11111111");
        let lv = pv.to_logicvec();
        assert!(lv.bits.iter().all(|b| *b == LogicVal::One));
    }

    #[test]
    fn test_fill_zero() {
        let pv = PackedLogicVec::fill(LogicVal::Zero, 4);
        assert_eq!(format!("{}", pv), "0000");
    }

    #[test]
    fn test_fill_z() {
        let pv = PackedLogicVec::fill(LogicVal::Z, 3);
        assert_eq!(format!("{}", pv), "zzz");
    }

    #[test]
    fn test_fill_single_bit() {
        let pv = PackedLogicVec::fill(LogicVal::One, 1);
        assert_eq!(pv.to_u64(), 1, "fill(One, 1).to_u64() should be 1, got {}", pv.to_u64());
    }

    #[test]
    fn test_from_logicvec() {
        let lv = LogicVec::from_u64(0b1100, 4);
        let pv = PackedLogicVec::from_logicvec(&lv);
        assert_eq!(format!("{}", pv), "1100");
    }

    #[test]
    fn test_to_logicvec_roundtrip() {
        let original = LogicVec::from_u64(0xDEAD, 16);
        let packed = PackedLogicVec::from_logicvec(&original);
        let recovered = packed.to_logicvec();
        assert_eq!(original, recovered);
    }

    #[test]
    fn test_to_logicvec_with_xz() {
        let lv = LogicVec { bits: vec![LogicVal::X, LogicVal::Z, LogicVal::Zero, LogicVal::One], width: 4 };
        let packed = PackedLogicVec::from_logicvec(&lv);
        let recovered = packed.to_logicvec();
        assert_eq!(lv, recovered);
    }

    // ─── Bitwise Operation Tests ───

    #[test]
    fn test_bitwise_and() {
        let a = PackedLogicVec::from_u64(0b1100, 4);
        let b = PackedLogicVec::from_u64(0b1010, 4);
        let r = a.bitwise_and(&b);
        assert_eq!(r.to_u64(), 0b1000);
        assert_eq!(format!("{}", r), "1000");
    }

    #[test]
    fn test_bitwise_or() {
        let a = PackedLogicVec::from_u64(0b1100, 4);
        let b = PackedLogicVec::from_u64(0b1010, 4);
        let r = a.bitwise_or(&b);
        assert_eq!(r.to_u64(), 0b1110);
    }

    #[test]
    fn test_bitwise_xor() {
        let a = PackedLogicVec::from_u64(0b1100, 4);
        let b = PackedLogicVec::from_u64(0b1010, 4);
        let r = a.bitwise_xor(&b);
        assert_eq!(r.to_u64(), 0b0110);
    }

    #[test]
    fn test_bitwise_not() {
        let a = PackedLogicVec::from_u64(0b1010, 4);
        let r = a.bitwise_not();
        assert_eq!(r.to_u64(), 0b0101);
    }

    #[test]
    fn test_bitwise_xnor() {
        let a = PackedLogicVec::from_u64(0b1100, 4);
        let b = PackedLogicVec::from_u64(0b1010, 4);
        let r = a.bitwise_xnor(&b);
        assert_eq!(r.to_u64(), 0b1001);
    }

    #[test]
    fn test_bitwise_and_x_0() {
        // X & 0 = 0 (karena 0 mendominasi AND)
        let x_packed = PackedLogicVec::fill(LogicVal::X, 4);
        let zero_packed = PackedLogicVec::fill(LogicVal::Zero, 4);
        let r = x_packed.bitwise_and(&zero_packed);
        let lv = r.to_logicvec();
        assert!(lv.bits.iter().all(|b| *b == LogicVal::Zero), "X & 0 should be 0, got {}", r);
    }

    #[test]
    fn test_bitwise_and_x_1() {
        // X & 1 = X
        let x_packed = PackedLogicVec::fill(LogicVal::X, 4);
        let one_packed = PackedLogicVec::fill(LogicVal::One, 4);
        let r = x_packed.bitwise_and(&one_packed);
        assert!(r.all_x(), "X & 1 should be X, got {}", r);
    }

    #[test]
    fn test_bitwise_or_x_1() {
        // X | 1 = 1 (karena 1 mendominasi OR)
        let x_packed = PackedLogicVec::fill(LogicVal::X, 4);
        let one_packed = PackedLogicVec::fill(LogicVal::One, 4);
        let r = x_packed.bitwise_or(&one_packed);
        let lv = r.to_logicvec();
        assert!(lv.bits.iter().all(|b| *b == LogicVal::One), "X | 1 should be 1, got {}", r);
    }

    #[test]
    fn test_bitwise_or_x_0() {
        // X | 0 = X
        let x_packed = PackedLogicVec::fill(LogicVal::X, 4);
        let zero_packed = PackedLogicVec::fill(LogicVal::Zero, 4);
        let r = x_packed.bitwise_or(&zero_packed);
        assert!(r.all_x(), "X | 0 should be X, got {}", r);
    }

    // ─── Reduction Tests ───

    #[test]
    fn test_red_and_all_ones() {
        let pv = PackedLogicVec::fill(LogicVal::One, 8);
        let r = pv.red_and();
        assert_eq!(r.to_u64(), 1, "red_and all ones should be 1, got {}", r.to_u64());
    }

    #[test]
    fn test_red_and_has_zero() {
        let pv = PackedLogicVec::from_u64(0b1110, 4);
        let r = pv.red_and();
        assert_eq!(r.to_u64(), 0);
    }

    #[test]
    fn test_red_or_all_zeros() {
        let pv = PackedLogicVec::fill(LogicVal::Zero, 8);
        let r = pv.red_or();
        assert_eq!(r.to_u64(), 0);
    }

    #[test]
    fn test_red_or_has_one() {
        let pv = PackedLogicVec::from_u64(0b0010, 4);
        let r = pv.red_or();
        assert_eq!(r.to_u64(), 1, "red_or with bit=1 should be 1, got {}", r.to_u64());
    }

    #[test]
    fn test_red_xor() {
        let pv = PackedLogicVec::from_u64(0b1010, 4);
        let r = pv.red_xor();
        // 1 xor 0 xor 1 xor 0 = 0
        assert_eq!(r.to_u64(), 0);
    }

    #[test]
    fn test_red_xor_odd() {
        let pv = PackedLogicVec::from_u64(0b1101, 4);
        let r = pv.red_xor();
        // 1 xor 1 xor 0 xor 1 = 1
        assert_eq!(r.to_u64(), 1, "red_xor of 1101 should be 1, got {}", r.to_u64());
    }

    #[test]
    fn test_red_nand() {
        let pv = PackedLogicVec::fill(LogicVal::One, 8);
        let r = pv.red_nand();
        assert_eq!(r.to_u64(), 0, "nand of all ones should be 0");
    }

    #[test]
    fn test_red_nor() {
        let pv = PackedLogicVec::fill(LogicVal::Zero, 8);
        let r = pv.red_nor();
        assert_eq!(r.to_u64(), 1, "nor of all zeros should be 1");
    }

    #[test]
    fn test_red_xnor() {
        let pv = PackedLogicVec::from_u64(0b1010, 4);
        let r = pv.red_xnor();
        assert_eq!(r.to_u64(), 1, "xnor of 1010 should be 1");
    }

    // ─── Shift Tests ───

    #[test]
    fn test_shl() {
        let pv = PackedLogicVec::from_u64(0b0001, 4);
        let r = pv.shl(2);
        assert_eq!(r.to_u64(), 0b0100);
    }

    #[test]
    fn test_shr() {
        let pv = PackedLogicVec::from_u64(0b1000, 4);
        let r = pv.shr(2);
        assert_eq!(r.to_u64(), 0b0010);
    }

    #[test]
    fn test_sshr_sign_extend() {
        // 4-bit: 1000 = -8 signed, shift right → 1110 = -2
        let pv = PackedLogicVec::from_u64(0b1000, 4);
        let r = pv.sshr(2);
        assert_eq!(format!("{}", r), "1110");
    }

    #[test]
    fn test_shl_full() {
        let pv = PackedLogicVec::from_u64(0b0001, 4);
        let r = pv.shl(4);
        assert_eq!(r.to_u64(), 0);
    }

    // ─── Comparison Tests ───

    #[test]
    fn test_eq_equal() {
        let a = PackedLogicVec::from_u64(0xAB, 8);
        let b = PackedLogicVec::from_u64(0xAB, 8);
        let r = a.eq(&b);
        assert_eq!(r.to_u64(), 1);
    }

    #[test]
    fn test_eq_not_equal() {
        let a = PackedLogicVec::from_u64(0xAB, 8);
        let b = PackedLogicVec::from_u64(0xCD, 8);
        let r = a.eq(&b);
        assert_eq!(r.to_u64(), 0);
    }

    #[test]
    fn test_eq_xz_match() {
        // X===X should be true, Z===Z should be true
        let x1 = PackedLogicVec::fill(LogicVal::X, 4);
        let x2 = PackedLogicVec::fill(LogicVal::X, 4);
        assert_eq!(x1.eq(&x2).to_u64(), 1, "X === X should be 1");
        
        let z1 = PackedLogicVec::fill(LogicVal::Z, 4);
        let z2 = PackedLogicVec::fill(LogicVal::Z, 4);
        assert_eq!(z1.eq(&z2).to_u64(), 1, "Z === Z should be 1");
        
        let r = x1.eq(&z1);
        assert_eq!(r.to_u64(), 0, "X === Z should be 0");
    }

    #[test]
    fn test_casex_eq() {
        let a = PackedLogicVec::from_u64(0b1010, 4);
        let pattern = PackedLogicVec::from_u64(0b1010, 4);
        let r = a.casex_eq(&pattern);
        assert_eq!(r.to_u64(), 1, "casex_eq with same value should be 1");
    }

    #[test]
    fn test_casex_eq_dont_care() {
        let a = PackedLogicVec::from_u64(0b1010, 4);
        // Pattern with X don't-care: 1x1x
        let pattern = PackedLogicVec::from_logicvec(&LogicVec {
            bits: vec![LogicVal::X, LogicVal::One, LogicVal::X, LogicVal::One],
            width: 4,
        });
        let r = a.casex_eq(&pattern);
        assert_eq!(r.to_u64(), 1, "casex_eq with X don't-care should be 1");
    }

    #[test]
    fn test_casez_eq() {
        let a = PackedLogicVec::from_u64(0b1010, 4);
        let pattern = PackedLogicVec::from_u64(0b1010, 4);
        let r = a.casez_eq(&pattern);
        assert_eq!(r.to_u64(), 1);
    }

    // ─── Conversion & Roundtrip Tests ───

    #[test]
    fn test_roundtrip_xz() {
        let lv = LogicVec {
            bits: vec![LogicVal::X, LogicVal::Z, LogicVal::X, LogicVal::Z],
            width: 4,
        };
        let packed = PackedLogicVec::from_logicvec(&lv);
        let recovered = packed.to_logicvec();
        assert_eq!(lv, recovered);
    }

    #[test]
    fn test_resize_truncate() {
        let pv = PackedLogicVec::from_u64(0b1111, 8);
        let r = pv.resize(4);
        assert_eq!(r.to_u64(), 0b1111);
        assert_eq!(r.width(), 4);
    }

    #[test]
    fn test_resize_extend() {
        let pv = PackedLogicVec::from_u64(0b1111, 4);
        let r = pv.resize(8);
        // Extended bits should be X (unknown)
        assert_eq!(r.to_u64(), 0b1111);
        let lv = r.to_logicvec();
        assert_eq!(lv.bits[4], LogicVal::X);
    }

    // ─── Display Tests ───

    #[test]
    fn test_display() {
        let pv = PackedLogicVec::from_logicvec(&LogicVec {
            bits: vec![LogicVal::One, LogicVal::Zero, LogicVal::X, LogicVal::Z],
            width: 4,
        });
        // LV bits[0]=One(LSB), [1]=Zero, [2]=X, [3]=Z(MSB)
        // Display MSB-first: Z X 0 1
        assert_eq!(format!("{}", pv), "zx01");
    }

    #[test]
    fn test_display_all_x() {
        let pv = PackedLogicVec::new(4);
        assert_eq!(format!("{}", pv), "xxxx");
    }

    // ─── Evaluator Tests ───

    #[test]
    fn test_eval_binary_packed_bitand() {
        let a = PackedLogicVec::from_u64(0xFF, 8);
        let b = PackedLogicVec::from_u64(0x0F, 8);
        let r = eval_binary_packed(&BinaryIrOp::BitAnd, &a, &b).unwrap();
        assert_eq!(r.to_u64(), 0x0F);
    }

    #[test]
    fn test_eval_binary_packed_bitor() {
        let a = PackedLogicVec::from_u64(0xF0, 8);
        let b = PackedLogicVec::from_u64(0x0F, 8);
        let r = eval_binary_packed(&BinaryIrOp::BitOr, &a, &b).unwrap();
        assert_eq!(r.to_u64(), 0xFF);
    }

    #[test]
    fn test_eval_unary_packed_bitnot() {
        let a = PackedLogicVec::from_u64(0xFF, 8);
        let r = eval_unary_packed(&UnaryIrOp::BitNot, &a).unwrap();
        assert_eq!(r.to_u64(), 0x00);
    }

    #[test]
    fn test_eval_unary_packed_red_and() {
        let a = PackedLogicVec::fill(LogicVal::One, 8);
        let r = eval_unary_packed(&UnaryIrOp::RedAnd, &a).unwrap();
        assert_eq!(r.to_u64(), 1, "red_and all ones via eval should be 1, got {}", r.to_u64());
    }

    #[test]
    fn test_eval_unary_packed_red_or() {
        let a = PackedLogicVec::from_u64(0x00, 8);
        let r = eval_unary_packed(&UnaryIrOp::RedOr, &a).unwrap();
        assert_eq!(r.to_u64(), 0);
    }

    #[test]
    fn test_eval_unary_packed_logical_not() {
        let a = PackedLogicVec::from_u64(1, 1);
        let r = eval_unary_packed(&UnaryIrOp::Not, &a).unwrap();
        assert_eq!(r.to_u64(), 0);
    }

    #[test]
    fn test_eval_binary_packed_eq() {
        // Eq is now handled by JIT/interpreted, not packed eval
        let a = PackedLogicVec::from_u64(42, 8);
        let b = PackedLogicVec::from_u64(42, 8);
        let r = eval_binary_packed(&BinaryIrOp::Eq, &a, &b);
        assert!(r.is_none(), "Eq should return None (not packable)");
    }

    #[test]
    fn test_eval_binary_packed_eq_wild() {
        // EqWild is now handled by JIT/interpreted, not packed eval
        let a = PackedLogicVec::from_u64(0b1010, 4);
        let pattern = PackedLogicVec::from_u64(0b1010, 4);
        let r = eval_binary_packed(&BinaryIrOp::EqWild, &a, &pattern);
        assert!(r.is_none(), "EqWild should return None (not packable)");
    }

    // ─── Cross-validation with LogicVec ───

    fn cross_validate_binary(op: BinaryIrOp) {
        let a_lv = LogicVec::from_u64(0xACE, 12);
        let b_lv = LogicVec::from_u64(0xDEF, 12);
        let a_pv = PackedLogicVec::from_logicvec(&a_lv);
        let b_pv = PackedLogicVec::from_logicvec(&b_lv);
        
        let lv_result = crate::simulator::value::eval_binary(op.clone(), &a_lv, &b_lv);
        let pv_result = eval_binary_packed_extended(&op, &a_pv, &b_pv).to_logicvec();
        
        assert_eq!(lv_result, pv_result, "Mismatch for op {:?}", op);
    }

    #[test]
    fn test_cross_validate_bitwise_and() {
        cross_validate_binary(BinaryIrOp::BitAnd);
    }

    #[test]
    fn test_cross_validate_bitwise_or() {
        cross_validate_binary(BinaryIrOp::BitOr);
    }

    #[test]
    fn test_cross_validate_bitwise_xor() {
        cross_validate_binary(BinaryIrOp::BitXor);
    }

    #[test]
    fn test_cross_validate_bitwise_xnor() {
        cross_validate_binary(BinaryIrOp::BitXnor);
    }

    #[test]
    fn test_cross_validate_eq() {
        cross_validate_binary(BinaryIrOp::Eq);
    }

    #[test]
    fn test_cross_validate_neq() {
        cross_validate_binary(BinaryIrOp::Neq);
    }

    #[test]
    fn test_cross_validate_bitwise_not() {
        let lv = LogicVec::from_u64(0xACE, 12);
        let pv = PackedLogicVec::from_logicvec(&lv);
        
        let lv_result = crate::simulator::value::eval_unary(UnaryIrOp::BitNot, &lv);
        let pv_result = eval_unary_packed(&UnaryIrOp::BitNot, &pv).unwrap().to_logicvec();
        
        assert_eq!(lv_result, pv_result);
    }

    #[test]
    fn test_cross_validate_red_and() {
        let lv = LogicVec::from_u64(0xFF, 8);
        let pv = PackedLogicVec::from_logicvec(&lv);
        
        let lv_result = crate::simulator::value::eval_unary(UnaryIrOp::RedAnd, &lv);
        let pv_result = eval_unary_packed(&UnaryIrOp::RedAnd, &pv).unwrap().to_logicvec();
        
        assert_eq!(lv_result, pv_result, "red_and cross: LV={:?}, PV={:?}", lv_result, pv_result);
    }

    #[test]
    fn test_cross_validate_red_or() {
        let lv = LogicVec::from_u64(0xF0, 8);
        let pv = PackedLogicVec::from_logicvec(&lv);
        
        let lv_result = crate::simulator::value::eval_unary(UnaryIrOp::RedOr, &lv);
        let pv_result = eval_unary_packed(&UnaryIrOp::RedOr, &pv).unwrap().to_logicvec();
        
        assert_eq!(lv_result, pv_result);
    }

    #[test]
    fn test_cross_validate_red_xor() {
        let lv = LogicVec::from_u64(0xAA, 8);
        let pv = PackedLogicVec::from_logicvec(&lv);
        
        let lv_result = crate::simulator::value::eval_unary(UnaryIrOp::RedXor, &lv);
        let pv_result = eval_unary_packed(&UnaryIrOp::RedXor, &pv).unwrap().to_logicvec();
        
        assert_eq!(lv_result, pv_result);
    }

    #[test]
    fn test_cross_validate_wide_signals() {
        // 128-bit signal
        let lv_a = LogicVec::from_u64(0xABCD_EF01, 128);
        let lv_b = LogicVec::from_u64(0xDEAD_BEEF, 128);
        let pv_a = PackedLogicVec::from_logicvec(&lv_a);
        let pv_b = PackedLogicVec::from_logicvec(&lv_b);
        
        let lv_result = crate::simulator::value::eval_binary(BinaryIrOp::BitXor, &lv_a, &lv_b);
        let pv_result = pv_a.bitwise_xor(&pv_b).to_logicvec();
        
        assert_eq!(lv_result.width, pv_result.width);
        assert_eq!(lv_result, pv_result);
    }

    // ─── Edge Cases ───

    #[test]
    fn test_all_x_empty() {
        let pv = PackedLogicVec::new(0);
        assert_eq!(pv.width(), 1);
        assert!(pv.all_x());
    }

    #[test]
    fn test_all_z_check() {
        let pv = PackedLogicVec::fill(LogicVal::Z, 8);
        assert!(pv.all_z(), "fill(Z, 8).all_z() should be true, got {}", pv);
    }

    #[test]
    fn test_to_bool_known() {
        let pv = PackedLogicVec::from_u64(1, 1);
        assert_eq!(pv.to_bool(), Some(true));
        
        let pv = PackedLogicVec::from_u64(0, 1);
        assert_eq!(pv.to_bool(), Some(false));
    }

    #[test]
    fn test_to_bool_x() {
        let pv = PackedLogicVec::fill(LogicVal::X, 1);
        assert_eq!(pv.to_bool(), None);
    }

    #[test]
    fn test_extend() {
        let a = PackedLogicVec::from_u64(0xFF, 8);
        let b = PackedLogicVec::from_u64(0xAA, 8);
        let r = a.extend(&b);
        assert_eq!(r.width(), 16);
        let lv = r.to_logicvec();
        assert_eq!(lv.bits[0], LogicVal::One); // LSB from a
    }

    #[test]
    fn test_eq_different_widths() {
        let a = PackedLogicVec::from_u64(0xFF, 8);
        let b = PackedLogicVec::from_u64(0x00FF, 16);
        // Different widths should be NOT equal (=== semantics)
        let r = a.eq(&b);
        assert_eq!(r.to_u64(), 0, "diff widths should not be equal");
    }

    #[test]
    fn test_resize_identity() {
        let pv = PackedLogicVec::from_u64(0xABCD, 16);
        let r = pv.resize(16);
        assert_eq!(pv, r);
    }

    #[test]
    fn test_fill_x_then_to_u64() {
        let pv = PackedLogicVec::fill(LogicVal::X, 32);
        assert_eq!(pv.to_u64(), 0, "X fill should give 0 from to_u64");
    }

    // ─── Integration: cross-validate ALL ops via eval_binary_packed ───

    #[test]
    fn test_is_packable_binary_op_true() {
        assert!(is_packable_binary_op(&BinaryIrOp::BitAnd));
        assert!(is_packable_binary_op(&BinaryIrOp::BitOr));
        assert!(is_packable_binary_op(&BinaryIrOp::BitXor));
        assert!(is_packable_binary_op(&BinaryIrOp::BitXnor));
        assert!(!is_packable_binary_op(&BinaryIrOp::Eq)); // Eq now handled by JIT
        assert!(!is_packable_binary_op(&BinaryIrOp::Neq)); // Neq now handled by JIT
    }

    #[test]
    fn test_is_packable_binary_op_false() {
        assert!(!is_packable_binary_op(&BinaryIrOp::Add));
        assert!(!is_packable_binary_op(&BinaryIrOp::Sub));
        assert!(!is_packable_binary_op(&BinaryIrOp::Mul));
        assert!(!is_packable_binary_op(&BinaryIrOp::Div));
        assert!(!is_packable_binary_op(&BinaryIrOp::Shl));
        assert!(!is_packable_binary_op(&BinaryIrOp::Shr));
        assert!(!is_packable_binary_op(&BinaryIrOp::Lt));
        assert!(!is_packable_binary_op(&BinaryIrOp::Gt));
        assert!(!is_packable_binary_op(&BinaryIrOp::LogicalAnd));
        assert!(!is_packable_binary_op(&BinaryIrOp::LogicalOr));
    }

    /// Stress test: verify ALL bitwise ops produce identical results
    /// between classic LogicVec eval and packed eval for random 64-bit values.
    #[test]
    fn test_stress_cross_validate_all_bitwise_ops() {
        use rand::Rng;
        let mut rng = rand::thread_rng();

        let ops = [
            BinaryIrOp::BitAnd,
            BinaryIrOp::BitOr,
            BinaryIrOp::BitXor,
            BinaryIrOp::BitXnor,
        ];

        for _ in 0..100 {
            let a_val: u64 = rng.gen();
            let b_val: u64 = rng.gen();
            let width: usize = rng.gen_range(1..=64);

            let lv_a = LogicVec::from_u64(a_val, width);
            let lv_b = LogicVec::from_u64(b_val, width);
            let pv_a = PackedLogicVec::from_u64(a_val, width);
            let pv_b = PackedLogicVec::from_u64(b_val, width);

            for op in &ops {
                let lv_result = crate::simulator::value::eval_binary(op.clone(), &lv_a, &lv_b);
                let pv_result = eval_binary_packed(op, &pv_a, &pv_b)
                    .expect("packed eval returned None for bitwise op")
                    .to_logicvec();
                assert_eq!(
                    lv_result, pv_result,
                    "Mismatch for op={:?} a=0x{:X} b=0x{:X} width={}",
                    op, a_val, b_val, width
                );
            }
        }
    }

    /// Verify packed eval produces identical results as regular eval for a simple design.
    /// Creates two engines (packed on/off) and compares signal values.
    #[test]
    fn test_simulation_equivalence_packed_vs_regular() {
        let source = r#"
module top;
    reg [7:0] a, b, c_and, c_or, c_xor, c_xnor;
    reg eq_flag, neq_flag;
    initial begin
        a = 8'hA5;
        b = 8'h5A;
        c_and = a & b;
        c_or  = a | b;
        c_xor = a ^ b;
        c_xnor = ~(a ^ b);
        eq_flag = (a == b);
        neq_flag = (a != b);
        #1 $finish;
    end
endmodule
"#;
        use crate::compile_str;
        use crate::simulator::SimulationEngine;

        let design = compile_str(source).unwrap();

        // Run with regular eval
        let mut engine_regular = SimulationEngine::new(design.clone(), 10);
        engine_regular.use_packed_eval = false;
        engine_regular.run().unwrap();

        // Run with packed eval
        let mut engine_packed = SimulationEngine::new(design, 10);
        engine_packed.use_packed_eval = true;
        engine_packed.run().unwrap();

        // Compare all signal values
        for (i, sig) in engine_regular.design.top.signals.iter().enumerate() {
            let regular_val = engine_regular.state.read_signal(i).clone();
            let packed_val = engine_packed.state.read_signal(i).clone();
            assert_eq!(
                regular_val, packed_val,
                "Signal '{}' mismatch: regular={} packed={}",
                sig.name, regular_val, packed_val
            );
        }
    }

    #[test]
    fn test_all_x_partial_chunk() {
        // 7-bit signal (not aligned to 64)
        let pv = PackedLogicVec::new(7);
        assert!(pv.all_x(), "7-bit new should be all X");
        assert_eq!(pv.width(), 7);
        assert_eq!(format!("{}", pv), "xxxxxxx");
    }
}
