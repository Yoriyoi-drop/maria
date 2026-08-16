use maria_core::error::SimError;
use maria_ir::{BinaryIrOp, CaseType, IrExpr, IrLValue, IrStmt, LogicVal, LogicVec, SignalId, SignalInfo};
use crate::simulator::util::{is_signed_expr, string_to_logicvec};
use crate::simulator::value::*;
use std::sync::Arc;

/// Pandangan sinyal untuk evaluasi paralel (SIM-28): base array + peta id
/// global→lokal + overlay tulis per-process. Per-process setup = O(0) (tanpa
/// clone seluruh sinyal). Baca: overlay dulu, lalu base via id_map. Tulis:
/// ke overlay (copy-on-write). Sinyal sintetis (Foreach) memakai id >=
/// id_map.len() agar tidak pernah bentrok dengan id sinyal global.
pub struct SignalView<'a> {
    base: &'a [Arc<LogicVec>],
    id_map: &'a [Option<usize>],
    overlay: &'a mut std::collections::HashMap<usize, Arc<LogicVec>>,
    next_synth: usize,
}

impl<'a> SignalView<'a> {
    pub fn new(
        base: &'a [Arc<LogicVec>],
        id_map: &'a [Option<usize>],
        overlay: &'a mut std::collections::HashMap<usize, Arc<LogicVec>>,
    ) -> Self {
        SignalView {
            base,
            id_map,
            overlay,
            next_synth: id_map.len(),
        }
    }

    #[inline]
    pub fn get(&self, id: usize) -> Option<&Arc<LogicVec>> {
        if let Some(v) = self.overlay.get(&id) {
            return Some(v);
        }
        if id < self.id_map.len() {
            match self.id_map[id] {
                Some(i) => self.base.get(i),
                None => None,
            }
        } else {
            None
        }
    }

    /// Tulis sinyal ke overlay local process (copy-on-write).
    #[inline]
    pub fn set(&mut self, id: usize, val: Arc<LogicVec>) {
        self.overlay.insert(id, val);
    }

    /// Id sintetis berikutnya untuk Foreach (>= id_map.len()).
    #[inline]
    pub fn len(&self) -> usize {
        self.next_synth
    }

