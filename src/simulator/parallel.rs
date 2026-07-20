use crate::error::SimError;
use crate::ir::{
    CaseType, IrExpr, IrLValue, IrStmt, LogicVal, LogicVec,
    SignalId,
};
use crate::simulator::value::*;

/// Configuration for parallel execution
#[derive(Debug, Clone, Copy)]
pub struct ParallelConfig {
    /// Number of worker threads (0 = auto-detect)
    pub num_threads: usize,
    /// Enable parallel process evaluation
    pub parallel_processes: bool,
    /// Enable parallel signal snapshot
    pub parallel_snapshot: bool,
    /// Minimum number of processes before parallelizing
    pub min_processes_parallel: usize,
    /// Minimum number of signals before parallelizing
    pub min_signals_parallel: usize,
}

impl Default for ParallelConfig {
    fn default() -> Self {
        let num_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        ParallelConfig {
            num_threads,
            parallel_processes: true,
            parallel_snapshot: true,
            min_processes_parallel: 4,
            min_signals_parallel: 64,
        }
    }
}

// ---------------------------------------------------------------------------
// Simplified expression evaluation for parallel context.
/// This version does NOT need &IrDesign, making it safe to use in rayon closures.
/// It handles the common expression types found in combinational processes.
pub fn evaluate_expr_simple(
    expr: &IrExpr,
    signals: &[LogicVec],
) -> Result<LogicVec, SimError> {
    match expr {
        IrExpr::Const(val) => Ok(val.clone()),
        IrExpr::FillLit(val) => Ok(LogicVec::fill(*val, 1)),
        IrExpr::Signal(id, _) => {
            Ok(signals.get(*id).cloned().unwrap_or_else(|| LogicVec::new(1)))
        }
        IrExpr::RangeSelect(sig_id, msb, lsb) => {
            if let Some(val) = signals.get(*sig_id) {
                let (start, end) = if *msb > *lsb { (*lsb, *msb) } else { (*msb, *lsb) };
                if end >= val.width {
                    return Ok(LogicVec::new(1));
                }
                let mut bits = val.bits[start..=end].to_vec();
                if *msb > *lsb { bits.reverse(); }
                Ok(LogicVec { width: bits.len(), bits })
            } else {
                Ok(LogicVec::new(1))
            }
        }
        IrExpr::BitSelect(sig_id, idx) => {
            let val = signals.get(*sig_id).cloned().unwrap_or_else(|| LogicVec::new(1));
            let bit = val.bits.get(*idx).copied().unwrap_or(LogicVal::X);
            Ok(LogicVec { bits: vec![bit], width: 1 })
        }
        IrExpr::ExprRangeSelect(inner, msb, lsb) => {
            let val = evaluate_expr_simple(inner, signals)?;
            let (start, end) = if *msb > *lsb { (*lsb, *msb) } else { (*msb, *lsb) };
            if end >= val.width {
                return Ok(LogicVec::new(1));
            }
            let mut bits = val.bits[start..=end].to_vec();
            if *msb > *lsb { bits.reverse(); }
            Ok(LogicVec { width: bits.len(), bits })
        }
        IrExpr::ExprBitSelect(inner, idx) => {
            let val = evaluate_expr_simple(inner, signals)?;
            let bit = val.bits.get(*idx).copied().unwrap_or(LogicVal::X);
            Ok(LogicVec { bits: vec![bit], width: 1 })
        }
        IrExpr::ExprPartSelect(inner, base_expr, width_expr) => {
            let val = evaluate_expr_simple(inner, signals)?;
            let base = evaluate_expr_simple(base_expr, signals)?.to_u64() as usize;
            let width = evaluate_expr_simple(width_expr, signals)?.to_u64() as usize;
            if width == 0 || base >= val.width {
                return Ok(LogicVec::new(1));
            }
            let end = (base + width - 1).min(val.width - 1);
            let mut bits = val.bits[base..=end].to_vec();
            bits.reverse();
            Ok(LogicVec { width: bits.len(), bits })
        }
        IrExpr::ArrayIndex { sig_id, index, elem_width } => {
            let key_val = evaluate_expr_simple(index, signals)?;
            let idx = key_val.to_u64() as usize;
            if let Some(array_val) = signals.get(*sig_id) {
                let start = idx * elem_width;
                let end = start + elem_width - 1;
                let mut bits = Vec::with_capacity(*elem_width);
                for i in start..=end.min(array_val.width.saturating_sub(1)) {
                    bits.push(array_val.bits.get(i).copied().unwrap_or(LogicVal::X));
                }
                Ok(LogicVec { width: *elem_width, bits })
            } else {
                Ok(LogicVec::new(*elem_width))
            }
        }
        IrExpr::Concat(exprs) => {
            let mut result = LogicVec::new(0);
            for e in exprs.iter().rev() {
                let part = evaluate_expr_simple(e, signals)?;
                result = result.extend(&part);
            }
            Ok(result)
        }
        IrExpr::Replicate(count, inner) => {
            let val = evaluate_expr_simple(inner, signals)?;
            let mut result = LogicVec::new(0);
            for _ in 0..*count {
                result = result.extend(&val);
            }
            Ok(result)
        }
        IrExpr::UnaryOp(op, inner) => {
            let val = evaluate_expr_simple(inner, signals)?;
            Ok(eval_unary(op.clone(), &val))
        }
        IrExpr::BinaryOp(op, lhs, rhs) => {
            let lhs_val = evaluate_expr_simple(lhs, signals)?;
            let rhs_val = evaluate_expr_simple(rhs, signals)?;
            Ok(eval_binary(op.clone(), &lhs_val, &rhs_val))
        }
        IrExpr::Cond(cond, true_val, false_val) => {
            let cond_val = evaluate_expr_simple(cond, signals)?;
            if cond_val.to_bool().unwrap_or(false) {
                evaluate_expr_simple(true_val, signals)
            } else {
                evaluate_expr_simple(false_val, signals)
            }
        }
        IrExpr::Signed(inner) => {
            evaluate_expr_simple(inner, signals)
        }
        IrExpr::String(s) => Ok(string_to_logicvec(s)),
        IrExpr::Cast { width, expr } => {
            let val = evaluate_expr_simple(expr, signals)?;
            Ok(val.resize(*width))
        }
        IrExpr::Inside { expr: inner, list } => {
            let val = evaluate_expr_simple(inner, signals)?;
            for item in list {
                let item_val = evaluate_expr_simple(item, signals)?;
                if val == item_val || val.casex_eq(&item_val) {
                    return Ok(LogicVec::from_u64(1, 1));
                }
            }
            Ok(LogicVec::from_u64(0, 1))
        }
        _ => {
            Ok(LogicVec::new(32))
        }
    }
}

