//! Control flow evaluation methods untuk IR statement blocks.
//! Diekstrak dari block.rs — 1 file = 1 tanggung jawab.
//!
//! Menangani: Block, NamedBlock, If, Case, LoopFor, LoopWhile,
//! LoopDoWhile, Repeat, Foreach, Break, Continue.

use super::super::SimulationEngine;
use super::super::MAX_LOOP_ITER;
use crate::error::SimError;
use crate::ir::*;
use crate::Symbol;
use crate::simulator::types::*;
use std::collections::HashMap;

impl SimulationEngine {
    // ─── Block / NamedBlock ─────────────────────────────────────────

    /// Evaluate a Block statement with delay/fork support.
    pub(crate) fn evaluate_block_fork(
        &mut self,
        inner: &[IrStmt],
        fork_id: Option<usize>,
    ) -> Result<bool, SimError> {
        self.evaluate_block_with_delay_fork(inner, fork_id)
    }

    /// Evaluate a Block statement without delay/fork (stmt context).
    pub(crate) fn evaluate_block_stmt(&mut self, inner: &[IrStmt]) -> Result<(), SimError> {
        self.evaluate_stmt_block(inner)
    }

    /// Evaluate a NamedBlock with delay/fork support.
    pub(crate) fn evaluate_named_block_fork(
        &mut self,
        name: Symbol,
        inner: &[IrStmt],
        fork_id: Option<usize>,
    ) -> Result<bool, SimError> {
        if self.disable_pending == Some(name) {
            self.disable_pending = None;
            return Ok(true);
        }
        let old = self.disable_pending.take();
        let completed = self.evaluate_block_with_delay_fork(inner, fork_id)?;
        if let Some(ref n) = self.disable_pending {
            if *n == name {
                self.disable_pending = None;
            }
        }
        self.disable_pending = self.disable_pending.take().or(old);
        if !completed {
            return Ok(false);
        }
        Ok(true)
    }

    /// Evaluate a NamedBlock without delay/fork (stmt context).
    pub(crate) fn evaluate_named_block_stmt(
        &mut self,
        name: Symbol,
        inner: &[IrStmt],
    ) -> Result<(), SimError> {
        if self.disable_pending == Some(name) {
            self.disable_pending = None;
            return Ok(());
        }
        let old = self.disable_pending.take();
        self.evaluate_stmt_block(inner)?;
        if let Some(ref n) = self.disable_pending {
            if *n == name {
                self.disable_pending = None;
            }
        }
        self.disable_pending = self.disable_pending.take().or(old);
        Ok(())
    }

    // ─── If ─────────────────────────────────────────────────────────

    /// Evaluate an If statement with delay/fork support.
    pub(crate) fn evaluate_if_fork(
        &mut self,
        cond: &IrExpr,
        then_stmts: &[IrStmt],
        else_stmts: &[IrStmt],
        fork_id: Option<usize>,
    ) -> Result<bool, SimError> {
        let cond_val = self.evaluate_expr(cond)?;
        self.cover_branch_counter += 1;
        let branch_key = Symbol::intern(&format!(
            "{}.if_fork#{}",
            self.current_process_name.as_deref().unwrap_or("?"),
            self.cover_branch_counter
        ));
        if cond_val.to_bool().unwrap_or(false) {
            self.record_branch_hit(branch_key, "true");
            self.evaluate_block_with_delay_fork(then_stmts, fork_id)
        } else if !else_stmts.is_empty() {
            self.record_branch_hit(branch_key, "false");
            self.evaluate_block_with_delay_fork(else_stmts, fork_id)
        } else {
            self.record_branch_hit(branch_key, "false_no_else");
            Ok(true)
        }
    }

    /// Evaluate an If statement without delay/fork (stmt context).
    pub(crate) fn evaluate_if_stmt(
        &mut self,
        cond: &IrExpr,
        then_stmts: &[IrStmt],
        else_stmts: &[IrStmt],
    ) -> Result<(), SimError> {
        let cond_val = self.evaluate_expr(cond)?;
        self.cover_branch_counter += 1;
        let branch_key = Symbol::intern(&format!(
            "{}.if_stmt#{}",
            self.current_process_name.as_deref().unwrap_or("?"),
            self.cover_branch_counter
        ));
        if cond_val.to_bool().unwrap_or(false) {
            self.record_branch_hit(branch_key, "true");
            self.evaluate_stmt_block(then_stmts)
        } else if !else_stmts.is_empty() {
            self.record_branch_hit(branch_key, "false");
            self.evaluate_stmt_block(else_stmts)
        } else {
            self.record_branch_hit(branch_key, "false_no_else");
            Ok(())
        }
    }