    /// Push sinyal sintetis (Foreach index variable) ke overlay.
    #[inline]
    pub fn push(&mut self, val: Arc<LogicVec>) {
        self.overlay.insert(self.next_synth, val);
        self.next_synth += 1;
    }
}

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
/// `sig_info` (SignalInfo per signal) dipakai `is_signed_expr` agar
/// perbandingan/div/mod SIGNED konsisten dengan jalur serial (ROUND 36) —
/// parallel eval default-on untuk Process::Combinational.
pub fn evaluate_expr_simple(
    expr: &IrExpr,
    signals: &SignalView,
    sig_info: &[SignalInfo],
) -> Result<LogicVec, SimError> {
    match expr {
        IrExpr::Const(val) => Ok(val.clone()),
        IrExpr::FillLit(val) => Ok(LogicVec::fill(*val, 1)),
        IrExpr::Signal(id, _) => Ok(signals
            .get(*id)
            .map(|a| (**a).clone())
            .unwrap_or_else(|| LogicVec::new(1))),
        IrExpr::RangeSelect(sig_id, msb, lsb) => {
            if let Some(val) = signals.get(*sig_id) {
                let (start, end) = if *msb > *lsb {
                    (*lsb, *msb)
                } else {
                    (*msb, *lsb)
                };
                // Guard OOB lengkap (start & end & bits.len) — selain panic,
                // LogicVec dengan width>0 tapi bits kosong pernah muncul.
                let n = val.bits.len();
                if n == 0 || start >= n || end >= n || start > end {
                    return Ok(LogicVec::fill(LogicVal::X, (end - start + 1).max(1)));
                }
                let bits = val.bits[start..=end].to_vec();
                Ok(LogicVec {
                    width: bits.len(),
                    bits,
                })
            } else {
                Ok(LogicVec::new(1))
            }
        }
        IrExpr::BitSelect(sig_id, idx) => {
            let val = signals
                .get(*sig_id)
                .map(|a| (**a).clone())
                .unwrap_or_else(|| LogicVec::new(1));
            let bit = val.bits.get(*idx).copied().unwrap_or(LogicVal::X);
            Ok(LogicVec {
                bits: vec![bit],
                width: 1,
            })
        }
        IrExpr::ExprRangeSelect(inner, msb, lsb) => {
            let val = evaluate_expr_simple(inner, signals, sig_info)?;
            let (start, end) = if *msb > *lsb {
                (*lsb, *msb)
            } else {
                (*msb, *lsb)
            };
            let n = val.bits.len();
            if n == 0 || start >= n || end >= n || start > end {
                return Ok(LogicVec::fill(LogicVal::X, (end - start + 1).max(1)));
            }
            let bits = val.bits[start..=end].to_vec();
            Ok(LogicVec {
                width: bits.len(),
                bits,
            })
        }
        IrExpr::ExprBitSelect(inner, idx) => {
            let val = evaluate_expr_simple(inner, signals, sig_info)?;
            let bit = val.bits.get(*idx).copied().unwrap_or(LogicVal::X);
            Ok(LogicVec {
                bits: vec![bit],
                width: 1,
            })
        }
        IrExpr::ExprPartSelect(inner, base_expr, width_expr) => {
            let val = evaluate_expr_simple(inner, signals, sig_info)?;
            let base = evaluate_expr_simple(base_expr, signals, sig_info)?.to_u64() as usize;
            let width = evaluate_expr_simple(width_expr, signals, sig_info)?.to_u64() as usize;
            let n = val.bits.len();
            if n == 0 || width == 0 || base >= n {
                return Ok(LogicVec::new(1));
            }
            let end = (base + width - 1).min(n - 1);
            let bits = val.bits[base..=end].to_vec();
            Ok(LogicVec {
                width: bits.len(),
                bits,
            })
        }
        IrExpr::ArrayIndex {
            sig_id,
            index,
            elem_width,
        } => {
            let key_val = evaluate_expr_simple(index, signals, sig_info)?;
            let idx = key_val.to_u64() as usize;
            if let Some(array_val) = signals.get(*sig_id) {
                let start = idx * elem_width;
                let end = start + elem_width - 1;
                // SELALU hasilkan elem_width bit — index OOB di-pad X (sama
                // dengan jalur serial eval/expr.rs). Versi lama membatasi loop
                // ke array_val.width sehingga index OOB menghasilkan `bits`
                // KOSONG tapi width=elem_width>0 → panic di value.rs:28
                // (to_bitmasks) saat hasilnya dipakai op berikutnya.
                let mut bits = Vec::with_capacity(*elem_width);
                for i in start..=end {
                    bits.push(array_val.bits.get(i).copied().unwrap_or(LogicVal::X));
                }
                Ok(LogicVec {
                    width: *elem_width,
                    bits,
                })
            } else {
                Ok(LogicVec::new(*elem_width))
            }
        }
        IrExpr::Concat(exprs) => {
            let mut result = LogicVec::new(0);
            for e in exprs.iter().rev() {
                let part = evaluate_expr_simple(e, signals, sig_info)?;
                result = result.extend(&part);
            }
            Ok(result)
        }
        IrExpr::Replicate(count, inner) => {
            let val = evaluate_expr_simple(inner, signals, sig_info)?;
            let mut result = LogicVec::new(0);
            for _ in 0..*count {
                result = result.extend(&val);
            }
            Ok(result)
        }
        IrExpr::UnaryOp(op, inner) => {
            let val = evaluate_expr_simple(inner, signals, sig_info)?;
            Ok(eval_unary(op.clone(), &val))
        }
        IrExpr::BinaryOp(op, lhs, rhs) => {
            let lhs_val = evaluate_expr_simple(lhs, signals, sig_info)?;
            let rhs_val = evaluate_expr_simple(rhs, signals, sig_info)?;
            if matches!(
                op,
                BinaryIrOp::Lt
                    | BinaryIrOp::Le
                    | BinaryIrOp::Gt
                    | BinaryIrOp::Ge
                    | BinaryIrOp::Div
                    | BinaryIrOp::Mod
            ) && (is_signed_expr(lhs.as_ref(), sig_info)
                && is_signed_expr(rhs.as_ref(), sig_info))
            {
                Ok(eval_binary_signed(op.clone(), &lhs_val, &rhs_val))
            } else if matches!(op, BinaryIrOp::Sshr) {
                // `>>>`: arithmetic bila lhs signed, logical bila unsigned
                // (konsisten dengan jalur serial — ROUND 36).
                if is_signed_expr(lhs.as_ref(), sig_info) {
                    Ok(eval_sshr_signed(&lhs_val, &rhs_val))
                } else {
                    Ok(eval_binary(BinaryIrOp::Shr, &lhs_val, &rhs_val))
                }
            } else {
                Ok(eval_binary(op.clone(), &lhs_val, &rhs_val))
            }
        }
        IrExpr::Cond(cond, true_val, false_val) => {
            let cond_val = evaluate_expr_simple(cond, signals, sig_info)?;
            if cond_val.to_bool().unwrap_or(false) {
                evaluate_expr_simple(true_val, signals, sig_info)
            } else {
                evaluate_expr_simple(false_val, signals, sig_info)
            }
        }
        IrExpr::Signed(inner) => evaluate_expr_simple(inner, signals, sig_info),
        IrExpr::String(s) => Ok(string_to_logicvec(s)),
        IrExpr::Cast { width, expr } => {
            let val = evaluate_expr_simple(expr, signals, sig_info)?;
            Ok(val.resize(*width))
        }
        IrExpr::Inside { expr: inner, list } => {
            let val = evaluate_expr_simple(inner, signals, sig_info)?;
            for item in list {
                let item_val = evaluate_expr_simple(item, signals, sig_info)?;
                if val == item_val || val.casex_eq(&item_val) {
                    return Ok(LogicVec::from_u64(1, 1));
                }
            }
            Ok(LogicVec::from_u64(0, 1))
        }
        _ => Ok(LogicVec::new(32)),
    }
}