/// Evaluate a block of IR statements against a mutable signal array,
/// collecting writes for later application.
/// This is the parallel-safe version that doesn't need SimulationEngine or IrDesign.
pub fn evaluate_stmt_block_parallel(
    stmts: &[IrStmt],
    signals: &mut Vec<LogicVec>,
    writes: &mut Vec<(SignalId, LogicVec)>,
) -> Result<(), SimError> {
    for stmt in stmts {
        match stmt {
            IrStmt::Block { stmts: inner } => {
                evaluate_stmt_block_parallel(inner, signals, writes)?;
            }
            IrStmt::BlockingAssign { lhs, rhs, delay: _ } => {
                let val = eval_assign_rhs_simple(rhs, lhs, signals)?;
                write_lvalue_simple(lhs, val, signals, writes)?;
            }
            IrStmt::NonBlockingAssign { lhs, rhs, delay: _ } => {
                let val = eval_assign_rhs_simple(rhs, lhs, signals)?;
                write_lvalue_simple(lhs, val, signals, writes)?;
            }
            IrStmt::If { cond, true_branch, false_branch } => {
                let cond_val = evaluate_expr_simple(cond, signals)?;
                if cond_val.to_bool().unwrap_or(false) {
                    evaluate_stmt_block_parallel(true_branch, signals, writes)?;
                } else if !false_branch.is_empty() {
                    evaluate_stmt_block_parallel(false_branch, signals, writes)?;
                }
            }
            IrStmt::Case { case_type, expr: case_expr, items, default } => {
                let case_val = evaluate_expr_simple(case_expr, signals)?;
                let mut matched = false;
                for case_item in items {
                    let mut item_matched = false;
                    for pat in &case_item.labels {
                        let pat_val = evaluate_expr_simple(pat, signals)?;
                        let eq = match case_type {
                            CaseType::CaseX => case_val.casex_eq(&pat_val),
                            CaseType::CaseZ => case_val.casez_eq(&pat_val),
                            CaseType::Normal => case_val.eq(&pat_val),
                        };
                        if eq {
                            evaluate_stmt_block_parallel(&case_item.body, signals, writes)?;
                            item_matched = true;
                            matched = true;
                            break;
                        }
                    }
                    if item_matched { break; }
                }
                if !matched && !default.is_empty() {
                    evaluate_stmt_block_parallel(default, signals, writes)?;
                }
            }
            IrStmt::LoopFor { init, cond, step, body } => {
                let mut iter_count = 0u64;
                if let Some(init_stmt) = init {
                    let cloned: IrStmt = init_stmt.as_ref().clone();
                    evaluate_stmt_block_parallel(&[cloned], signals, writes)?;
                }
                while iter_count < 1_000_000 {
                    let cond_val = evaluate_expr_simple(cond, signals)?;
                    if !cond_val.to_bool().unwrap_or(false) { break; }
                    evaluate_stmt_block_parallel(body, signals, writes)?;
                    if let Some(step_stmt) = step {
                        let cloned: IrStmt = step_stmt.as_ref().clone();
                        evaluate_stmt_block_parallel(&[cloned], signals, writes)?;
                    }
                    iter_count += 1;
                }
            }
            IrStmt::LoopWhile { cond, body } => {
                let mut iter_count = 0u64;
                while iter_count < 1_000_000 {
                    let cond_val = evaluate_expr_simple(cond, signals)?;
                    if !cond_val.to_bool().unwrap_or(false) { break; }
                    evaluate_stmt_block_parallel(body, signals, writes)?;
                    iter_count += 1;
                }
            }
            IrStmt::LoopDoWhile { cond, body } => {
                let mut iter_count = 0u64;
                loop {
                    evaluate_stmt_block_parallel(body, signals, writes)?;
                    iter_count += 1;
                    if iter_count >= 1_000_000 { break; }
                    let cond_val = evaluate_expr_simple(cond, signals)?;
                    if !cond_val.to_bool().unwrap_or(false) { break; }
                }
            }
            IrStmt::Repeat { count, body } => {
                let count_val = evaluate_expr_simple(count, signals)?;
                let n = count_val.to_u64().min(1_000_000);
                for _ in 0..n {
                    evaluate_stmt_block_parallel(body, signals, writes)?;
                }
            }
            IrStmt::Foreach { array_var, index_var: _, body } => {
                let arr_val = evaluate_expr_simple(array_var, signals)?;
                let elem_width = match array_var {
                    IrExpr::Signal(_, _) => {
                        // Try to estimate elem_width from signal array structure
                        // For simplicity, assume 1 bit per element if we can't determine
                        1
                    }
                    _ => 1,
                };
                let num_elems = if elem_width > 0 { arr_val.width / elem_width } else { 0 };
                let idx_sig = signals.len();
                signals.push(LogicVec::from_u64(0, 32));
                for i in 0..num_elems.min(10_000) {
                    signals[idx_sig] = LogicVec::from_u64(i as u64, 32);
                    evaluate_stmt_block_parallel(body, signals, writes)?;
                }
            }
            IrStmt::SysCall { .. } | IrStmt::SysFinish | IrStmt::Null => {}
            _ => {
                // Skip unsupported statement types in parallel eval.
                // These will be handled by the sequential fallback path.
                // Types skipped: Force, Release, Deassign, Wait, WaitOrder,
                // NamedBlock, Disable, EventControl, EventTrigger, Fork,
                // Assert, Assume, Cover, RandCase, RandSequence, Return.
            }
        }
    }
    Ok(())
}

