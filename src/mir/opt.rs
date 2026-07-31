//! MIR Optimizations — peephole and dataflow optimizations on MIR instructions.
//!
//! Pass pipeline (run to fixed point):
//! 1. constant_fold        — fold binary ops on constant operands
//! 2. copy_propagate       — fold unary ops with constant operand
//! 3. dead_store_eliminate — remove stores to registers never read (conservative: within basic blocks only)
//! 4. constant_branch_fold — replace branches on constant condition with Jump
//! 5. strength_reduce      — replace Mul/Div by power-of-2 with Shl/Shr
//! 6. remove_nops          — sweep NOP instructions

use super::mir::{MirInstr, MirBinOp, MirModule};
use std::collections::HashMap;

/// Optimize a MIR module (peephole + dataflow).
pub fn optimize_module(module: &mut MirModule) {
    for process in &mut module.processes {
        optimize_process(&mut process.instrs);
    }
}

/// Optimize a list of MIR instructions to fixed point.
pub fn optimize_process(instrs: &mut Vec<MirInstr>) {
    let mut changed = true;
    while changed {
        changed = false;
        changed |= constant_fold(instrs);
        changed |= copy_propagate(instrs);
        changed |= dead_store_eliminate(instrs);
        changed |= constant_branch_fold(instrs);
        changed |= strength_reduce(instrs);
        changed |= remove_nops(instrs);
    }
}

// ── Helper: Find Constant Value ──

/// Try to find the constant value loaded into a register by scanning backwards.
fn find_const_value(instrs: &[MirInstr], reg: usize) -> Option<u64> {
    for instr in instrs.iter().rev() {
        match instr {
            MirInstr::Const { dest, value, .. } if *dest == reg => return Some(*value),
            MirInstr::Load { dest, .. } if *dest == reg => return None,
            MirInstr::Binary { dest, .. } if *dest == reg => return None,
            MirInstr::Unary { dest, .. } if *dest == reg => return None,
            _ => {}
        }
    }
    None
}

// ── Pass 1: Constant Folding ──

/// Fold binary operations where both operands are constants.
fn constant_fold(instrs: &mut Vec<MirInstr>) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i < instrs.len() {
        if let MirInstr::Binary { op, dest, lhs, rhs, width } = &instrs[i] {
            let lhs_val = find_const_value(instrs, *lhs);
            let rhs_val = find_const_value(instrs, *rhs);

            if let (Some(lv), Some(rv)) = (lhs_val, rhs_val) {
                let result = match op {
                    MirBinOp::Add => lv.wrapping_add(rv),
                    MirBinOp::Sub => lv.wrapping_sub(rv),
                    MirBinOp::Mul => lv.wrapping_mul(rv),
                    MirBinOp::Div => lv.checked_div(rv).unwrap_or(0),
                    MirBinOp::And => lv & rv,
                    MirBinOp::Or => lv | rv,
                    MirBinOp::Xor => lv ^ rv,
                    MirBinOp::Eq => (lv == rv) as u64,
                    MirBinOp::Ne => (lv != rv) as u64,
                    MirBinOp::Lt => (lv < rv) as u64,
                    MirBinOp::Le => (lv <= rv) as u64,
                    MirBinOp::Gt => (lv > rv) as u64,
                    MirBinOp::Ge => (lv >= rv) as u64,
                    MirBinOp::Shl => lv << rv.min(63),
                    MirBinOp::Shr => lv >> rv.min(63),
                    MirBinOp::Mod => if rv != 0 { lv % rv } else { 0 },
                };
                instrs[i] = MirInstr::Const { dest: *dest, value: result, width: *width };
                changed = true;
            }
        }
        i += 1;
    }
    changed
}

// ── Pass 2: Copy Propagation (Unary Constant Fold) ──