/// Evaluate a block of IR statements against a mutable signal array,
/// collecting writes for later application.
/// This is the parallel-safe version that doesn't need SimulationEngine or IrDesign.
pub fn evaluate_stmt_block_parallel(
    stmts: &[IrStmt],
    signals: &mut SignalView,
    writes: &mut Vec<(SignalId, LogicVec)>,
    sig_info: &[SignalInfo],
) -> Result<(), SimError> {
    for stmt in stmts {
        match stmt {
            IrStmt::Block { stmts: inner } => {
                evaluate_stmt_block_parallel(inner, signals, writes, sig_info)?;
            }
            IrStmt::BlockingAssign { lhs, rhs, delay: _ } => {
                let val = eval_assign_rhs_simple(rhs, lhs, signals, sig_info)?;
                write_lvalue_simple(lhs, val, signals, writes, sig_info)?;
            }
            IrStmt::NonBlockingAssign { lhs, rhs, delay: _ } => {
                let val = eval_assign_rhs_simple(rhs, lhs, signals, sig_info)?;
                write_lvalue_simple(lhs, val, signals, writes, sig_info)?;
            }
            IrStmt::If {
                cond,
                true_branch,
                false_branch,
            } => {
                let cond_val = evaluate_expr_simple(cond, signals, sig_info)?;
                if cond_val.to_bool().unwrap_or(false) {
                    evaluate_stmt_block_parallel(true_branch, signals, writes, sig_info)?;
                } else if !false_branch.is_empty() {
                    evaluate_stmt_block_parallel(false_branch, signals, writes, sig_info)?;
                }
            }
            IrStmt::Case {
                case_type,
                expr: case_expr,
                items,
                default,
            } => {
                let case_val = evaluate_expr_simple(case_expr, signals, sig_info)?;
                let mut matched = false;
                for case_item in items {
                    let mut item_matched = false;
                    for pat in &case_item.labels {
                        let eq = match (case_type, pat) {
                            (CaseType::Inside, IrExpr::InsideRange { lo, hi, .. }) => {
                                let lo_v = evaluate_expr_simple(lo, signals, sig_info)?.to_u64();
                                let hi_v = evaluate_expr_simple(hi, signals, sig_info)?.to_u64();
                                let v = case_val.to_u64();
                                v >= lo_v.min(hi_v) && v <= lo_v.max(hi_v)
                            }
                            _ => {
                                let pat_val = evaluate_expr_simple(pat, signals, sig_info)?;
                                match case_type {
                                    CaseType::CaseX => case_val.casex_eq(&pat_val),
                                    CaseType::CaseZ => case_val.casez_eq(&pat_val),
                                    CaseType::Normal | CaseType::Inside => {
                                        case_val.case_val_eq(&pat_val)
                                    }
                                    CaseType::Unique | CaseType::Unique0 | CaseType::Priority => {
                                        case_val.case_val_eq(&pat_val)
                                    }
                                }
                            }
                        };
                        if eq {
                            evaluate_stmt_block_parallel(&case_item.body, signals, writes, sig_info)?;
                            item_matched = true;
                            matched = true;
                            break;
                        }
                    }
                    if item_matched {
                        break;
                    }
                }
                if !matched && !default.is_empty() {
                    evaluate_stmt_block_parallel(default, signals, writes, sig_info)?;
                }
            }
            IrStmt::LoopFor {
                init,
                cond,
                step,
                body,
            } => {
                let mut iter_count = 0u64;
                if let Some(init_stmt) = init {
                    let cloned: IrStmt = init_stmt.as_ref().clone();
                    evaluate_stmt_block_parallel(&[cloned], signals, writes, sig_info)?;
                }
                while iter_count < 1_000_000 {
                    let cond_val = evaluate_expr_simple(cond, signals, sig_info)?;
                    if !cond_val.to_bool().unwrap_or(false) {
                        break;
                    }
                    evaluate_stmt_block_parallel(body, signals, writes, sig_info)?;
                    if let Some(step_stmt) = step {
                        let cloned: IrStmt = step_stmt.as_ref().clone();
                        evaluate_stmt_block_parallel(&[cloned], signals, writes, sig_info)?;
                    }
                    iter_count += 1;
                }
            }
            IrStmt::LoopWhile { cond, body } => {
                let mut iter_count = 0u64;
                while iter_count < 1_000_000 {
                    let cond_val = evaluate_expr_simple(cond, signals, sig_info)?;
                    if !cond_val.to_bool().unwrap_or(false) {
                        break;
                    }
                    evaluate_stmt_block_parallel(body, signals, writes, sig_info)?;
                    iter_count += 1;
                }
            }
            IrStmt::LoopDoWhile { cond, body } => {
                let mut iter_count = 0u64;
                loop {
                    evaluate_stmt_block_parallel(body, signals, writes, sig_info)?;
                    iter_count += 1;
                    if iter_count >= 1_000_000 {
                        break;
                    }
                    let cond_val = evaluate_expr_simple(cond, signals, sig_info)?;
                    if !cond_val.to_bool().unwrap_or(false) {
                        break;
                    }
                }
            }
            IrStmt::Repeat { count, body } => {
                let count_val = evaluate_expr_simple(count, signals, sig_info)?;
                let n = count_val.to_u64().min(1_000_000);
                for _ in 0..n {
                    evaluate_stmt_block_parallel(body, signals, writes, sig_info)?;
                }
            }
            IrStmt::Foreach {
                array_var,
                index_var: _,
                body,
            } => {
                let arr_val = evaluate_expr_simple(array_var, signals, sig_info)?;
                let elem_width = match array_var {
                    IrExpr::Signal(_, _) => {
                        // Try to estimate elem_width from signal array structure
                        // For simplicity, assume 1 bit per element if we can't determine
                        1
                    }
                    _ => 1,
                };
                let num_elems = arr_val.width.checked_div(elem_width).unwrap_or(0);
                let idx_sig = signals.len();
                signals.push(Arc::new(LogicVec::from_u64(0, 32)));
                for i in 0..num_elems.min(10_000) {
                    signals.set(idx_sig, Arc::new(LogicVec::from_u64(i as u64, 32)));
                    evaluate_stmt_block_parallel(body, signals, writes, sig_info)?;
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
    signals: &SignalView,
    sig_info: &[SignalInfo],
) -> Result<LogicVec, SimError> {
    if let IrExpr::FillLit(v) = expr {
        let w = get_lvalue_width_simple(lhs, signals, sig_info);
        Ok(LogicVec::fill(*v, w))
    } else if let IrExpr::Signed(inner) = expr {
        let mut val = evaluate_expr_simple(inner, signals, sig_info)?;
        let target_w = get_lvalue_width_simple(lhs, signals, sig_info);
        if val.width < target_w {
            let msb = val.bits.last().copied().unwrap_or(LogicVal::Zero);
            val.bits.resize(target_w, msb);
            val.width = target_w;
        }
        Ok(val)
    } else {
        evaluate_expr_simple(expr, signals, sig_info)
    }
}

/// Get lvalue width (no design reference)
fn get_lvalue_width_simple(
    lvalue: &IrLValue,
    signals: &SignalView,
    sig_info: &[SignalInfo],
) -> usize {
    match lvalue {
        IrLValue::Signal(id, _) => signals.get(*id).map(|s| s.width).unwrap_or(1),
        IrLValue::RangeSelect(_, msb, lsb) => {
            let (lo, hi) = if *msb > *lsb {
                (*lsb, *msb)
            } else {
                (*msb, *lsb)
            };
            hi - lo + 1
        }
        IrLValue::BitSelect(_, _) => 1,
        IrLValue::ArrayIndex { elem_width, .. } => *elem_width,
        IrLValue::ArrayRangeSelect {
            elem_width,
            msb,
            lsb,
            ..
        } => {
            let (lo, hi) = if *msb > *lsb {
                (*lsb, *msb)
            } else {
                (*msb, *lsb)
            };
            (hi - lo + 1) * elem_width
        }
        IrLValue::ArrayBitSelect { elem_width, .. } => *elem_width,
        IrLValue::ExprPartSelect { width, .. } => *width,
        IrLValue::HierRef(_) | IrLValue::HierRefIndex { .. } => 1,
        IrLValue::ObjectField { .. } => 64,
        IrLValue::Concat(items) => items
            .iter()
            .map(|i| get_lvalue_width_simple(i, signals, sig_info))
            .sum(),
    }
}

/// Simple write lvalue (no design reference)
fn write_lvalue_simple(
    lvalue: &IrLValue,
    val: LogicVec,
    signals: &mut SignalView,
    writes: &mut Vec<(SignalId, LogicVec)>,
    sig_info: &[SignalInfo],
) -> Result<(), SimError> {
    match lvalue {
        IrLValue::Signal(id, _) => {
            let target_width = signals.get(*id).map(|s| s.width).unwrap_or(1);
            let resized = if val.width != target_width {
                val.resize(target_width)
            } else {
                val
            };
            // Defensif: normalisasi nilai korup (width>0 tapi bits kosong)
            // agar tidak mencemari state dan memicu panic di jalur eval lain.
            let resized = if resized.width > 0 && resized.bits.is_empty() {
                LogicVec::fill(LogicVal::X, resized.width)
            } else {
                resized
            };
            signals.set(*id, Arc::new(resized.clone()));
            writes.push((*id, resized));
        }
        IrLValue::RangeSelect(sig_id, msb, lsb) => {
            let (start, end) = if *msb > *lsb {
                (*lsb, *msb)
            } else {
                (*msb, *lsb)
            };
            let mut existing = signals
                .get(*sig_id)
                .map(|a| (**a).clone())
                .unwrap_or_else(|| LogicVec::new(1));
            for i in start..=end.min(existing.width.saturating_sub(1)) {
                let src_idx = if *msb > *lsb { end - i } else { i - start };
                existing.bits[i] = val.bits.get(src_idx).copied().unwrap_or(LogicVal::X);
            }
            signals.set(*sig_id, Arc::new(existing.clone()));
            writes.push((*sig_id, existing));
        }
        IrLValue::BitSelect(sig_id, idx) => {
            let mut existing = signals
                .get(*sig_id)
                .map(|a| (**a).clone())
                .unwrap_or_else(|| LogicVec::new(1));
            if *idx < existing.width {
                existing.bits[*idx] = val.bits.first().copied().unwrap_or(LogicVal::X);
            }
            signals.set(*sig_id, Arc::new(existing.clone()));
            writes.push((*sig_id, existing));
        }
        IrLValue::ArrayIndex {
            sig_id,
            index,
            elem_width,
        } => {
            let idx_val = evaluate_expr_simple(index, signals, sig_info)?;
            let idx_u64 = idx_val.to_u64() as usize;
            let mut existing = signals
                .get(*sig_id)
                .map(|a| (**a).clone())
                .unwrap_or_else(|| LogicVec::new(1));
            let start = idx_u64 * elem_width;
            for i in 0..*elem_width {
                if start + i < existing.width {
                    existing.bits[start + i] = val.bits.get(i).copied().unwrap_or(LogicVal::X);
                }
            }
            signals.set(*sig_id, Arc::new(existing.clone()));
            writes.push((*sig_id, existing));
        }
        IrLValue::ExprPartSelect {
            sig_id,
            base,
            width,
        } => {
            let idx_val = evaluate_expr_simple(base, signals, sig_info)?;
            let start = idx_val.to_u64() as usize;
            let mut existing = signals
                .get(*sig_id)
                .map(|a| (**a).clone())
                .unwrap_or_else(|| LogicVec::new(1));
            for i in 0..*width {
                if start + i < existing.width {
                    existing.bits[start + i] = val.bits.get(i).copied().unwrap_or(LogicVal::X);
                }
            }
            signals.set(*sig_id, Arc::new(existing.clone()));
            writes.push((*sig_id, existing));
        }
        IrLValue::Concat(items) => {
            // LRM 1800-2017 §10.7: assignment ke concat lvalue — RHS
            // di-zero-extend ke lebar total concat, lalu dibagikan MSB-first
            // (part PERTAMA = bit paling tinggi). Handler lama mengiris
            // LSB-first (offset naik dari 0) → bit terbalik (`{co, s} = a+b`
            // menaruh LSB ke co) DAN mengisi X saat RHS lebih sempit dari
            // total concat (panic logic.rs:97 width=8 bits.len=7).
            let total: usize = items
                .iter()
                .map(|it| get_lvalue_width_simple(it, signals, sig_info))
                .sum();
            let mut bits = val.bits.clone();
            if bits.len() < total {
                bits.resize(total, LogicVal::Zero);
            } else if bits.len() > total {
                bits.truncate(total);
            }
            let mut offset = total;
            for item in items {
                let item_w = get_lvalue_width_simple(item, signals, sig_info);
                offset -= item_w;
                let sub_val = LogicVec {
                    width: item_w,
                    bits: bits[offset..offset + item_w].to_vec(),
                };
                write_lvalue_simple(item, sub_val, signals, writes, sig_info)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Parallel signal snapshot: create a copy of all signal values using rayon
pub fn parallel_snapshot(signals: &[LogicVec]) -> Vec<LogicVec> {
    use rayon::prelude::*;
    signals.par_iter().cloned().collect()
}