/// Simplified assign RHS evaluation (no design reference needed)
fn eval_assign_rhs_simple(
    expr: &IrExpr,
    lhs: &IrLValue,
    signals: &[LogicVec],
) -> Result<LogicVec, SimError> {
    if let IrExpr::FillLit(v) = expr {
        let w = get_lvalue_width_simple(lhs, signals);
        Ok(LogicVec::fill(*v, w))
    } else if let IrExpr::Signed(inner) = expr {
        let mut val = evaluate_expr_simple(inner, signals)?;
        let target_w = get_lvalue_width_simple(lhs, signals);
        if val.width < target_w {
            let msb = val.bits.last().copied().unwrap_or(LogicVal::Zero);
            val.bits.resize(target_w, msb);
            val.width = target_w;
        }
        Ok(val)
    } else {
        evaluate_expr_simple(expr, signals)
    }
}

/// Get lvalue width (no design reference)
fn get_lvalue_width_simple(
    lvalue: &IrLValue,
    signals: &[LogicVec],
) -> usize {
    match lvalue {
        IrLValue::Signal(id, _) => {
            signals.get(*id).map(|s| s.width).unwrap_or(1)
        }
        IrLValue::RangeSelect(_, msb, lsb) => {
            let (lo, hi) = if *msb > *lsb { (*lsb, *msb) } else { (*msb, *lsb) };
            hi - lo + 1
        }
        IrLValue::BitSelect(_, _) => 1,
        IrLValue::ArrayIndex { elem_width, .. } => *elem_width,
        IrLValue::ArrayRangeSelect { elem_width, msb, lsb, .. } => {
            let (lo, hi) = if *msb > *lsb { (*lsb, *msb) } else { (*msb, *lsb) };
            (hi - lo + 1) * elem_width
        }
        IrLValue::ArrayBitSelect { elem_width, .. } => *elem_width,
        IrLValue::Concat(items) => {
            items.iter().map(|i| get_lvalue_width_simple(i, signals)).sum()
        }
    }
}