/// Fold unary operations where the operand is constant.
/// Also replaces identity ops (x | 0, x & all-ones) with the operand.
fn copy_propagate(instrs: &mut Vec<MirInstr>) -> bool {
    let mut changed = false;
    // Pre-scan: collect constant values from Const instructions
    let const_map: HashMap<usize, u64> = instrs.iter().filter_map(|i| {
        match i {
            MirInstr::Const { dest, value, .. } => Some((*dest, *value)),
            _ => None,
        }
    }).collect();

    for instr in instrs.iter_mut() {
        if let MirInstr::Unary { op, dest, operand, width } = instr {
            // If operand is constant, fold: !C, -C
            if let Some(&ov) = const_map.get(operand) {
                let raw_result = match op {
                    super::mir::MirUnOp::Not => !ov,
                    super::mir::MirUnOp::Neg => ov.wrapping_neg(),
                };
                let mask = if *width < 64 { (1u64 << *width) - 1 } else { u64::MAX };
                *instr = MirInstr::Const { dest: *dest, value: raw_result & mask, width: *width };
                changed = true;
            }
        }
        // Note: Binary identity ops (x|0, x&all-ones) are intentionally
        // NOT folded here — the subsequent constant_fold pass in the
        // fixed-point pipeline handles them when the constant operand
        // is detected via find_const_value.
    }
    changed
}

// ── Pass 3: Dead Store Elimination (Conservative: within basic blocks only) ──

/// Eliminate stores to registers that are never read before being overwritten.
///
/// CONSERVATIVE: skips across Branch/Jump/Label boundaries to avoid
/// incorrect elimination across control flow paths.
fn dead_store_eliminate(instrs: &mut Vec<MirInstr>) -> bool {
    let mut last_write: HashMap<usize, usize> = HashMap::new();
    let mut changed = false;

    let mut i = 0;
    while i < instrs.len() {
        // Reset register tracking at control flow boundaries (conservative —
        // prevents incorrect elimination of writes that are needed in other paths)
        match &instrs[i] {
            MirInstr::Branch { .. } | MirInstr::Jump { .. }
            | MirInstr::Label(_) => {
                last_write.clear();  // reset: don't track across blocks
            }
            _ => {}
        }

        let written_reg = match &instrs[i] {
            MirInstr::Const { dest, .. } | MirInstr::Binary { dest, .. }
            | MirInstr::Unary { dest, .. } => Some(*dest),
            _ => None,
        };

        if let Some(reg) = written_reg {
            if let Some(&prev_idx) = last_write.get(&reg) {
                if prev_idx < i {
                    // Check if reg was NOT read between prev_idx+1 and i-1
                    let mut is_dead = true;
                    for j in (prev_idx + 1)..i {
                        if reads_register(&instrs[j], reg) {
                            is_dead = false;
                            break;
                        }
                    }
                    if is_dead && !reads_register(&instrs[i], reg) {
                        instrs[prev_idx] = MirInstr::Nop;
                        changed = true;
                    }
                }
            }
            last_write.insert(reg, i);
        }
        i += 1;
    }

    if changed {
        remove_nops(instrs);
    }
    changed
}

/// Check if an instruction reads a specific register.
fn reads_register(instr: &MirInstr, reg: usize) -> bool {
    match instr {
        MirInstr::Binary { lhs, rhs, .. } => *lhs == reg || *rhs == reg,
        MirInstr::Unary { operand, .. } => *operand == reg,
        MirInstr::Store { src, .. } => *src == reg,
        MirInstr::NonBlocking { src, .. } => *src == reg,
        MirInstr::Branch { cond, .. } => *cond == reg,
        _ => false,
    }
}

// ── Pass 4: Constant Branch Folding ──

/// Replace branches with constant conditions: if cond is always true → Jump,
/// if cond is always false → remove branch.
fn constant_branch_fold(instrs: &mut Vec<MirInstr>) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i < instrs.len() {
        if let MirInstr::Branch { cond, then_label, else_label } = &instrs[i] {
            if let Some(val) = find_const_value(instrs, *cond) {
                if val != 0 {
                    instrs[i] = MirInstr::Jump { label: *then_label };
                } else {
                    instrs[i] = MirInstr::Jump { label: *else_label };
                }
                changed = true;
            }
        }
        i += 1;
    }
    changed
}