    // ─── Case ────────────────────────────────────────────────────────

    /// Evaluate a Case statement with delay/fork support.
    #[allow(clippy::needless_return)]
    pub(crate) fn evaluate_case_fork(
        &mut self,
        case_type: &CaseType,
        case_expr: &IrExpr,
        items: &[IrCaseItem],
        default: &[IrStmt],
        fork_id: Option<usize>,
    ) -> Result<bool, SimError> {
        let case_val = self.evaluate_expr(case_expr)?;
        self.cover_branch_counter += 1;
        let case_key = Symbol::intern(&format!(
            "{}.case_fork#{}",
            self.current_process_name.as_deref().unwrap_or("?"),
            self.cover_branch_counter
        ));
        let mut matched = false;
        for (item_idx, case_item) in items.iter().enumerate() {
            let mut item_matched = false;
            for pat in &case_item.labels {
                let pat_val = self.evaluate_expr(pat)?;
                let eq = match case_type {
                    CaseType::CaseX => case_val.casex_eq(&pat_val),
                    CaseType::CaseZ => case_val.casez_eq(&pat_val),
                    CaseType::Normal => case_val.eq(&pat_val),
                };
                if eq {
                    self.record_branch_hit(case_key, &format!("item{}_matched", item_idx));
                    if !self.evaluate_block_with_delay_fork(&case_item.body, fork_id)? {
                        return Ok(false);
                    }
                    if self.disable_pending.is_some() {
                        return Ok(true);
                    }
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
            self.record_branch_hit(case_key, "default");
            if !self.evaluate_block_with_delay_fork(default, fork_id)? {
                return Ok(false);
            }
        }
        if !matched && default.is_empty() {
            self.record_branch_hit(case_key, "nomatch_nodefault");
        }
        Ok(true)
    }

    /// Evaluate a Case statement without delay/fork (stmt context).
    pub(crate) fn evaluate_case_stmt(
        &mut self,
        case_type: &CaseType,
        case_expr: &IrExpr,
        items: &[IrCaseItem],
        default: &[IrStmt],
    ) -> Result<(), SimError> {
        let case_val = self.evaluate_expr(case_expr)?;
        self.cover_branch_counter += 1;
        let case_key = Symbol::intern(&format!(
            "{}.case_stmt#{}",
            self.current_process_name.as_deref().unwrap_or("?"),
            self.cover_branch_counter
        ));
        let mut matched = false;
        for (item_idx, case_item) in items.iter().enumerate() {
            let mut item_matched = false;
            for pat in &case_item.labels {
                let pat_val = self.evaluate_expr(pat)?;
                let eq = match case_type {
                    CaseType::CaseX => case_val.casex_eq(&pat_val),
                    CaseType::CaseZ => case_val.casez_eq(&pat_val),
                    CaseType::Normal => case_val.eq(&pat_val),
                };
                if eq {
                    self.record_branch_hit(case_key, &format!("item{}_matched", item_idx));
                    self.evaluate_stmt_block(&case_item.body)?;
                    if self.disable_pending.is_some() {
                        return Ok(());
                    }
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
            self.record_branch_hit(case_key, "default");
            self.evaluate_stmt_block(default)?;
        }
        if !matched && default.is_empty() {
            self.record_branch_hit(case_key, "nomatch_nodefault");
        }
        Ok(())
    }

    // ─── Break / Continue ────────────────────────────────────────────

    /// Evaluate a Break statement with delay/fork support.
    pub(crate) fn evaluate_break_fork(&mut self) -> Result<bool, SimError> {
        self.control_flow = Some(FlowControl::Break);
        Ok(true)
    }

    /// Evaluate a Break statement without delay/fork (stmt context).
    pub(crate) fn evaluate_break_stmt(&mut self) -> Result<(), SimError> {
        self.control_flow = Some(FlowControl::Break);
        Ok(())
    }

    /// Evaluate a Continue statement with delay/fork support.
    pub(crate) fn evaluate_continue_fork(&mut self) -> Result<bool, SimError> {
        self.control_flow = Some(FlowControl::Continue);
        Ok(true)
    }

    /// Evaluate a Continue statement without delay/fork (stmt context).
    pub(crate) fn evaluate_continue_stmt(&mut self) -> Result<(), SimError> {
        self.control_flow = Some(FlowControl::Continue);
        Ok(())
    }

    // ─── LoopFor ─────────────────────────────────────────────────────

    /// Evaluate a for loop with delay/fork support.
    pub(crate) fn evaluate_loop_for_fork(
        &mut self,
        init: &Option<Box<IrStmt>>,
        cond: &IrExpr,
        step: &Option<Box<IrStmt>>,
        body: &[IrStmt],
        fork_id: Option<usize>,
    ) -> Result<bool, SimError> {
        if let Some(init_stmt) = init {
            if !self.evaluate_block_with_delay_fork(&[*init_stmt.clone()], fork_id)? {
                return Ok(false);
            }
        }
        let mut iter_count = 0usize;
        loop {
            if iter_count >= MAX_LOOP_ITER {
                eprintln!(
                    "warning: for loop exceeded {} iterations, breaking",
                    MAX_LOOP_ITER
                );
                break;
            }
            iter_count += 1;
            if self.disable_pending.is_some() {
                break;
            }
            if self.control_flow.is_some() {
                self.control_flow = None;
                break;
            }
            let cond_val = self.evaluate_expr(cond)?;
            if !cond_val.to_bool().unwrap_or(false) {
                break;
            }
            let body_completed = self.evaluate_block_with_delay_fork(body, fork_id)?;
            if !body_completed {
                return Ok(false);
            }
            let cf = self.control_flow.take();
            if cf == Some(FlowControl::Continue) {
                if let Some(step_stmt) = step {
                    if !self.evaluate_block_with_delay_fork(&[*step_stmt.clone()], fork_id)? {
                        return Ok(false);
                    }
                }
                continue;
            }
            if cf == Some(FlowControl::Break) {
                break;
            }
            if self.disable_pending.is_some() {
                break;
            }
            if let Some(step_stmt) = step {
                if !self.evaluate_block_with_delay_fork(&[*step_stmt.clone()], fork_id)? {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// Evaluate a for loop without delay/fork (stmt context).
    pub(crate) fn evaluate_loop_for_stmt(
        &mut self,
        init: &Option<Box<IrStmt>>,
        cond: &IrExpr,
        step: &Option<Box<IrStmt>>,
        body: &[IrStmt],
    ) -> Result<(), SimError> {
        if let Some(init_stmt) = init {
            self.evaluate_stmt_block(&[*init_stmt.clone()])?;
        }
        let mut iter_count = 0usize;
        loop {
            if iter_count >= MAX_LOOP_ITER {
                eprintln!(
                    "warning: for loop exceeded {} iterations, breaking",
                    MAX_LOOP_ITER
                );
                break;
            }
            iter_count += 1;
            if self.disable_pending.is_some() {
                break;
            }
            if self.control_flow.is_some() {
                self.control_flow = None;
                break;
            }
            let cond_val = self.evaluate_expr(cond)?;
            if !cond_val.to_bool().unwrap_or(false) {
                break;
            }
            self.evaluate_stmt_block(body)?;
            let cf = self.control_flow.take();
            if cf == Some(FlowControl::Continue) {
                if let Some(step_stmt) = step {
                    self.evaluate_stmt_block(&[*step_stmt.clone()])?;
                }
                continue;
            }
            if cf == Some(FlowControl::Break) {
                break;
            }
            if self.disable_pending.is_some() {
                break;
            }
            if let Some(step_stmt) = step {
                self.evaluate_stmt_block(&[*step_stmt.clone()])?;
            }
        }
        Ok(())
    }

    // ─── LoopWhile ───────────────────────────────────────────────────

    /// Evaluate a while loop with delay/fork support (handles loop_continuation).
    pub(crate) fn evaluate_loop_while_fork(
        &mut self,
        cond: &IrExpr,
        body: &[IrStmt],
        fork_id: Option<usize>,
    ) -> Result<bool, SimError> {
        let mut iter_count = 0usize;
        loop {
            if iter_count >= MAX_LOOP_ITER {
                eprintln!(
                    "warning: while loop exceeded {} iterations, breaking",
                    MAX_LOOP_ITER
                );
                break;
            }
            iter_count += 1;
            if self.disable_pending.is_some() {
                break;
            }
            if self.control_flow.is_some() {
                self.control_flow = None;
                break;
            }
            let cond_val = self.evaluate_expr(cond)?;
            if !cond_val.to_bool().unwrap_or(false) {
                break;
            }
            let old_loop_cont = self.loop_continuation.take();
            let mut lc = vec![IrStmt::LoopWhile {
                cond: cond.clone(),
                body: body.to_vec(),
            }];
            if !self.post_loop_tail.is_empty() {
                lc.extend(self.post_loop_tail.clone());
            }
            self.loop_continuation = Some(lc);
            let completed = self.evaluate_block_with_delay_fork(body, fork_id)?;
            self.loop_continuation = old_loop_cont;
            if !completed {
                return Ok(false);
            }
            let cf = self.control_flow.take();
            if cf == Some(FlowControl::Continue) {
                continue;
            }
            if cf == Some(FlowControl::Break) {
                break;
            }
        }
        Ok(true)
    }

    /// Evaluate a while loop without delay/fork (no loop_continuation needed).
    pub(crate) fn evaluate_loop_while_stmt(
        &mut self,
        cond: &IrExpr,
        body: &[IrStmt],
    ) -> Result<(), SimError> {
        let mut iter_count = 0usize;
        loop {
            if iter_count >= MAX_LOOP_ITER {
                eprintln!(
                    "warning: while loop exceeded {} iterations, breaking",
                    MAX_LOOP_ITER
                );
                break;
            }
            iter_count += 1;
            if self.disable_pending.is_some() {
                break;
            }
            if self.control_flow.is_some() {
                self.control_flow = None;
                break;
            }
            let cond_val = self.evaluate_expr(cond)?;
            if !cond_val.to_bool().unwrap_or(false) {
                break;
            }
            self.evaluate_stmt_block(body)?;
            let cf = self.control_flow.take();
            if cf == Some(FlowControl::Continue) {
                continue;
            }
            if cf == Some(FlowControl::Break) {
                break;
            }
        }
        Ok(())
    }

    // ─── LoopDoWhile ─────────────────────────────────────────────────

    /// Evaluate a do-while loop with delay/fork support.
    pub(crate) fn evaluate_loop_do_while_fork(
        &mut self,
        cond: &IrExpr,
        body: &[IrStmt],
        fork_id: Option<usize>,
    ) -> Result<bool, SimError> {
        let mut iter_count = 0usize;
        loop {
            if iter_count >= MAX_LOOP_ITER {
                eprintln!(
                    "warning: do-while loop exceeded {} iterations, breaking",
                    MAX_LOOP_ITER
                );
                break;
            }
            iter_count += 1;
            if self.disable_pending.is_some() {
                break;
            }
            if self.control_flow.is_some() {
                self.control_flow = None;
                break;
            }
            let old_loop_cont = self.loop_continuation.take();
            let mut lc = vec![IrStmt::LoopDoWhile {
                cond: cond.clone(),
                body: body.to_vec(),
            }];
            if !self.post_loop_tail.is_empty() {
                lc.extend(self.post_loop_tail.clone());
            }
            self.loop_continuation = Some(lc);
            let completed = self.evaluate_block_with_delay_fork(body, fork_id)?;
            self.loop_continuation = old_loop_cont;
            if !completed {
                return Ok(false);
            }
            let cf = self.control_flow.take();
            if cf == Some(FlowControl::Continue) {
                continue;
            }
            if cf == Some(FlowControl::Break) {
                break;
            }
            let cond_val = self.evaluate_expr(cond)?;
            if !cond_val.to_bool().unwrap_or(false) {
                break;
            }
        }
        Ok(true)
    }

    /// Evaluate a do-while loop without delay/fork (stmt context).
    pub(crate) fn evaluate_loop_do_while_stmt(
        &mut self,
        cond: &IrExpr,
        body: &[IrStmt],
    ) -> Result<(), SimError> {
        let mut iter_count = 0usize;
        loop {
            if iter_count >= MAX_LOOP_ITER {
                eprintln!(
                    "warning: do-while loop exceeded {} iterations, breaking",
                    MAX_LOOP_ITER
                );
                break;
            }
            iter_count += 1;
            if self.disable_pending.is_some() {
                break;
            }
            if self.control_flow.is_some() {
                self.control_flow = None;
                break;
            }
            self.evaluate_stmt_block(body)?;
            let cf = self.control_flow.take();
            if cf == Some(FlowControl::Continue) {
                continue;
            }
            if cf == Some(FlowControl::Break) {
                break;
            }
            let cond_val = self.evaluate_expr(cond)?;
            if !cond_val.to_bool().unwrap_or(false) {
                break;
            }
        }
        Ok(())
    }

    // ─── Repeat ──────────────────────────────────────────────────────

    /// Evaluate a Repeat loop with delay/fork support.
    pub(crate) fn evaluate_repeat_fork(
        &mut self,
        count: &IrExpr,
        body: &[IrStmt],
        fork_id: Option<usize>,
    ) -> Result<bool, SimError> {
        let count_val = self.evaluate_expr(count)?;
        let n = (count_val.to_u64() as usize).min(MAX_LOOP_ITER);
        for _ in 0..n {
            if self.disable_pending.is_some() {
                break;
            }
            if self.control_flow.is_some() {
                self.control_flow = None;
                break;
            }
            if !self.evaluate_block_with_delay_fork(body, fork_id)? {
                return Ok(false);
            }
            let cf = self.control_flow.take();
            if cf == Some(FlowControl::Continue) {
                continue;
            }
            if cf == Some(FlowControl::Break) {
                break;
            }
        }
        Ok(true)
    }

    /// Evaluate a Repeat loop without delay/fork (stmt context).
    pub(crate) fn evaluate_repeat_stmt(
        &mut self,
        count: &IrExpr,
        body: &[IrStmt],
    ) -> Result<(), SimError> {
        let count_val = self.evaluate_expr(count)?;
        let n = (count_val.to_u64() as usize).min(MAX_LOOP_ITER);
        for _ in 0..n {
            if self.disable_pending.is_some() {
                break;
            }
            if self.control_flow.is_some() {
                self.control_flow = None;
                break;
            }
            self.evaluate_stmt_block(body)?;
            let cf = self.control_flow.take();
            if cf == Some(FlowControl::Continue) {
                continue;
            }
            if cf == Some(FlowControl::Break) {
                break;
            }
        }
        Ok(())
    }

    // ─── Foreach ─────────────────────────────────────────────────────

    /// Evaluate a foreach loop with delay/fork support.
    pub(crate) fn evaluate_foreach_fork(
        &mut self,
        array_var: &IrExpr,
        index_var: &Symbol,
        body: &[IrStmt],
        fork_id: Option<usize>,
    ) -> Result<bool, SimError> {
        let lv = self.evaluate_expr(array_var)?;
        let sig_info = if let IrExpr::Signal(id, _) = array_var {
            self.design.top.signals.get(*id)
        } else {
            None
        };
        let elem_width = sig_info.map(|s| s.elem_width).unwrap_or(1);
        let count = lv.width.checked_div(elem_width).unwrap_or(0);
        for i in 0..count {
            if self.disable_pending.is_some() {
                break;
            }
            if self.control_flow.is_some() {
                self.control_flow = None;
                break;
            }
            let idx_val = LogicVec::from_u64(i as u64, 32);
            let mut scope = HashMap::new();
            scope.insert(*index_var, idx_val);
            let depth = self.method_locals.len();
            self.method_locals.push(scope);
            if !self.evaluate_block_with_delay_fork(body, fork_id)? {
                self.method_locals.truncate(depth);
                return Ok(false);
            }
            self.method_locals.truncate(depth);
            let cf = self.control_flow.take();
            if cf == Some(FlowControl::Continue) {
                continue;
            }
            if cf == Some(FlowControl::Break) {
                break;
            }
        }
        Ok(true)
    }

    /// Evaluate a foreach loop without delay/fork (stmt context).
    pub(crate) fn evaluate_foreach_stmt(
        &mut self,
        array_var: &IrExpr,
        index_var: &Symbol,
        body: &[IrStmt],
    ) -> Result<(), SimError> {
        let lv = self.evaluate_expr(array_var)?;
        let sig_info = if let IrExpr::Signal(id, _) = array_var {
            self.design.top.signals.get(*id)
        } else {
            None
        };
        let elem_width = sig_info.map(|s| s.elem_width).unwrap_or(1);
        let count = lv.width.checked_div(elem_width).unwrap_or(0);
        for i in 0..count {
            if self.disable_pending.is_some() {
                break;
            }
            if self.control_flow.is_some() {
                self.control_flow = None;
                break;
            }
            let idx_val = LogicVec::from_u64(i as u64, 32);
            let mut scope = HashMap::new();
            scope.insert(*index_var, idx_val);
            let depth = self.method_locals.len();
            self.method_locals.push(scope);
            self.evaluate_stmt_block(body)?;
            self.method_locals.truncate(depth);
            let cf = self.control_flow.take();
            if cf == Some(FlowControl::Continue) {
                continue;
            }
            if cf == Some(FlowControl::Break) {
                break;
            }
        }
        Ok(())
    }
}
