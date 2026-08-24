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

use maria_ir::{BinaryIrOp, LogicVal, LogicVec};

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
        let num_chunks = w.div_ceil(CELLS_PER_CHUNK);
        // X = known:0, value:0
        let chunks = vec![(0u64, 0u64); num_chunks];
        PackedLogicVec { chunks, width: w }
    }

    /// Create from a single bit value with given width.
    pub fn fill(val: LogicVal, width: usize) -> Self {
        let w = width.max(1);
        let num_chunks = w.div_ceil(CELLS_PER_CHUNK);
        let mut chunks = Vec::with_capacity(num_chunks);
        for c in 0..num_chunks {
            let chunk_width = (w - c * CELLS_PER_CHUNK).min(CELLS_PER_CHUNK);
            let mask = if chunk_width >= 64 {
                !0u64
            } else {
                (1u64 << chunk_width) - 1
            };
            let (known, value) = match val {
                LogicVal::X => (0u64, 0u64),
                LogicVal::Z => (0u64, mask),    // all bits = Z
                LogicVal::Zero => (mask, 0u64), // all bits = 0 (known)
                LogicVal::One => (mask, mask),  // all bits = 1 (known)
            };
            chunks.push((known, value));
        }
        PackedLogicVec { chunks, width: w }
    }

    /// Create from a u64 value with given width (0/1 bits only).
    /// Untuk sinyal >64 bit, hanya lower 64 bit yang diisi; sisanya X.
    pub fn from_u64(val: u64, width: usize) -> Self {
        let w = width.max(1);
        let num_chunks = w.div_ceil(CELLS_PER_CHUNK);
        let mut chunks = Vec::with_capacity(num_chunks);
        for c in 0..num_chunks {
            let bit_offset = c * CELLS_PER_CHUNK;
            let chunk_width = (w - bit_offset).min(CELLS_PER_CHUNK);
            let mask = if chunk_width >= 64 {
                !0u64
            } else {
                (1u64 << chunk_width) - 1
            };
            let shifted = if bit_offset < 64 {
                val >> bit_offset
            } else {
                0
            };
            let value = shifted & mask;
            let known = mask; // All bits are known 0/1
            chunks.push((known, value));
        }
        PackedLogicVec { chunks, width: w }
    }

    /// Convert from a traditional LogicVec (Vec<LogicVal> based).
    pub fn from_logicvec(lv: &LogicVec) -> Self {
        let w = lv.width.max(1);
        let num_chunks = w.div_ceil(CELLS_PER_CHUNK);
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
        if last_bit == 0 {
            !0u64
        } else {
            (1u64 << last_bit) - 1
        }
    }

    /// Apply width mask to all chunks.
    fn apply_mask(&self) -> Vec<(u64, u64)> {
        let last_idx = self.chunks.len().saturating_sub(1);
        let mask = self.last_chunk_mask();
        self.chunks
            .iter()
            .enumerate()
            .map(|(i, &(k, v))| {
                if i == last_idx && mask != !0u64 {
                    (k & mask, v & mask)
                } else {
                    (k, v)
                }
            })
            .collect()
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
        masked
            .iter()
            .all(|&(known, value)| known == 0 && value == 0)
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
        let width_mask = if chunk_width >= 64 {
            !0u64
        } else {
            (1u64 << chunk_width) - 1
        };
        (value & known) & width_mask
    }

    /// Convert to bool (1-bit interpretation).
    pub fn to_bool(&self) -> Option<bool> {
        if self.width == 0 {
            return Some(false);
        }
        let (known0, value0) = self.chunks[0];
        let chunk_width = self.width.min(CELLS_PER_CHUNK);
        let width_mask = if chunk_width >= 64 {
            !0u64
        } else {
            (1u64 << chunk_width) - 1
        };
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
        if w <= self.width {
            // Truncate: just mask the excess bits
            let mut chunks = self.chunks.clone();
            let last_bit = w % CELLS_PER_CHUNK;
            if last_bit > 0 {
                let last_idx = chunks.len() - 1;
                let mask = (1u64 << last_bit) - 1;
                chunks[last_idx].0 &= mask;
                chunks[last_idx].1 &= mask;
            }
            // Remove any chunks beyond the new width
            let new_chunks = w.div_ceil(CELLS_PER_CHUNK);
            chunks.truncate(new_chunks);
            PackedLogicVec { chunks, width: w }
        } else {
            // Extend: zero-extend (fill new bits with known=1, value=0)
            let old_width = self.width;
            let new_chunks = w.div_ceil(CELLS_PER_CHUNK);
            let mut chunks = self.chunks.clone();
            chunks.resize(new_chunks, (0u64, 0u64));
            // Fill the new bits in the last old chunk with zeros
            let old_last_bit = old_width % CELLS_PER_CHUNK;
            if old_last_bit > 0 {
                let last_idx = old_width / CELLS_PER_CHUNK;
                if last_idx < chunks.len() {
                    let old_mask = (1u64 << old_last_bit) - 1;
                    let new_mask = if CELLS_PER_CHUNK >= 64 {
                        !0u64
                    } else {
                        (1u64 << CELLS_PER_CHUNK) - 1
                    };
                    let extend_mask = new_mask & !old_mask;
                    // Set known=1, value=0 for new bits
                    chunks[last_idx].0 |= extend_mask;
                    // value bits already 0
                }
            }
            // New chunks are already (0, 0) = X, need to make them zero
            for i in (old_width / CELLS_PER_CHUNK + 1)..new_chunks {
                let chunk_width = (w - i * CELLS_PER_CHUNK).min(CELLS_PER_CHUNK);
                let mask = if chunk_width >= 64 {
                    !0u64
                } else {
                    (1u64 << chunk_width) - 1
                };
                chunks[i] = (mask, 0u64); // known=1, value=0 for all bits
            }
            // Mask the last chunk
            let last_bit = w % CELLS_PER_CHUNK;
            if last_bit > 0 {
                let last_idx = chunks.len() - 1;
                let mask = (1u64 << last_bit) - 1;
                chunks[last_idx].0 &= mask;
                chunks[last_idx].1 &= mask;
            }
            PackedLogicVec { chunks, width: w }
        }
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
        let self_ext = self.resize(max_width);
        let other_ext = other.resize(max_width);
        let chunks = crate::simulator::simd_packed::simd_and(&self_ext.chunks, &other_ext.chunks);
        PackedLogicVec {
            chunks,
            width: max_width,
        }
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
        let self_ext = self.resize(max_width);
        let other_ext = other.resize(max_width);
        let chunks = crate::simulator::simd_packed::simd_or(&self_ext.chunks, &other_ext.chunks);
        PackedLogicVec {
            chunks,
            width: max_width,
        }
    }

    /// Bitwise XOR — 4-state correct.
    ///
    /// Truth table: X/Z if either input is X/Z, normal XOR otherwise.
    /// Formula per bit:
    /// - known = a.known & b.known
    /// - value = (a.value ^ b.value) & known
    pub fn bitwise_xor(&self, other: &PackedLogicVec) -> PackedLogicVec {
        let max_width = self.width.max(other.width);
        let self_ext = self.resize(max_width);
        let other_ext = other.resize(max_width);
        let chunks = crate::simulator::simd_packed::simd_xor(&self_ext.chunks, &other_ext.chunks);
        PackedLogicVec {
            chunks,
            width: max_width,
        }
    }

    /// Bitwise XNOR = NOT(XOR).
    pub fn bitwise_xnor(&self, other: &PackedLogicVec) -> PackedLogicVec {
        let max_width = self.width.max(other.width);
        let self_ext = self.resize(max_width);
        let other_ext = other.resize(max_width);
        let xor = self_ext.bitwise_xor(&other_ext);
        xor.bitwise_not()
    }

    /// Bitwise NOT — 2 operasi per chunk.
    ///
    /// Formula: known unchanged, value flipped where known.
    /// X→X, Z→Z, 0→1, 1→0
    pub fn bitwise_not(&self) -> PackedLogicVec {
        let chunks = crate::simulator::simd_packed::simd_not(&self.chunks);
        PackedLogicVec {
            chunks,
            width: self.width,
        }
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
            xor_acc ^= vm & km;
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
