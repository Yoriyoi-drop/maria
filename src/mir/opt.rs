//! MIR Optimizations — peephole optimizations on MIR instructions.

use super::mir::{MirInstr, MirBinOp, MirModule};

/// Optimize a MIR module (peephole + constant folding).
pub fn optimize_module(module: &mut MirModule) {
    for process in &mut module.processes {
        optimize_process(&mut process.instrs);
    }
}

/// Optimize a list of MIR instructions.
pub fn optimize_process(instrs: &mut Vec<MirInstr>) {
    let mut changed = true;
    while changed {
        changed = false;
        changed |= constant_fold(instrs);
        changed |= dead_code_eliminate(instrs);
        changed |= remove_nops(instrs);
    }
}

/// Fold constant expressions.
fn constant_fold(instrs: &mut Vec<MirInstr>) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i < instrs.len() {
        if let MirInstr::Binary { op, dest, lhs, rhs, width } = &instrs[i] {
            // Check if lhs and rhs are constants
            let lhs_val = find_const_value(instrs, *lhs);
            let rhs_val = find_const_value(instrs, *rhs);

            if let (Some(lv), Some(rv)) = (lhs_val, rhs_val) {
                let result = match op {
                    MirBinOp::Add => lv.wrapping_add(rv),
                    MirBinOp::Sub => lv.wrapping_sub(rv),
                    MirBinOp::Mul => lv.wrapping_mul(rv),
                    MirBinOp::Div => if rv != 0 { lv / rv } else { 0 },
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
                    MirBinOp::Mul => lv.wrapping_mul(rv),
                };

                instrs[i] = MirInstr::Const { dest: *dest, value: result, width: *width };
                changed = true;
            }
        }
        i += 1;
    }
    changed
}

/// Try to find the constant value loaded into a register.
fn find_const_value(instrs: &[MirInstr], reg: usize) -> Option<u64> {
    for instr in instrs.iter().rev() {
        match instr {
            MirInstr::Const { dest, value, .. } if *dest == reg => return Some(*value),
            MirInstr::Load { .. } => return None,
            _ => {}
        }
    }
    None
}

/// Remove dead code (stores that are immediately overwritten).
fn dead_code_eliminate(instrs: &mut Vec<MirInstr>) -> bool {
    let len = instrs.len();
    if len < 2 {
        return false;
    }

    let mut changed = false;
    let mut i = 0;
    while i < instrs.len() - 1 {
        // Pattern: Store X → Store X (second one overwrites first)
        if let (MirInstr::Store { signal: s1, .. }, MirInstr::Store { signal: s2, .. }) =
            (&instrs[i], &instrs[i + 1])
        {
            if s1 == s2 {
                instrs.remove(i);
                changed = true;
                continue;
            }
        }
        i += 1;
    }
    changed
}

/// Remove consecutive Nop instructions.
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
    use crate::intern::Symbol;

    #[test]
    fn test_constant_fold() {
        let mut instrs = vec![
            MirInstr::Const { dest: 0, value: 3, width: 32 },
            MirInstr::Const { dest: 1, value: 4, width: 32 },
            MirInstr::Binary { op: MirBinOp::Add, dest: 2, lhs: 0, rhs: 1, width: 32 },
        ];

        let changed = constant_fold(&mut instrs);
        assert!(changed);

        // Result should be folded to const 7
        assert!(matches!(&instrs[2], MirInstr::Const { value: 7, .. }));
    }

    #[test]
    fn test_remove_nops() {
        let mut instrs = vec![
            MirInstr::Nop,
            MirInstr::Const { dest: 0, value: 1, width: 1 },
            MirInstr::Nop,
        ];

        let changed = remove_nops(&mut instrs);
        assert!(changed);
        assert_eq!(instrs.len(), 1);
    }
}