/// Simple write lvalue (no design reference)
fn write_lvalue_simple(
    lvalue: &IrLValue,
    val: LogicVec,
    signals: &mut Vec<LogicVec>,
    writes: &mut Vec<(SignalId, LogicVec)>,
) -> Result<(), SimError> {
    match lvalue {
        IrLValue::Signal(id, _) => {
            let target_width = signals.get(*id).map(|s| s.width).unwrap_or(1);
            let resized = if val.width != target_width { val.resize(target_width) } else { val };
            if *id < signals.len() {
                signals[*id] = resized.clone();
            }
            writes.push((*id, resized));
        }
        IrLValue::RangeSelect(sig_id, msb, lsb) => {
            let (start, end) = if *msb > *lsb { (*lsb, *msb) } else { (*msb, *lsb) };
            let mut existing = signals.get(*sig_id).cloned().unwrap_or_else(|| LogicVec::new(1));
            for i in start..=end.min(existing.width.saturating_sub(1)) {
                let src_idx = if *msb > *lsb { end - i } else { i - start };
                existing.bits[i] = val.bits.get(src_idx).copied().unwrap_or(LogicVal::X);
            }
            if *sig_id < signals.len() {
                signals[*sig_id] = existing.clone();
            }
            writes.push((*sig_id, existing));
        }
        IrLValue::BitSelect(sig_id, idx) => {
            let mut existing = signals.get(*sig_id).cloned().unwrap_or_else(|| LogicVec::new(1));
            if *idx < existing.width {
                existing.bits[*idx] = val.bits.first().copied().unwrap_or(LogicVal::X);
            }
            if *sig_id < signals.len() {
                signals[*sig_id] = existing.clone();
            }
            writes.push((*sig_id, existing));
        }
        IrLValue::ArrayIndex { sig_id, index, elem_width } => {
            let idx_val = evaluate_expr_simple(index, signals)?;
            let idx_u64 = idx_val.to_u64() as usize;
            let mut existing = signals.get(*sig_id).cloned().unwrap_or_else(|| LogicVec::new(1));
            let start = idx_u64 * elem_width;
            for i in 0..*elem_width {
                if start + i < existing.width {
                    existing.bits[start + i] = val.bits.get(i).copied().unwrap_or(LogicVal::X);
                }
            }
            if *sig_id < signals.len() {
                signals[*sig_id] = existing.clone();
            }
            writes.push((*sig_id, existing));
        }
        IrLValue::Concat(items) => {
            let mut offset = 0usize;
            for item in items {
                let item_w = get_lvalue_width_simple(item, signals);
                let slice_end = (offset + item_w).min(val.width);
                let slice = if offset < val.width {
                    let mut bits = Vec::with_capacity(item_w);
                    for i in offset..slice_end {
                        bits.push(val.bits.get(i).copied().unwrap_or(LogicVal::X));
                    }
                    LogicVec { width: item_w, bits }
                } else {
                    LogicVec::new(item_w)
                };
                write_lvalue_simple(item, slice, signals, writes)?;
                offset += item_w;
            }
        }
        _ => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Convert LogicVec to string (for string concat operations)
/// Convert string to LogicVec
fn string_to_logicvec(s: &str) -> LogicVec {
    let width = s.len() * 8;
    let mut bits = Vec::with_capacity(width);
    for byte in s.bytes() {
        for i in 0..8 {
            bits.push(if (byte >> i) & 1 == 1 { LogicVal::One } else { LogicVal::Zero });
        }
    }
    // Add null terminator
    for _ in 0..8 {
        bits.push(LogicVal::Zero);
    }
    LogicVec { bits, width: width + 8 }
}

/// Parallel signal snapshot: create a copy of all signal values using rayon
pub fn parallel_snapshot(signals: &[LogicVec]) -> Vec<LogicVec> {
    use rayon::prelude::*;
    signals.par_iter().cloned().collect()
}