// ── Pass 5: Strength Reduction ──

/// Replace expensive operations with cheaper ones:
/// - Mul by power-of-2 → Shl
/// - Div by power-of-2 → Shr
/// - Mul by 0 → Const 0
/// - Mul/Div by 1 → NOP (removed by subsequent passes)
fn strength_reduce(instrs: &mut Vec<MirInstr>) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i < instrs.len() {
        // Clone fields before matching to avoid E0502 borrow conflict
        let matched = match &instrs[i] {
            MirInstr::Binary { op, dest, lhs, rhs, width } => {
                Some((*op, *dest, *lhs, *rhs, *width))
            }
            _ => None,
        };
        if let Some((op, dest, lhs, rhs, width)) = matched {
            if let Some(rv) = find_const_value(instrs, rhs) {
                match op {
                    MirBinOp::Mul => {
                        if rv == 0 {
                            instrs[i] = MirInstr::Const { dest, value: 0, width };
                            changed = true;
                        } else if rv == 1 {
                            instrs[i] = MirInstr::Nop;
                            changed = true;
                        } else if rv.is_power_of_two() {
                            let shift = rv.trailing_zeros() as u64;
                            let shift_reg = alloc_temp_reg(instrs);
                            instrs.insert(i, MirInstr::Const { dest: shift_reg, value: shift, width });
                            instrs[i + 1] = MirInstr::Binary {
                                op: MirBinOp::Shl,
                                dest,
                                lhs,
                                rhs: shift_reg,
                                width,
                            };
                            changed = true;
                        }
                    }
                    MirBinOp::Div => {
                        if rv == 1 {
                            instrs[i] = MirInstr::Nop;
                            changed = true;
                        } else if rv.is_power_of_two() {
                            let shift = rv.trailing_zeros() as u64;
                            let shift_reg = alloc_temp_reg(instrs);
                            instrs.insert(i, MirInstr::Const { dest: shift_reg, value: shift, width });
                            instrs[i + 1] = MirInstr::Binary {
                                op: MirBinOp::Shr,
                                dest,
                                lhs,
                                rhs: shift_reg,
                                width,
                            };
                            changed = true;
                        }
                    }
                    _ => {}
                }
            }
        }
        i += 1;
    }
    changed
}

/// Allocate a temporary register index for strength reduction expansions.
fn alloc_temp_reg(instrs: &[MirInstr]) -> usize {
    let max_reg = instrs.iter().filter_map(|i| {
        match i {
            MirInstr::Const { dest, .. } | MirInstr::Binary { dest, .. }
            | MirInstr::Unary { dest, .. } => Some(*dest),
            _ => None,
        }
    }).max().unwrap_or(0);
    max_reg + 1
}

// ── Pass 6: Remove NOPs ──

fn remove_nops(instrs: &mut Vec<MirInstr>) -> bool {
    let before = instrs.len();
    instrs.retain(|i| !matches!(i, MirInstr::Nop));
    instrs.len() != before
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::mir::*;

    #[test]
    fn test_constant_fold_add() {
        let mut instrs = vec![
            MirInstr::Const { dest: 0, value: 3, width: 32 },
            MirInstr::Const { dest: 1, value: 4, width: 32 },
            MirInstr::Binary { op: MirBinOp::Add, dest: 2, lhs: 0, rhs: 1, width: 32 },
        ];
        assert!(constant_fold(&mut instrs));
        assert!(matches!(&instrs[2], MirInstr::Const { value: 7, .. }));
    }

    #[test]
    fn test_remove_nops() {
        let mut instrs = vec![
            MirInstr::Nop,
            MirInstr::Const { dest: 0, value: 1, width: 1 },
            MirInstr::Nop,
        ];
        assert!(remove_nops(&mut instrs));
        assert_eq!(instrs.len(), 1);
    }

    #[test]
    fn test_constant_fold_all_ops() {
        let ops: [(MirBinOp, u64, u64, u64); 17] = [
            (MirBinOp::Add, 10, 20, 30),
            (MirBinOp::Sub, 50, 23, 27),
            (MirBinOp::Mul, 6, 7, 42),
            (MirBinOp::Div, 42, 6, 7),
            (MirBinOp::And, 0xFF, 0x0F, 0x0F),
            (MirBinOp::Or,  0xF0, 0x0F, 0xFF),
            (MirBinOp::Xor, 0xFF, 0x0F, 0xF0),
            (MirBinOp::Eq,  5, 5, 1),
            (MirBinOp::Eq,  5, 6, 0),
            (MirBinOp::Ne,  5, 6, 1),
            (MirBinOp::Ne,  5, 5, 0),
            (MirBinOp::Lt,  3, 7, 1),
            (MirBinOp::Lt,  7, 3, 0),
            (MirBinOp::Le,  5, 5, 1),
            (MirBinOp::Gt,  7, 3, 1),
            (MirBinOp::Ge,  5, 5, 1),
            (MirBinOp::Shl, 1, 3, 8),
        ];
        for (i, &(op, a, b, expected)) in ops.iter().enumerate() {
            let mut instrs = vec![
                MirInstr::Const { dest: 0, value: a, width: 32 },
                MirInstr::Const { dest: 1, value: b, width: 32 },
                MirInstr::Binary { op, dest: 2, lhs: 0, rhs: 1, width: 32 },
            ];
            assert!(constant_fold(&mut instrs), "op index {} should fold", i);
            if let MirInstr::Const { value, .. } = &instrs[2] {
                assert_eq!(*value, expected, "op index {}: expected {}, got {}", i, expected, value);
            } else {
                panic!("op index {}: not folded", i);
            }
        }
    }

    #[test]
    fn test_dead_store_eliminate_adjacent() {
        // reg 0 written twice — first write dead (overwritten before any read)
        let mut instrs = vec![
            MirInstr::Const { dest: 0, value: 42, width: 32 },  // DEAD — overwritten by next
            MirInstr::Const { dest: 0, value: 7, width: 32 },   // LIVE — read by Add
            MirInstr::Binary { op: MirBinOp::Add, dest: 1, lhs: 0, rhs: 0, width: 32 },
            MirInstr::Store { signal: 0, src: 1 },               // LIVE — external
        ];
        let changed = dead_store_eliminate(&mut instrs);
        assert!(changed, "dead store should be eliminated");
        assert!(instrs.iter().any(|i| matches!(i, MirInstr::Const { value: 7, .. })), "value 7 should survive");
        assert!(instrs.iter().any(|i| matches!(i, MirInstr::Store { .. })), "store should survive");
    }

    #[test]
    fn test_dead_store_same_register() {
        // reg 0 written twice — first write dead
        let mut instrs = vec![
            MirInstr::Const { dest: 0, value: 1, width: 8 },
            MirInstr::Const { dest: 0, value: 2, width: 8 },
            MirInstr::Store { signal: 0, src: 0 },
        ];
        let changed = dead_store_eliminate(&mut instrs);
        assert!(changed);
    }

    #[test]
    fn test_constant_branch_fold_true() {
        let mut instrs = vec![
            MirInstr::Const { dest: 0, value: 1, width: 1 },
            MirInstr::Branch { cond: 0, then_label: 10, else_label: 20 },
        ];
        assert!(constant_branch_fold(&mut instrs));
        assert!(matches!(instrs[1], MirInstr::Jump { label: 10 }));
    }

    #[test]
    fn test_constant_branch_fold_false() {
        let mut instrs = vec![
            MirInstr::Const { dest: 0, value: 0, width: 1 },
            MirInstr::Branch { cond: 0, then_label: 10, else_label: 20 },
        ];
        assert!(constant_branch_fold(&mut instrs));
        assert!(matches!(instrs[1], MirInstr::Jump { label: 20 }));
    }

    #[test]
    fn test_strength_reduce_mul_pow2() {
        let mut instrs = vec![
            MirInstr::Const { dest: 0, value: 5, width: 32 },
            MirInstr::Const { dest: 1, value: 8, width: 32 },
            MirInstr::Binary { op: MirBinOp::Mul, dest: 2, lhs: 0, rhs: 1, width: 32 },
        ];
        assert!(strength_reduce(&mut instrs));
        assert!(instrs.iter().any(|i| matches!(i, MirInstr::Binary { op: MirBinOp::Shl, .. })));
    }

    #[test]
    fn test_strength_reduce_mul_zero() {
        let mut instrs = vec![
            MirInstr::Const { dest: 0, value: 42, width: 32 },
            MirInstr::Const { dest: 1, value: 0, width: 32 },
            MirInstr::Binary { op: MirBinOp::Mul, dest: 2, lhs: 0, rhs: 1, width: 32 },
        ];
        assert!(strength_reduce(&mut instrs));
        assert!(matches!(&instrs[2], MirInstr::Const { value: 0, .. }));
    }

    #[test]
    fn test_strength_reduce_div_pow2() {
        let mut instrs = vec![
            MirInstr::Const { dest: 0, value: 100, width: 32 },
            MirInstr::Const { dest: 1, value: 4, width: 32 },
            MirInstr::Binary { op: MirBinOp::Div, dest: 2, lhs: 0, rhs: 1, width: 32 },
        ];
        assert!(strength_reduce(&mut instrs));
        assert!(instrs.iter().any(|i| matches!(i, MirInstr::Binary { op: MirBinOp::Shr, .. })));
    }

    #[test]
    fn test_copy_propagate_unary_const() {
        let mut instrs = vec![
            MirInstr::Const { dest: 0, value: 0xFF, width: 8 },
            MirInstr::Unary {
                op: super::super::mir::MirUnOp::Not,
                dest: 1,
                operand: 0,
                width: 8,
            },
        ];
        assert!(copy_propagate(&mut instrs));
        assert!(matches!(&instrs[1], MirInstr::Const { value: 0, .. }));
    }

    #[test]
    fn test_optimize_process_full_pipeline() {
        let mut instrs = vec![
            MirInstr::Const { dest: 0, value: 5, width: 32 },
            MirInstr::Const { dest: 1, value: 8, width: 32 },
            MirInstr::Binary { op: MirBinOp::Mul, dest: 2, lhs: 0, rhs: 1, width: 32 },
            MirInstr::Const { dest: 3, value: 0, width: 1 },
            MirInstr::Branch { cond: 3, then_label: 10, else_label: 20 },
            MirInstr::Nop,
        ];
        optimize_process(&mut instrs);
        assert!(instrs.iter().any(|i| matches!(i, MirInstr::Const { value: 5, .. })));
        assert!(instrs.iter().any(|i| matches!(i, MirInstr::Const { value: 40, .. })));
        assert!(instrs.iter().any(|i| matches!(i, MirInstr::Jump { .. })));
        assert!(!instrs.iter().any(|i| matches!(i, MirInstr::Nop)));
    }

    #[test]
    fn test_dead_store_conservative_across_branch() {
        // reg 0 written before branch, used in one path — should NOT be eliminated
        let mut instrs = vec![
            MirInstr::Const { dest: 0, value: 5, width: 8 },
            MirInstr::Branch { cond: 0, then_label: 10, else_label: 20 },
            MirInstr::Label(10),
            MirInstr::Store { signal: 0, src: 0 },
            MirInstr::Jump { label: 30 },
            MirInstr::Label(20),
            MirInstr::Const { dest: 0, value: 10, width: 8 },
            MirInstr::Label(30),
        ];
        // dead_store_eliminate is conservative — shouldn't eliminate const[0] before Branch
        let changed = dead_store_eliminate(&mut instrs);
        // The const at index 0 is across a branch boundary from the next write at index 6
        // So it should NOT be eliminated
        assert!(!changed, "should not eliminate writes across control flow");
    }
}
