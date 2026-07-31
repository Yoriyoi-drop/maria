use super::SimulationEngine;
use crate::diagnostics::diagnostic::{DiagCode, DiagLevel, Diagnostic, RuntimeContext, SourceSnippet};
use crate::error::SimError;
use crate::ir::*;
use crate::mir::*;
use crate::scheduler::clock_domain::ClockDomain;
use crate::simulator::parallel::ParallelConfig;
use crate::simulator::sdf::SdfData;
use crate::simulator::state::SimulationState;
use crate::simulator::types::*;
use crate::Symbol;
use crate::waveform::{CsvWaveWriter, FstWaveWriter, SignalStats, VcdWriter};
use rand::SeedableRng;
use std::collections::{HashMap, HashSet};

impl SimulationEngine {
    pub fn new(design: IrDesign, max_time: u64) -> Self {
        let state = SimulationState::new(&design);
        SimulationEngine {
            state,
            design,
            max_time,
            running: true,
            events: Vec::new(),
            nba_pending: Vec::new(),
            vcd: None,
            fst: None,
            csv: None,
            signal_stats: None,
            current_this: None,
            method_locals: Vec::new(),
            current_method: None,
            disable_pending: None,
            rng: rand::rngs::StdRng::seed_from_u64(42),
            file_handles: HashMap::new(),
            file_ungetc_buf: HashMap::new(),
            file_read_pos: HashMap::new(),
            next_file_handle: 1,
            monitor_args: None,
            monitor_last_values: None,
            control_flow: None,
            expr_recursion_depth: 0,
            forced_signals: HashSet::new(),
            signal_snapshot: None,
            pending_waits: Vec::new(),
            pending_await_target: None,
            pending_wait_orders: Vec::new(),
            loop_continuation: None,
            post_loop_tail: Vec::new(),
            current_time: 0,
            fork_groups: Vec::new(),
            reactive_events: Vec::new(),
            strobe_events: Vec::new(),
            fstrobe_events: Vec::new(),
            fmonitor_map: HashMap::new(),
            mailbox_queues: HashMap::new(),
            semaphore_counts: HashMap::new(),
            assoc_data: HashMap::new(),
            uvm_object_data: HashMap::new(),
            uvm_component_data: HashMap::new(),
            uvm_sequencer_data: HashMap::new(),
            uvm_driver_data: HashMap::new(),
            uvm_analysis_port_data: HashMap::new(),
            uvm_analysis_imp_data: HashMap::new(),
            uvm_config_db_data: HashMap::new(),
            sdf_timing_checks: Vec::new(),
            uvm_resource_db_data: HashMap::new(),
            uvm_reg_data: std::collections::HashMap::new(),
            uvm_reg_field_data: std::collections::HashMap::new(),
            uvm_reg_block_data: std::collections::HashMap::new(),
            uvm_reg_map_data: std::collections::HashMap::new(),
            callback_queues: HashMap::new(),
            factory_type_overrides: HashMap::new(),
            root_test_obj_id: None,
            process_map: HashMap::new(),
            _next_process_id: 1,
            current_process_id: None,
            cover_hits: HashMap::new(),
            cover_total: HashMap::new(),
            cover_bins: HashMap::new(),
            plusargs: HashMap::new(),
            debug_mode: DebugMode::Normal,
            breakpoints: Vec::new(),
            watchpoints: Vec::new(),
            signal_history: crate::simulator::signal_history::SignalHistoryStore::new(10000, None),
            signal_last_change: HashMap::new(),
            udp_prev_args: HashMap::new(),
            parallel_config: ParallelConfig::default(),
            sysfunc_prev: HashMap::new(),
            sysfunc_history: HashMap::new(),
            snapshots: Vec::new(),
            paused: false,
            step_mode: StepMode::Running,
            event_log: Vec::new(),
            snapshot_interval: 1000,
            assert_off_all: false,
            assert_kill_all: false,
            assert_modules_off: HashSet::new(),
            coverage_options: HashMap::new(),
            coverage_enabled: true,
            coverage_enabled_types: std::collections::HashSet::new(),
            cover_line: HashMap::new(),
            cover_toggle: HashMap::new(),
            cover_branches: HashMap::new(),
            cover_fsm: HashMap::new(),
            cover_branch_counter: 0,
            coverage_model_handles: HashMap::new(),
            next_coverage_model_handle: 1,
            sequence_attempts: Vec::new(),
            recursion_depth: HashMap::new(),
            max_recursion_depth: 256,
            objection_count: 0,
            objection_triggered: false,
            jit_evaluator: Some(crate::simulator::JITEvaluator::new()),
            use_packed_eval: true,
            use_jit_expression: false, // Expression-level JIT: disabled by default (opt-in for stability)
            sim_arena: crate::simulator::arena::SimulationArena::with_bump_size(4 * 1024 * 1024), // 4MB initial
            sim_dag: None,
            use_dag_parallel: false,
            clock_analysis: None,
            use_cycle_fusion: false,
            mir_jit: crate::mir::MirJitCompiler::new(),
            use_mir_jit: false,
            diag_sink: crate::diagnostics::DiagSink::new(),
            current_delta: 0,
            current_process_name: None,
            current_instance_path: None,
            signal_writers: std::collections::HashMap::new(),
            signal_write_count: std::collections::HashMap::new(),            delta_limit: 20_000_000,

            timing_wheel: None,
            use_timing_wheel: false,

            cosim_state: None,
            cosim_signals: Vec::new(),
            signal_delays: std::collections::HashMap::new(),
            power_intent: None,
            process_body_cache: HashMap::new(),
        }
    }

    /// Buat RuntimeContext dari state engine saat ini.
    /// Menyertakan: time, delta, module, process, seed.
    pub fn runtime_context(&self) -> RuntimeContext {
        let mut ctx = RuntimeContext::new()
            .with_time(format!("{} ns", self.state.time))
            .with_delta(self.current_delta)
            .with_module(self.design.top.name.as_str());
        if let Some(ref pname) = self.current_process_name {
            ctx = ctx.with_process(pname.clone());
        }
        if let Some(ref inst) = self.current_instance_path {
            ctx = ctx.with_instance(inst.clone());
        }
        ctx
    }

    /// Emit diagnostic runtime ke DiagSink.
    pub fn emit_diag(&self, level: DiagLevel, code: DiagCode, message: impl Into<String>) {
        let msg: String = message.into();
        let diag = Diagnostic::new(level, code, msg)
            .with_runtime_context(self.runtime_context());
        self.diag_sink.push(diag);
    }

    /// Emit warning diagnostic ke DiagSink.
    pub fn emit_warning(&self, code: DiagCode, message: impl Into<String>) {
        self.emit_diag(DiagLevel::Warning, code, message);
    }

    /// Emit error diagnostic ke DiagSink dan return SimError dengan full context.
    pub fn diag_error(&self, code: DiagCode, message: impl Into<String>) -> SimError {
        self.diag_error_at(code, message, 0, 0)
    }

    /// Emit error diagnostic dengan posisi source (line, col).
    pub fn diag_error_at(&self, code: DiagCode, message: impl Into<String>, line: usize, col: usize) -> SimError {
        let msg: String = message.into();
        let mut diag = Diagnostic::new(DiagLevel::Error, code, msg)
            .with_runtime_context(self.runtime_context())
            .with_code_context();
        // Add source snippet if source lines are available
        if line > 0 {
            if let Some(ref source_lines) = self.design.source_lines {
                if line <= source_lines.len() {
                    let source_line = &source_lines[line - 1];
                    let file = self.design.source_file.as_deref().unwrap_or("<unknown>");
                    diag = diag.with_source_snippet(SourceSnippet::new(file, line, col, source_line));
                }
            }
        }
        self.diag_sink.push(diag.clone());
        SimError::Diagnostic(diag)
    }

    /// Emit fatal diagnostic ke DiagSink dan return SimError dengan full context.
    pub fn diag_fatal(&self, code: DiagCode, message: impl Into<String>) -> SimError {
        self.diag_fatal_at(code, message, 0, 0)
    }

    /// Emit fatal diagnostic dengan posisi source.
    pub fn diag_fatal_at(&self, code: DiagCode, message: impl Into<String>, line: usize, col: usize) -> SimError {
        let msg: String = message.into();
        let mut diag = Diagnostic::new(DiagLevel::Fatal, code, msg)
            .with_runtime_context(self.runtime_context())
            .with_code_context();
        if line > 0 {
            if let Some(ref source_lines) = self.design.source_lines {
                if line <= source_lines.len() {
                    let source_line = &source_lines[line - 1];
                    let file = self.design.source_file.as_deref().unwrap_or("<unknown>");
                    diag = diag.with_source_snippet(SourceSnippet::new(file, line, col, source_line));
                }
            }
        }
        self.diag_sink.push(diag.clone());
        SimError::Diagnostic(diag)
    }

    /// Flush diagnostics from DiagSink and return them.
    pub fn flush_diagnostics(&self) -> Vec<Diagnostic> {
        self.diag_sink.diagnostics()
    }

    pub fn set_vcd(&mut self, vcd: VcdWriter) {
        self.vcd = Some(vcd);
    }

    pub fn set_fst(&mut self, fst: FstWaveWriter) {
        self.fst = Some(fst);
    }

    pub fn set_csv(&mut self, csv: CsvWaveWriter) {
        self.csv = Some(csv);
    }

    pub fn set_signal_stats(&mut self, stats: SignalStats) {
        self.signal_stats = Some(stats);
    }

    pub fn set_parallel_config(&mut self, config: ParallelConfig) {
        self.parallel_config = config;
    }

    pub fn set_use_packed_eval(&mut self, enabled: bool) {
        self.use_packed_eval = enabled;
    }

    pub fn set_use_jit_expression(&mut self, enabled: bool) {
        self.use_jit_expression = enabled;
    }

    pub fn set_delta_limit(&mut self, limit: u64) {
        self.delta_limit = limit;
    }

    pub fn set_use_dag_parallel(&mut self, enabled: bool) {
        self.use_dag_parallel = enabled;
    }

    pub fn set_use_cycle_fusion(&mut self, enabled: bool) {
        self.use_cycle_fusion = enabled;
    }

    pub fn set_use_mir_jit(&mut self, enabled: bool) {
        self.use_mir_jit = enabled;
    }

    /// Ensure the events Vec is large enough to hold events at time `t`.
    pub fn ensure_events(&mut self, t: usize) {
        if t >= self.events.len() {
            self.events.resize(t + 1, Vec::new());
        }
    }

    /// Push an event at time `t` with dynamic allocation.
    /// When `use_timing_wheel` is enabled, events at future times (t > current_time)
    /// are stored in the hierarchical timing wheel for O(1) scheduling.
    /// Events at the current time are still stored in `events[t]` directly
    /// so the delta loop can process them without wheel advance overhead.
    pub fn push_event(&mut self, t: usize, event: RegionEvent) {
        if self.use_timing_wheel {
            if let Some(ref mut wheel) = self.timing_wheel {
                let current = self.state.time as usize;
                if t > current {
                    // Future event — store in timing wheel
                    wheel.add_event(t, event.region, event.event);
                    return;
                }
                // Current or past time — fall through to events[t]
            }
        }
        self.ensure_events(t);
        self.events[t].push(event);
    }

    /// Enable the hierarchical timing wheel for O(1) event scheduling.
    /// Call before run(). Allocates the wheel on first use.
    pub fn set_use_timing_wheel(&mut self, enabled: bool) {
        self.use_timing_wheel = enabled;
        if enabled && self.timing_wheel.is_none() {
            self.timing_wheel = Some(
                crate::simulator::engine::scheduler::timing_wheel::HierarchicalTimingWheel::new()
            );
        }
    }

    /// Map Ir BinaryIrOp to MirBinOp (for MIR JIT compilation).
    fn ir_binop_to_mir(op: &crate::ir::BinaryIrOp) -> MirBinOp {
        match op {
            crate::ir::BinaryIrOp::Add => MirBinOp::Add,
            crate::ir::BinaryIrOp::Sub => MirBinOp::Sub,
            crate::ir::BinaryIrOp::Mul => MirBinOp::Mul,
            crate::ir::BinaryIrOp::Div => MirBinOp::Div,
            crate::ir::BinaryIrOp::Mod => MirBinOp::Mod,
            crate::ir::BinaryIrOp::BitAnd => MirBinOp::And,
            crate::ir::BinaryIrOp::BitOr => MirBinOp::Or,
            crate::ir::BinaryIrOp::BitXor => MirBinOp::Xor,
            crate::ir::BinaryIrOp::Eq
            | crate::ir::BinaryIrOp::CaseEq
            | crate::ir::BinaryIrOp::EqWild => MirBinOp::Eq,
            crate::ir::BinaryIrOp::Neq
            | crate::ir::BinaryIrOp::CaseNeq
            | crate::ir::BinaryIrOp::NeqWild => MirBinOp::Ne,
            crate::ir::BinaryIrOp::Lt => MirBinOp::Lt,
            crate::ir::BinaryIrOp::Le => MirBinOp::Le,
            crate::ir::BinaryIrOp::Gt => MirBinOp::Gt,
            crate::ir::BinaryIrOp::Ge => MirBinOp::Ge,
            crate::ir::BinaryIrOp::Shl => MirBinOp::Shl,
            crate::ir::BinaryIrOp::Shr => MirBinOp::Shr,
            _ => MirBinOp::Add, // fallback
        }
    }

    /// Check if an IrExpr contains any unsupported variants that would produce
    /// incorrect results in the JIT path. Returns true if the expression can be JIT-compiled.
    fn is_expr_jit_safe(expr: &IrExpr) -> bool {
        match expr {
            IrExpr::Const(_) => true,
            IrExpr::Signal(_, _) => true,
            IrExpr::FillLit(lv) => matches!(lv, LogicVal::Zero | LogicVal::One),
            IrExpr::BinaryOp(_, lhs, rhs) => {
                Self::is_expr_jit_safe(lhs) && Self::is_expr_jit_safe(rhs)
            }
            IrExpr::UnaryOp(op, inner) => {
                match op {
                    crate::ir::UnaryIrOp::BitNot | crate::ir::UnaryIrOp::Minus
                    | crate::ir::UnaryIrOp::Plus => Self::is_expr_jit_safe(inner),
                    _ => false,
                }
            }
            // Cond (ternary) supported with Branch/Jump/Label in MIR JIT phase 3
            IrExpr::Cond(cond, t, f) => {
                Self::is_expr_jit_safe(cond)
                    && Self::is_expr_jit_safe(t)
                    && Self::is_expr_jit_safe(f)
            }
            IrExpr::Concat(exprs) => exprs.iter().all(|e| Self::is_expr_jit_safe(e)),
            IrExpr::Cast { expr: inner, .. } => Self::is_expr_jit_safe(inner),
            IrExpr::Signed(inner) => Self::is_expr_jit_safe(inner),
            _ => false,
        }
    }

    /// Check if an IrStmt body is fully JIT-safe (all expressions within are supported).
    fn is_body_jit_safe(body: &[IrStmt]) -> bool {
        for stmt in body {
            match stmt {
                IrStmt::BlockingAssign { lhs, rhs, delay } => {
                    if delay.is_some() {
                        return false;
                    }
                    if !matches!(lhs, IrLValue::Signal(_, _)) {
                        return false; // Only simple Signal lvalues supported
                    }
                    if !Self::is_expr_jit_safe(rhs) {
                        return false;
                    }
                }
                IrStmt::Block { stmts: inner }
                | IrStmt::NamedBlock { stmts: inner, .. } => {
                    if !Self::is_body_jit_safe(inner) {
                        return false;
                    }
                }
                IrStmt::If {
                    cond,
                    true_branch,
                    false_branch,
                } => {
                    if !Self::is_expr_jit_safe(cond) {
                        return false;
                    }
                    if !Self::is_body_jit_safe(true_branch) {
                        return false;
                    }
                    if !Self::is_body_jit_safe(false_branch) {
                        return false;
                    }
                }
                IrStmt::Case {
                    expr: case_expr,
                    items,
                    default,
                    ..
                } => {
                    if !Self::is_expr_jit_safe(case_expr) {
                        return false;
                    }
                    for item in items {
                        for pat in &item.labels {
                            if !Self::is_expr_jit_safe(pat) {
                                return false;
                            }
                        }
                        if !Self::is_body_jit_safe(&item.body) {
                            return false;
                        }
                    }
                    if !Self::is_body_jit_safe(default) {
                        return false;
                    }
                }
                IrStmt::LoopWhile { cond, body, .. }
                | IrStmt::LoopDoWhile { cond, body, .. } => {
                    if !Self::is_expr_jit_safe(cond) {
                        return false;
                    }
                    if !Self::is_body_jit_safe(body) {
                        return false;
                    }
                }
                IrStmt::LoopFor {
                    init,
                    cond,
                    step,
                    body,
                    ..
                } => {
                    if let Some(init_stmt) = init {
                        if !Self::is_body_jit_safe(&[init_stmt.as_ref().clone()]) {
                            return false;
                        }
                    }
                    if !Self::is_expr_jit_safe(cond) {
                        return false;
                    }
                    if let Some(step_stmt) = step {
                        if !Self::is_body_jit_safe(&[step_stmt.as_ref().clone()]) {
                            return false;
                        }
                    }
                    if !Self::is_body_jit_safe(body) {
                        return false;
                    }
                }
                IrStmt::Repeat { count, body, .. } => {
                    if !Self::is_expr_jit_safe(count) {
                        return false;
                    }
                    if !Self::is_body_jit_safe(body) {
                        return false;
                    }
                }
                IrStmt::NonBlockingAssign { lhs, rhs, delay } => {
                    if delay.is_some() {
                        return false;
                    }
                    if !matches!(lhs, IrLValue::Signal(_, _)) {
                        return false;
                    }
                    if !Self::is_expr_jit_safe(rhs) {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        true
    }

    /// Lower an IrExpr into MIR instructions, storing the result in `dest_reg`.
    /// Caller must ensure `is_expr_jit_safe()` returned true first.
    fn ir_expr_to_mir(
        expr: &IrExpr,
        instrs: &mut Vec<crate::mir::MirInstr>,
        dest_reg: usize,
        next_reg: &mut usize,
    ) {
        match expr {
            IrExpr::Const(lv) => {
                instrs.push(crate::mir::MirInstr::Const {
                    dest: dest_reg,
                    value: lv.to_u64(),
                    width: lv.width.max(1),
                });
            }
            IrExpr::Signal(id, _width) => {
                instrs.push(crate::mir::MirInstr::Load {
                    dest: dest_reg,
                    signal: *id,
                });
            }
            IrExpr::FillLit(lv) => {
                let val = match lv {
                    LogicVal::Zero => 0u64,
                    LogicVal::One => 1u64,
                    _ => 0u64,
                };
                instrs.push(crate::mir::MirInstr::Const {
                    dest: dest_reg,
                    value: val,
                    width: 1,
                });
            }
            IrExpr::BinaryOp(op, lhs, rhs) => {
                let lhs_reg = *next_reg;
                *next_reg += 1;
                let rhs_reg = *next_reg;
                *next_reg += 1;
                Self::ir_expr_to_mir(lhs, instrs, lhs_reg, next_reg);
                Self::ir_expr_to_mir(rhs, instrs, rhs_reg, next_reg);
                let mir_op = Self::ir_binop_to_mir(op);
                let width = Self::compute_expr_width(expr).unwrap_or(64);
                instrs.push(crate::mir::MirInstr::Binary {
                    op: mir_op,
                    dest: dest_reg,
                    lhs: lhs_reg,
                    rhs: rhs_reg,
                    width,
                });
            }
            IrExpr::UnaryOp(op, inner) => {
                let inner_reg = *next_reg;
                *next_reg += 1;
                Self::ir_expr_to_mir(inner, instrs, inner_reg, next_reg);
                let width = Self::compute_expr_width(expr).unwrap_or(64);
                let mir_op = match op {
                    crate::ir::UnaryIrOp::BitNot => crate::mir::MirUnOp::Not,
                    crate::ir::UnaryIrOp::Minus => crate::mir::MirUnOp::Neg,
                    _ => unreachable!(), // is_expr_jit_safe guarantees this
                };
                instrs.push(crate::mir::MirInstr::Unary {
                    op: mir_op,
                    dest: dest_reg,
                    operand: inner_reg,
                    width,
                });
            }
            // Cond (ternary): write result directly to dest_reg via separate branches
            IrExpr::Cond(cond, t, f) => {
                let cond_reg = *next_reg;
                *next_reg += 1;
                Self::ir_expr_to_mir(cond, instrs, cond_reg, next_reg);
                let _width = Self::compute_expr_width(expr).unwrap_or(1);
                let then_label = Self::next_label(instrs);
                let else_label = Self::next_label(instrs) + 1;
                let end_label = Self::next_label(instrs) + 2;
                instrs.push(crate::mir::MirInstr::Branch {
                    cond: cond_reg,
                    then_label,
                    else_label,
                });
                // Then branch: compute true value into dest_reg
                instrs.push(crate::mir::MirInstr::Label(then_label));
                Self::ir_expr_to_mir(t, instrs, dest_reg, next_reg);
                instrs.push(crate::mir::MirInstr::Jump { label: end_label });
                // Else branch: compute false value into dest_reg
                instrs.push(crate::mir::MirInstr::Label(else_label));
                Self::ir_expr_to_mir(f, instrs, dest_reg, next_reg);
                instrs.push(crate::mir::MirInstr::Label(end_label));
            }
            // Concat: shift each part into place and OR
            IrExpr::Concat(exprs) => {
                let width = Self::compute_expr_width(expr).unwrap_or(64);
                instrs.push(crate::mir::MirInstr::Const { dest: dest_reg, value: 0, width });
                let mut offset = 0usize;
                for part in exprs.iter().rev() {
                    let part_reg = *next_reg;
                    *next_reg += 1;
                    Self::ir_expr_to_mir(part, instrs, part_reg, next_reg);
                    let part_w = Self::compute_expr_width(part).unwrap_or(1);
                    if offset > 0 {
                        let shift_reg = *next_reg;
                        *next_reg += 1;
                        instrs.push(crate::mir::MirInstr::Const { dest: shift_reg, value: offset as u64, width: 64 });
                        instrs.push(crate::mir::MirInstr::Binary {
                            op: crate::mir::MirBinOp::Shl,
                            dest: part_reg,
                            lhs: part_reg,
                            rhs: shift_reg,
                            width,
                        });
                    }
                    instrs.push(crate::mir::MirInstr::Binary {
                        op: crate::mir::MirBinOp::Or,
                        dest: dest_reg,
                        lhs: dest_reg,
                        rhs: part_reg,
                        width,
                    });
                    offset += part_w;
                }
            }
            // Cast: resize width (extend or truncate)
            IrExpr::Cast { expr: inner, width } => {
                let inner_reg = *next_reg;
                *next_reg += 1;
                Self::ir_expr_to_mir(inner, instrs, inner_reg, next_reg);
                if Self::compute_expr_width(inner).unwrap_or(1) != *width {
                    let mask = if *width < 64 { ((1u64 << width) - 1) as i64 } else { -1i64 };
                    let mask_reg = *next_reg;
                    *next_reg += 1;
                    instrs.push(crate::mir::MirInstr::Const { dest: mask_reg, value: mask as u64, width: *width });
                    instrs.push(crate::mir::MirInstr::Binary {
                        op: crate::mir::MirBinOp::And,
                        dest: dest_reg,
                        lhs: inner_reg,
                        rhs: mask_reg,
                        width: *width,
                    });
                } else {
                    // Same width — copy via OR with 0
                    instrs.push(crate::mir::MirInstr::Const { dest: dest_reg, value: 0, width: *width });
                    instrs.push(crate::mir::MirInstr::Binary {
                        op: crate::mir::MirBinOp::Or,
                        dest: dest_reg,
                        lhs: dest_reg,
                        rhs: inner_reg,
                        width: *width,
                    });
                }
            }
            // Signed: pass-through (width already accounts for signedness in MIR)
            IrExpr::Signed(inner) => {
                let inner_reg = *next_reg;
                *next_reg += 1;
                Self::ir_expr_to_mir(inner, instrs, inner_reg, next_reg);
                let width = Self::compute_expr_width(expr).unwrap_or(64);
                instrs.push(crate::mir::MirInstr::Const { dest: dest_reg, value: 0, width });
                instrs.push(crate::mir::MirInstr::Binary {
                    op: crate::mir::MirBinOp::Or,
                    dest: dest_reg,
                    lhs: dest_reg,
                    rhs: inner_reg,
                    width,
                });
            }
            _ => {
                unreachable!("ir_expr_to_mir called on unsupported expr variant");
            }
        }
    }

    /// Generate a unique label number for MIR Branch/Jump/Label instructions.
    fn next_label(instrs: &[crate::mir::MirInstr]) -> usize {
        let max_label = instrs.iter().filter_map(|i| {
            if let crate::mir::MirInstr::Label(l) = i { Some(*l) } else { None }
        }).max().unwrap_or(0);
        max_label + 1
    }

    /// Lower an IrStmt block into MIR instructions.
    /// Returns None if any statement or expression is unsupported (requires interpreter fallback).
    fn ir_body_to_mir(
        body: &[IrStmt],
        n_sigs: usize,
        mir_name: Symbol,
    ) -> Option<crate::mir::MirProcess> {
        // Pre-check: ensure ALL statements and sub-expressions are JIT-safe
        if !Self::is_body_jit_safe(body) {
            return None;
        }

        let mut instrs = Vec::new();
        let mut next_reg = 0usize;

        for stmt in body {
            match stmt {
                IrStmt::BlockingAssign { lhs, rhs, .. } => {
                    if let IrLValue::Signal(sig_id, _) = lhs {
                        if *sig_id >= n_sigs {
                            return None;
                        }
                        let dest_reg = next_reg;
                        next_reg += 1;
                        Self::ir_expr_to_mir(rhs, &mut instrs, dest_reg, &mut next_reg);
                        instrs.push(crate::mir::MirInstr::Store {
                            signal: *sig_id,
                            src: dest_reg,
                        });
                    } else {
                        return None;
                    }
                }
                // Block / NamedBlock: flatten inner statements
                IrStmt::Block { stmts: inner }
                | IrStmt::NamedBlock { stmts: inner, .. } => {
                    let (inner_instrs, _) = Self::ir_body_to_mir_inner(inner, n_sigs, next_reg)?;
                    instrs.extend(inner_instrs);
                    next_reg = Self::max_reg_used(&instrs).unwrap_or(next_reg);
                }
                // If: branch on condition
                IrStmt::If {
                    cond,
                    true_branch,
                    false_branch,
                } => {
                    let cond_reg = next_reg;
                    next_reg += 1;
                    Self::ir_expr_to_mir(cond, &mut instrs, cond_reg, &mut next_reg);
                    let then_label = Self::next_label(&instrs);
                    let else_label = Self::next_label(&instrs) + 1;
                    let end_label = Self::next_label(&instrs) + 2;
                    instrs.push(crate::mir::MirInstr::Branch {
                        cond: cond_reg,
                        then_label,
                        else_label,
                    });
                    instrs.push(crate::mir::MirInstr::Label(then_label));
                    let (t_instrs, _) = Self::ir_body_to_mir_inner(true_branch, n_sigs, next_reg)?;
                    instrs.extend(t_instrs);
                    next_reg = Self::max_reg_used(&instrs).unwrap_or(next_reg);
                    instrs.push(crate::mir::MirInstr::Jump { label: end_label });
                    instrs.push(crate::mir::MirInstr::Label(else_label));
                    let (f_instrs, _) = Self::ir_body_to_mir_inner(false_branch, n_sigs, next_reg)?;
                    instrs.extend(f_instrs);
                    next_reg = Self::max_reg_used(&instrs).unwrap_or(next_reg);
                    instrs.push(crate::mir::MirInstr::Label(end_label));
                }
                // Case: equality chain with dispatch
                IrStmt::Case {
                    expr: case_expr,
                    items,
                    default,
                    ..
                } => {
                    let case_reg = next_reg;
                    next_reg += 1;
                    Self::ir_expr_to_mir(case_expr, &mut instrs, case_reg, &mut next_reg);
                    let mut item_labels = Vec::new();
                    for _ in items {
                        item_labels.push(Self::next_label(&instrs) + item_labels.len() + 1);
                    }
                    let default_label = Self::next_label(&instrs) + item_labels.len() + 1;
                    let end_case_label = Self::next_label(&instrs) + item_labels.len() + 2;
                    for (i, item) in items.iter().enumerate() {
                        // Compare case_reg with each label expression
                        // Use a chain: if match → item_labels[i], else → next comparison or default
                        let next_test_label = if i + 1 < items.len() {
                            Self::next_label(&instrs) + item_labels.len() + 3 + i
                        } else {
                            default_label
                        };
                        for pat in &item.labels {
                            let pat_reg = next_reg;
                            next_reg += 1;
                            Self::ir_expr_to_mir(pat, &mut instrs, pat_reg, &mut next_reg);
                            let eq_reg = next_reg;
                            next_reg += 1;
                            instrs.push(crate::mir::MirInstr::Binary {
                                op: crate::mir::MirBinOp::Eq,
                                dest: eq_reg,
                                lhs: case_reg,
                                rhs: pat_reg,
                                width: 1,
                            });
                            // If match → item body, else continue to next pat/next item
                            let next_pat_label = next_test_label;
                            instrs.push(crate::mir::MirInstr::Branch {
                                cond: eq_reg,
                                then_label: item_labels[i],
                                else_label: next_pat_label,
                            });
                        }
                    }
                    // Item bodies
                    for (i, item) in items.iter().enumerate() {
                        instrs.push(crate::mir::MirInstr::Label(item_labels[i]));
                        let (item_instrs, _) = Self::ir_body_to_mir_inner(&item.body, n_sigs, next_reg)?;
                        instrs.extend(item_instrs);
                        next_reg = Self::max_reg_used(&instrs).unwrap_or(next_reg);
                        instrs.push(crate::mir::MirInstr::Jump { label: end_case_label });
                    }
                    // Default body
                    instrs.push(crate::mir::MirInstr::Label(default_label));
                    let (def_instrs, _) = Self::ir_body_to_mir_inner(default, n_sigs, next_reg)?;
                    instrs.extend(def_instrs);
                    next_reg = Self::max_reg_used(&instrs).unwrap_or(next_reg);
                    instrs.push(crate::mir::MirInstr::Label(end_case_label));
                }
                // LoopWhile: while(cond) body;
                IrStmt::LoopWhile { cond, body, .. } => {
                    let loop_start = Self::next_label(&instrs);
                    instrs.push(crate::mir::MirInstr::Label(loop_start));
                    let cond_reg = next_reg;
                    next_reg += 1;
                    Self::ir_expr_to_mir(cond, &mut instrs, cond_reg, &mut next_reg);
                    let body_label = Self::next_label(&instrs) + 1;
                    let end_label = Self::next_label(&instrs) + 2;
                    instrs.push(crate::mir::MirInstr::Branch {
                        cond: cond_reg,
                        then_label: body_label,
                        else_label: end_label,
                    });
                    instrs.push(crate::mir::MirInstr::Label(body_label));
                    let (b_instrs, _) = Self::ir_body_to_mir_inner(body, n_sigs, next_reg)?;
                    instrs.extend(b_instrs);
                    next_reg = Self::max_reg_used(&instrs).unwrap_or(next_reg);
                    instrs.push(crate::mir::MirInstr::Jump { label: loop_start });
                    instrs.push(crate::mir::MirInstr::Label(end_label));
                }
                // LoopDoWhile: do body; while(cond);
                IrStmt::LoopDoWhile { cond, body, .. } => {
                    let loop_start = Self::next_label(&instrs);
                    instrs.push(crate::mir::MirInstr::Label(loop_start));
                    let (b_instrs, _) = Self::ir_body_to_mir_inner(body, n_sigs, next_reg)?;
                    instrs.extend(b_instrs);
                    next_reg = Self::max_reg_used(&instrs).unwrap_or(next_reg);
                    let cond_reg = next_reg;
                    next_reg += 1;
                    Self::ir_expr_to_mir(cond, &mut instrs, cond_reg, &mut next_reg);
                    let end_label = Self::next_label(&instrs) + 1;
                    instrs.push(crate::mir::MirInstr::Branch {
                        cond: cond_reg,
                        then_label: loop_start,
                        else_label: end_label,
                    });
                    instrs.push(crate::mir::MirInstr::Label(end_label));
                }
                // LoopFor: for(init; cond; step) body;
                IrStmt::LoopFor {
                    init,
                    cond,
                    step,
                    body,
                    ..
                } => {
                    if let Some(init_stmt) = init {
                        let (i_instrs, _) = Self::ir_body_to_mir_inner(&[init_stmt.as_ref().clone()], n_sigs, next_reg)?;
                        instrs.extend(i_instrs);
                        next_reg = Self::max_reg_used(&instrs).unwrap_or(next_reg);
                    }
                    let loop_start = Self::next_label(&instrs);
                    instrs.push(crate::mir::MirInstr::Label(loop_start));
                    let cond_reg = next_reg;
                    next_reg += 1;
                    Self::ir_expr_to_mir(cond, &mut instrs, cond_reg, &mut next_reg);
                    let body_label = Self::next_label(&instrs) + 1;
                    let end_label = Self::next_label(&instrs) + 2;
                    instrs.push(crate::mir::MirInstr::Branch {
                        cond: cond_reg,
                        then_label: body_label,
                        else_label: end_label,
                    });
                    instrs.push(crate::mir::MirInstr::Label(body_label));
                    let (b_instrs, _) = Self::ir_body_to_mir_inner(body, n_sigs, next_reg)?;
                    instrs.extend(b_instrs);
                    next_reg = Self::max_reg_used(&instrs).unwrap_or(next_reg);
                    if let Some(step_stmt) = step {
                        let (s_instrs, _) = Self::ir_body_to_mir_inner(&[step_stmt.as_ref().clone()], n_sigs, next_reg)?;
                        instrs.extend(s_instrs);
                        next_reg = Self::max_reg_used(&instrs).unwrap_or(next_reg);
                    }
                    instrs.push(crate::mir::MirInstr::Jump { label: loop_start });
                    instrs.push(crate::mir::MirInstr::Label(end_label));
                }
                // Repeat: repeat(count) body;
                IrStmt::Repeat { count, body, .. } => {
                    let count_reg = next_reg;
                    next_reg += 1;
                    Self::ir_expr_to_mir(count, &mut instrs, count_reg, &mut next_reg);
                    let counter_reg = next_reg;
                    next_reg += 1;
                    instrs.push(crate::mir::MirInstr::Const { dest: counter_reg, value: 0, width: 32 });
                    let loop_start = Self::next_label(&instrs);
                    instrs.push(crate::mir::MirInstr::Label(loop_start));
                    // cond: counter < count
                    let lt_reg = next_reg;
                    next_reg += 1;
                    instrs.push(crate::mir::MirInstr::Binary {
                        op: crate::mir::MirBinOp::Lt,
                        dest: lt_reg,
                        lhs: counter_reg,
                        rhs: count_reg,
                        width: 1,
                    });
                    let body_label = Self::next_label(&instrs) + 1;
                    let end_label = Self::next_label(&instrs) + 2;
                    instrs.push(crate::mir::MirInstr::Branch {
                        cond: lt_reg,
                        then_label: body_label,
                        else_label: end_label,
                    });
                    instrs.push(crate::mir::MirInstr::Label(body_label));
                    let (b_instrs, _) = Self::ir_body_to_mir_inner(body, n_sigs, next_reg)?;
                    instrs.extend(b_instrs);
                    next_reg = Self::max_reg_used(&instrs).unwrap_or(next_reg);
                    // counter++
                    let one_reg = next_reg;
                    next_reg += 1;
                    instrs.push(crate::mir::MirInstr::Const { dest: one_reg, value: 1, width: 32 });
                    instrs.push(crate::mir::MirInstr::Binary {
                        op: crate::mir::MirBinOp::Add,
                        dest: counter_reg,
                        lhs: counter_reg,
                        rhs: one_reg,
                        width: 32,
                    });
                    instrs.push(crate::mir::MirInstr::Jump { label: loop_start });
                    instrs.push(crate::mir::MirInstr::Label(end_label));
                }
                // NonBlockingAssign: write to output buffer (JIT: same as Store, caller handles NBA semantics)
                IrStmt::NonBlockingAssign { lhs, rhs, .. } => {
                    if let IrLValue::Signal(sig_id, _) = lhs {
                        if *sig_id >= n_sigs {
                            return None;
                        }
                        let dest_reg = next_reg;
                        next_reg += 1;
                        Self::ir_expr_to_mir(rhs, &mut instrs, dest_reg, &mut next_reg);
                        instrs.push(crate::mir::MirInstr::NonBlocking {
                            signal: *sig_id,
                            src: dest_reg,
                            delay: None,
                        });
                    } else {
                        return None;
                    }
                }
                _ => {
                    return None;
                }
            }
        }

        if instrs.is_empty() {
            return None;
        }

        Some(crate::mir::MirProcess {
            name: mir_name,
            sensitivity: crate::mir::MirSensitivity::AlwaysComb,
            instrs,
        })
    }

    /// Lower an IrStmt block into MIR instructions (inner helper).
    /// Delegates to ir_body_to_mir for full statement support (If, Case, loops, etc.).
    fn ir_body_to_mir_inner(
        body: &[IrStmt],
        n_sigs: usize,
        start_reg: usize,
    ) -> Option<(Vec<crate::mir::MirInstr>, usize)> {
        if body.is_empty() {
            return Some((Vec::new(), start_reg));
        }
        let mir = Self::ir_body_to_mir(body, n_sigs, Symbol::intern("__inner"))?;
        let max_reg = mir.instrs.iter().filter_map(|i| {
            match i {
                crate::mir::MirInstr::Const { dest, .. }
                | crate::mir::MirInstr::Load { dest, .. }
                | crate::mir::MirInstr::Binary { dest, .. }
                | crate::mir::MirInstr::Unary { dest, .. } => Some(*dest + 1),
                _ => None,
            }
        }).max().unwrap_or(start_reg);
        Some((mir.instrs, max_reg.max(start_reg)))
    }

    /// Find the maximum register index used in a list of MIR instructions.
    fn max_reg_used(instrs: &[crate::mir::MirInstr]) -> Option<usize> {
        let mut max_reg = 0usize;
        for instr in instrs {
            match instr {
                crate::mir::MirInstr::Const { dest, .. }
                | crate::mir::MirInstr::Load { dest, .. }
                | crate::mir::MirInstr::Binary { dest, .. }
                | crate::mir::MirInstr::Unary { dest, .. } => {
                    max_reg = max_reg.max(*dest);
                }
                _ => {}
            }
        }
        if max_reg == 0 { None } else { Some(max_reg + 1) }
    }

    /// Try to evaluate a process using MIR JIT (compiled-code simulation path).
    /// Returns Ok(true) if JIT was used, Ok(false) if fallback to interpreted needed.
    ///
    /// Handles both blocking and non-blocking assignments:
    /// - Blocking assignments (IrStmt::BlockingAssign) → applied directly via write_signal
    /// - Non-blocking assignments (IrStmt::NonBlockingAssign) → queued to nba_pending
    pub fn try_evaluate_mir_jit(&mut self, pid: usize, body: &[IrStmt]) -> Result<bool, SimError> {
        if !self.use_mir_jit {
            return Ok(false);
        }
        let jit = match &mut self.mir_jit {
            Some(jit) => jit,
            None => return Ok(false),
        };
        // Check if body has any delay statements — if so, fallback
        let has_delay = body.iter().any(|s| matches!(s, IrStmt::Delay { .. }));
        if has_delay {
            return Ok(false);
        }
        // Convert IrStmt block to MIR
        let n_sigs = self.state.signals.len();
        let mir_name = Symbol::intern(&format!("jit_proc_{}", pid));
        let mir_process = match Self::ir_body_to_mir(body, n_sigs, mir_name) {
            Some(p) => p,
            None => return Ok(false),
        };
        // Pre-scan: recursively identify all signal IDs with NBA assignments in this body.
        // These must go through nba_pending, not direct write_signal.
        // Uses recursion to handle NBA inside if/case/loop/block control flow.
        fn collect_nba_signals(stmts: &[IrStmt]) -> HashSet<usize> {
            let mut targets = HashSet::new();
            for stmt in stmts {
                match stmt {
                    IrStmt::NonBlockingAssign { lhs, .. } => {
                        if let IrLValue::Signal(id, _) = lhs {
                            targets.insert(*id);
                        }
                    }
                    IrStmt::Block { stmts: inner }
                    | IrStmt::NamedBlock { stmts: inner, .. } => {
                        targets.extend(collect_nba_signals(inner));
                    }
                    IrStmt::If {
                        true_branch,
                        false_branch,
                        ..
                    } => {
                        targets.extend(collect_nba_signals(true_branch));
                        targets.extend(collect_nba_signals(false_branch));
                    }
                    IrStmt::Case {
                        items, default, ..
                    } => {
                        for item in items {
                            targets.extend(collect_nba_signals(&item.body));
                        }
                        targets.extend(collect_nba_signals(default));
                    }
                    IrStmt::LoopWhile { body, .. }
                    | IrStmt::LoopDoWhile { body, .. }
                    | IrStmt::LoopFor { body, .. }
                    | IrStmt::Repeat { body, .. } => {
                        targets.extend(collect_nba_signals(body));
                    }
                    _ => {}
                }
            }
            targets
        }
        let nba_targets = collect_nba_signals(body);

        // Compile to native code (or cache hit)
        let compiled = match jit.compile_process(&mir_process, n_sigs) {
            Some(c) => c,
            None => return Ok(false),
        };
        // Extract signal values
        let mut signal_vals = vec![0u64; n_sigs.max(1)];
        let mut out_vals = vec![0u64; n_sigs.max(1)];
        for i in 0..n_sigs {
            signal_vals[i] = self.state.read_signal(i).to_u64();
        }
        // Execute compiled native code
        unsafe {
            crate::mir::MirJitCompiler::call_process(compiled.code_ptr, &signal_vals, &mut out_vals);
        }
        // Apply output values back to state — differentiate blocking vs NBA
        for (i, &val) in out_vals.iter().enumerate() {
            if i < n_sigs && val != signal_vals[i] {
                let current = self.state.read_signal(i);
                let new_lv = LogicVec::from_u64(val, current.width.max(1));
                if *current != new_lv {
                    if nba_targets.contains(&i) {
                        // Non-blocking assign: queue to nba_pending for NBA region commit
                        self.nba_pending.push((IrLValue::Signal(i, current.width), new_lv));
                    } else {
                        // Blocking assign: apply directly
                        self.state.write_signal(i, new_lv);
                    }
                }
            }
        }
        Ok(true)
    }

    /// Compute approximate width of an IrExpr.
    fn compute_expr_width(expr: &IrExpr) -> Option<usize> {
        match expr {
            IrExpr::Const(lv) => Some(lv.width),
            IrExpr::Signal(_, w) => Some(*w),
            IrExpr::BinaryOp(_, lhs, rhs) => {
                let lw = Self::compute_expr_width(lhs)?;
                let rw = Self::compute_expr_width(rhs)?;
                Some(lw.max(rw))
            }
            IrExpr::UnaryOp(_, inner) => Self::compute_expr_width(inner),
            IrExpr::Cond(_, t, f) => {
                let tw = Self::compute_expr_width(t)?;
                let fw = Self::compute_expr_width(f)?;
                Some(tw.max(fw))
            }
            IrExpr::Concat(exprs) => {
                let total: usize = exprs.iter().filter_map(|e| Self::compute_expr_width(e)).sum();
                if total == 0 { Some(1) } else { Some(total) }
            }
            IrExpr::Cast { width, .. } => Some(*width),
            IrExpr::Signed(inner) => Self::compute_expr_width(inner),
            _ => None,
        }
    }

    pub fn run(&mut self) -> Result<(), SimError> {
        // ── VPI: Register engine for VPI callbacks ──
        crate::vpi::set_vpi_engine(self);
        crate::vpi::callback::dispatch_start_of_simulation();

        self.initialize_time_zero()?;
        self.execute_phases()?;

        // ── Register thread-local arena untuk zero-deallocation ──
        // Semua LogicVec::new(), fill(), from_u64() otomatis alokasi dari arena
        // selama event loop berjalan. Tidak perlu ubah evaluate_expr() call sites.
        crate::simulator::arena::set_thread_arena(Some(&mut self.sim_arena));

        // ── Build DAG untuk parallel process evaluation ──
        // Hanya untuk Combinational/CombReactive processes yang aman di-paralelkan.
        if self.use_dag_parallel && self.sim_dag.is_none() {
            let dag = crate::scheduler::SimulationDag::build(&self.design);
            if dag.num_processes() > 0 {
                let n_layers = dag.num_layers();
                let avg_par = dag.avg_parallelism();
                eprintln!(
                    "DAG: {} processes, {} layers, avg {:.1} parallelism",
                    dag.num_processes(),
                    n_layers,
                    avg_par
                );
                self.sim_dag = Some(dag);
            }
        }

        // ── Build clock domains untuk cycle-based simulation fusion ──
        if self.use_cycle_fusion && self.clock_analysis.is_none() {
            let analysis = crate::scheduler::ClockDomainAnalysis::analyze(&self.design);
            if analysis.num_domains() > 0 {
                eprintln!(
                    "Cycle fusion: {} clock domains, {} processes fused",
                    analysis.num_domains(),
                    analysis.num_fused_processes(),
                );
                self.clock_analysis = Some(analysis);
            }
        }

        // ── Start co-simulation server if configured ──
        if let Some(ref cosim_state) = self.cosim_state {
            if let Ok(mut cs) = cosim_state.lock() {
                cs.current_time = self.state.time;
            }
        }

        // ── Build process body cache for DAG parallel eval ──
        // Hindari clone bodies setiap cycle dengan cache sekali.
        if self.use_dag_parallel {
            self.process_body_cache.clear();
            for (pid, process) in self.design.top.processes.iter().enumerate() {
                if crate::scheduler::sim_dag::is_process_parallelizable(process) {
                    if let Some(body) = match process {
                        Process::Combinational { body, .. }
                        | Process::CombReactive { body, .. }
                        | Process::Initial { body, .. } => Some(body.clone()),
                        _ => None,
                    } {
                        self.process_body_cache.insert(pid, body);
                    }
                }
            }
        }

        while self.running && self.state.time <= self.max_time {
            let t = self.state.time as usize;

            // ── Timing wheel: advance to current time ──
            // Populates events[t] with all events scheduled for this time step
            // from the hierarchical timing wheel. Only uses wheel if enabled.
            if self.use_timing_wheel {
                if let Some(ref mut wheel) = self.timing_wheel {
                    let wheel_events = wheel.advance(t);
                    if !wheel_events.is_empty() {
                        self.ensure_events(t);
                        self.events[t].extend(wheel_events);
                    }
                }
            }

            // ── DPI: update thread-local scope path and simulation time ──
            #[cfg(feature = "dpi")]
            if let Some(ref path) = self.current_instance_path {
                crate::simulator::dpi::set_current_dpi_scope(path);
            }
            #[cfg(feature = "dpi")]
            crate::simulator::dpi::set_current_dpi_time(self.state.time);

            // ── Zero-deallocation: reset cycle arena (O(1) — bump pointer reset) ──
            self.sim_arena.reset_cycle();

            // ── Preponed region: initial snapshot for edge detection ──
            // Updated every delta cycle for correct edge detection (Sched-04 fix)
            let num_sigs = self.state.signals.len();
            let mut snapshot = Vec::with_capacity(num_sigs);
            for i in 0..num_sigs {
                snapshot.push(self.state.read_signal(i).clone());
            }
            self.signal_snapshot = Some(snapshot);

            self.dump_vcd_time()?;
            self.dump_fst_time()?;

            // ── IEEE 1800 stratified event loop ──
            let mut delta_count = 0u64;
            loop {
                let mut activity = false;
                let mut deltas: Vec<SignalId> = Vec::new();

                for &region in &IEEE_REGIONS[..] {
                    match region {
                        EventRegion::Preponed => {
                            // Already handled above — skip during re-circulation
                        }
                        EventRegion::PreActive
                        | EventRegion::PreNba
                        | EventRegion::PostNba
                        | EventRegion::PreObserved
                        | EventRegion::PostObserved
                        | EventRegion::PostReactive => {
                            // PLI regions: process any events in this region
                            self.ensure_events(t);
                            let mut matched = true;
                            while matched {
                                matched = false;
                                let mut to_process = Vec::new();
                                self.events[t].retain(|re| {
                                    if re.region == region {
                                        to_process.push(re.event.clone());
                                        false
                                    } else {
                                        true
                                    }
                                });
                                if !to_process.is_empty() {
                                    activity = true;
                                    matched = true;
                                    for event in to_process {
                                        self.process_event(event, t)?;
                                    }
                                }
                            }
                        }
                        EventRegion::Postponed => {
                            // Postponed region: process once per time step, does NOT re-circulate
                            self.ensure_events(t);
                            let mut to_process = Vec::new();
                                self.events[t].retain(|re| {
                                    if re.region == EventRegion::Postponed {
                                        to_process.push(re.event.clone());
                                        false
                                    } else {
                                        true
                                    }
                                });
                                if !to_process.is_empty() {
                                    for event in to_process {
                                        self.process_event(event, t)?;
                                    }
                                }
                        }
                        EventRegion::Observed => {
                            // Observed region: evaluate concurrent assertions (SVA).
                            // Process any assertion-evaluation events scheduled here.
                            self.ensure_events(t);
                                let mut matched = true;
                                while matched {
                                    matched = false;
                                    let mut to_process = Vec::new();
                                    self.events[t].retain(|re| {
                                        if re.region == EventRegion::Observed {
                                            to_process.push(re.event.clone());
                                            false
                                        } else {
                                            true
                                        }
                                    });
                                    if !to_process.is_empty() {
                                        activity = true;
                                        matched = true;
                                        for event in to_process {
                                            self.process_event(event, t)?;
                                        }
                                    }
                                }
                        }
                        EventRegion::Active | EventRegion::Inactive => {
                            self.ensure_events(t);
                            loop {
                                    let events: Vec<RegionEvent> = self.events[t]
                                        .drain(..)
                                        .filter(|re| re.region == region)
                                        .collect();
                                    if events.is_empty() {
                                        break;
                                    }
                                    activity = true;

                                    // ── DAG-Parallel: batch EvalProcess events ──
                                    // Hanya Combinational/CombReactive/Initial yang aman
                                    // di-paralelkan. Sequential, AlwaysWithDelay, dan Final
                                    // butuh event loop semantics (clock edges, delay).
                                    if self.use_dag_parallel && self.sim_dag.is_some() {
                                        let mut eval_pids: Vec<usize> = Vec::new();
                                        let mut other_events: Vec<RegionEvent> = Vec::new();
                                        for re in events {
                                            if let EventKind::EvalProcess(pid) = re.event {
                                                if pid < self.design.top.processes.len()
                                                    && {
                                                        // Safe access with bounds check
                                                        let process = self.design.top.processes.get(pid)
                                                            .expect("process pid bounds check failed in DAG loop");
                                                        crate::scheduler::is_process_parallelizable(process)
                                                    }
                                                {
                                                    eval_pids.push(pid);
                                                } else {
                                                    other_events.push(re);
                                                }
                                            } else {
                                                other_events.push(re);
                                            }
                                        }

                                        // Process non-EvalProcess events sequentially
                                        for re in other_events {
                                            self.process_event(re.event, t)?;
                                        }

                                        // Process EvalProcess events via DAG parallel
                                        if !eval_pids.is_empty() {
                                            self.evaluate_eval_processes_parallel(
                                                &eval_pids,
                                            )?;
                                        }
                                    } else {
                                        // Sequential: process all events one by one
                                        for re in events {
                                            self.process_event(re.event, t)?;
                                        }
                                    }

                                    // Inactive re-drains; Active drains once (outer loop
                                    // re-circulates if new events appear later)
                                    if region == EventRegion::Active {
                                        break;
                                    }
                                }
                        }
                        EventRegion::Nba => {
                            // NBA region: commit pending non-blocking assignments
                            self.commit_nba();
                            self.ensure_events(t);
                            let events: Vec<RegionEvent> = self.events[t]
                                .drain(..)
                                .filter(|re| re.region == EventRegion::Nba)
                                .collect();
                            if !events.is_empty() {
                                activity = true;
                                for re in events {
                                    self.process_event(re.event, t)?;
                                }
                            }
                        }
                        EventRegion::Reactive => {
                            // Commit changes and trigger sensitive processes
                            let changed = self.state.commit_changes();
                            if !changed.is_empty() {
                                activity = true;
                                for (id, _, _) in &changed {
                                    if !deltas.contains(id) {
                                        deltas.push(*id);
                                    }
                                }
                                self.trigger_sensitive_processes(&changed, t)?;
                            }
                            // Process Reactive events (from events[t] and reactive_events buffer)
                            self.ensure_events(t);
                            let events: Vec<RegionEvent> = self.events[t]
                                .drain(..)
                                .filter(|re| re.region == EventRegion::Reactive)
                                .collect();
                            if !events.is_empty() {
                                activity = true;
                                for re in events {
                                    self.process_event(re.event, t)?;
                                }
                            }
                            let buffered: Vec<EventKind> = self.reactive_events.drain(..).collect();
                            if !buffered.is_empty() {
                                activity = true;
                                for event in buffered {
                                    self.process_event(event, t)?;
                                }
                            }
                        }
                    }
                }

                if delta_count > self.delta_limit {
                    return Err(SimError::with_diag(
                        DiagCode::InfiniteDelta,
                        format!("simulation exceeded max delta cycles per time step ({})", self.delta_limit),
                    ));
                }
                let report_interval = if self.delta_limit >= 100_000 { 100_000 } else { self.delta_limit / 10 }.max(1);
                if delta_count > 0 && delta_count % report_interval == 0 {
                    eprintln!(
                        "warning: {} delta cycles at time {} (limit {})",
                        delta_count, self.state.time, self.delta_limit
                    );
                }
                delta_count += 1;
                self.current_delta = delta_count;

                // Check pending $wait conditions
                if !self.pending_waits.is_empty() && !deltas.is_empty() {
                    if self.process_pending_waits(&deltas)? {
                        activity = true;
                    }
                }

                // Check pending wait_order conditions
                if !self.pending_wait_orders.is_empty() && !deltas.is_empty() {
                    if self.process_pending_wait_orders(&deltas)? {
                        activity = true;
                    }
                }

                // Re-circulate if any events remain or NBA is pending
                // Postponed events do NOT re-circulate (they fire once per time step)
                self.ensure_events(t);
                let has_remaining = self.events[t].iter().any(|re| {
                        matches!(
                            re.region,
                            EventRegion::PreActive
                                | EventRegion::Active
                                | EventRegion::Inactive
                                | EventRegion::PreNba
                                | EventRegion::Nba
                                | EventRegion::PostNba
                                | EventRegion::PreObserved
                                | EventRegion::Observed
                                | EventRegion::PostObserved
                                | EventRegion::Reactive
                                | EventRegion::PostReactive
                        )
                    })
                    || !self.nba_pending.is_empty();

                if has_remaining {
                    activity = true;
                }

                if !activity {
                    break;
                }

                // Race detection: reset writer tracking setiap delta baru
                self.signal_writers.clear();

                // Sched-04: Refresh preponed snapshot every delta cycle for edge detection
                let num_sigs = self.state.signals.len();
                let mut snap = Vec::with_capacity(num_sigs);
                for i in 0..num_sigs {
                    snap.push(self.state.read_signal(i).clone());
                }
                self.signal_snapshot = Some(snap);
            }

            // ── UPF: Evaluate power states based on current supply net values ──
            if let Some(ref mut pi) = self.power_intent {
                if pi.enabled {
                    // Auto-bind supply net values from design signals with matching names
                    for (net_name, _) in &pi.supply_nets {
                        if let Some(sig_id) = self.design.top.signals.iter().position(|s| s.name.as_str() == *net_name) {
                            let val = self.state.read_signal(sig_id);
                            let is_high = val.to_bool().unwrap_or(false);
                            pi.supply_values.insert(net_name.clone(), is_high);
                        }
                    }
                    pi.evaluate_power_states();
                }
            }

            // ── Delta oscillation detection ──
            // Check if any signal toggled excessively within delta cycles
            for (sig_id, count) in self.signal_write_count.iter() {
                if *count > 10 {
                    if let Some(sig) = self.design.top.signals.get(*sig_id) {
                        self.emit_warning(
                            DiagCode::CombinationalLoop,
                            format!(
                                "possible combinational loop: signal '{}' toggled {} times in delta cycles",
                                sig.name, count
                            ),
                        );
                    }
                }
            }
            self.signal_write_count.clear();

            // ── Coverage tracking: toggle + FSM — record from committed changes ──
            self.record_coverage_after_commit();

            // ── Postponed region: $strobe, $monitor, VCD, timing checks ──
            // Postponed region events from events[t] are processed in the region loop above.
            // Standalone postponed operations execute here, once per time step.
            self.process_strobe()?;
            self.dump_vcd_state()?;
            self.dump_fst_state()?;
            self.dump_csv_state()?;
            self.record_signal_stats();
            self.check_monitor()?;
            self.check_timing_constraints()?;

            // ── VPI: Read-Write Synch callback after all signal updates ──
            crate::vpi::callback::dispatch_read_write_synch();

            // ── Debug check at start of cycle ──
            if self.debug_mode != DebugMode::Normal {
                self.debug_check()?;
                if self.paused {
                    break;
                }
                if self.step_mode == StepMode::StepCycle {
                    self.paused = true;
                    break;
                }
            }

            // ── Co-simulation: update shared state ──
            if let Some(ref cosim_state) = self.cosim_state {
                if let Ok(mut cs) = cosim_state.lock() {
                    cs.current_time = self.state.time;
                    // Copy output signal values (input signals of the external sim)
                    cs.outgoing_signals.clear();
                    for (sig_id, _, is_output) in &self.cosim_signals {
                        if *is_output && *sig_id < self.state.signals.len() {
                            let val = self.state.read_signal(*sig_id);
                            let bytes = val.to_u64().to_le_bytes().to_vec();
                            cs.outgoing_signals.push((*sig_id as u32, bytes));
                        }
                    }
                    // Poll incoming signals from external simulator
                    if cs.data_ready {
                        for (sig_id, val_bytes) in cs.incoming_signals.drain(..) {
                            if let Some(inner) = self.cosim_signals.iter().find(|(id, _, _)| *id as u32 == sig_id) {
                                let sid = inner.0;
                                if sid < self.state.signals.len() {
                                    let width = self.design.top.signals.get(sid).map(|s| s.width).unwrap_or(1);
                                    let val = u64::from_le_bytes(val_bytes[..8.min(val_bytes.len())].try_into().unwrap_or([0u8; 8]));
                                    let lv = LogicVec::from_u64(val, width);
                                    if *self.state.read_signal(sid) != lv {
                                        self.state.write_signal(sid, lv);
                                    }
                                }
                            }
                        }
                        cs.data_ready = false;
                    }
                }
            }

            // Advance and evaluate sequence attempts
            self.evaluate_sequence_attempts()?;
            self.state.time += 1;
            if self.state.time > self.max_time {
                break;
            }

            self.ensure_events(self.state.time as usize);
        }

        if !self.paused {
            self.execute_final_blocks()?;
            // ── VPI: End of simulation callback (setelah final blocks) ──
            crate::vpi::callback::dispatch_end_of_simulation();

            self.report_full_coverage();
            self.report_coverage();
            self.check_post_simulation_warnings();
        }

        // ── Cleanup: deregister thread-local arena untuk cegah dangling pointer ──
        crate::simulator::arena::set_thread_arena(None);

        // ── VPI Cleanup ──
        crate::vpi::handle::clear_cstring_cache();
        crate::vpi::handle::vpi_clear_all_objects();
        crate::vpi::callback::clear_all_callbacks();
        crate::vpi::systf::clear_all_systfs();
        crate::vpi::clear_vpi_engine();

        Ok(())
    }

    /// Check post-simulation warnings: uninitialized registers, unused signals,
    /// clock never toggles, reset permanently asserted.
    fn check_post_simulation_warnings(&self) {
        // Collect clock signal IDs from Sequential processes
        let clock_sigs: HashSet<SignalId> = self
            .design
            .top
            .processes
            .iter()
            .filter_map(|p| {
                if let Process::Sequential { clock, .. } = p {
                    match clock {
                        ClockEdge::PosEdge(id) | ClockEdge::NegEdge(id) => Some(*id),
                    }
                } else {
                    None
                }
            })
            .collect();

        // Collect reset signal IDs
        let reset_sigs: HashSet<SignalId> = self
            .design
            .top
            .processes
            .iter()
            .filter_map(|p| {
                if let Process::Sequential { reset: Some(r), .. } = p {
                    Some(r.signal)
                } else {
                    None
                }
            })
            .collect();

        for (sig_id, sig) in self.design.top.signals.iter().enumerate() {
            let sig_name = sig.name.as_str();
            let last_change = self.signal_last_change.get(&sig_id);
            let never_changed = last_change.is_none();

            // Skip input-only signals (driven by testbench, not by design)
            if sig.kind == SignalKind::Input || sig.kind == SignalKind::Inout {
                continue;
            }

            // ── WR0014: Uninitialized Register ──
            // Signal is all-X at init and was never written to
            if never_changed
                && (sig.kind == SignalKind::Reg || sig.kind == SignalKind::Logic)
                && sig.init_val.all_x()
            {
                self.emit_warning(
                    DiagCode::UninitializedRegister,
                    format!("uninitialized register '{}' was never assigned", sig_name),
                );
            }

            // ── WR0104: Unused Signal ──
            // Signal has a defined init value but never changed during simulation
            if never_changed && !sig.init_val.all_x() && !sig.init_val.all_z() {
                self.emit_warning(
                    DiagCode::UnusedSignal,
                    format!("unused signal '{}' never changed during simulation", sig_name),
                );
            }

            // ── WR0202: Clock Never Toggles ──
            // Signal is a clock but never toggled (only written at init time or never)
            // We check if the signal was ever written AFTER time 0 (init = time 0)
            if clock_sigs.contains(&sig_id) {
                let last_change_time = self.signal_last_change.get(&sig_id).copied();
                let has_real_change = last_change_time.map_or(false, |t| t > 0);
                if !has_real_change {
                    self.emit_warning(
                        DiagCode::ClockNeverToggles,
                        format!("clock signal '{}' never toggled during simulation", sig_name),
                    );
                }
            }

            // ── WR0203: Reset Permanently Asserted ──
            // Reset signal was never de-asserted (only written at init time or never)
            if reset_sigs.contains(&sig_id) {
                let last_change_time = self.signal_last_change.get(&sig_id).copied();
                let has_real_change = last_change_time.map_or(false, |t| t > 0);
                if !has_real_change {
                    let val = self.state.read_signal(sig_id);
                    if !val.all_x() && !val.all_z() {
                        self.emit_warning(
                            DiagCode::ResetPermanentlyAsserted,
                            format!(
                                "reset signal '{}' was permanently asserted during simulation",
                                sig_name
                            ),
                        );
                    }
                }
            }
        }
    }

    /// Evaluate EvalProcess events in parallel using DAG layers.
    ///
    /// Processes in the same DAG layer are independent (no signal conflicts)
    /// and are evaluated via rayon work-stealing. Writes are collected and
    /// applied after all processes in the layer complete.
    ///
    /// # Lock-Free Design
    ///
    /// Setiap process bekerja pada snapshot sinyal sendiri (clone).
    /// Tidak ada shared mutable state antar process dalam satu layer.
    fn evaluate_eval_processes_parallel(
        &mut self,
        pids: &[usize],
    ) -> Result<(), SimError> {
        // Gunakan cached process bodies + DAG layers + signal snapshot
        // untuk menghindari clone bodies setiap cycle.
        // process_body_cache dibangun sekali di run() — tidak ada clone per cycle.
        let dag_layers: Vec<Vec<usize>> = match &self.sim_dag {
            Some(dag) => dag.layers().to_vec(),
            None => {
                for &pid in pids {
                    self.process_event(EventKind::EvalProcess(pid), self.current_time as usize)?;
                }
                return Ok(());
            }
        };
        let signal_snapshot = self.state.signals.clone();

        // Evaluate each layer sequentially (processes WITHIN a layer are parallel)
        // Pass process_body_cache langsung — zero clone per cycle
        let body_cache = &self.process_body_cache;
        for layer in &dag_layers {
            let layer_pids: Vec<&usize> = layer.iter().filter(|pid| pids.contains(pid)).collect();
            if layer_pids.is_empty() {
                continue;
            }

            // Evaluate all processes in this layer in parallel via rayon
            // Each worker gets its own signal clone + body reference
            // body_cache langsung digunakan — tidak ada clone body per cycle
            let writes = crate::scheduler::sim_dag::evaluate_bodies_parallel(
                &layer_pids,
                body_cache,
                &signal_snapshot,
            )?;

            // Apply writes back to state (no borrow conflicts, all data cloned)
            for (sig_id, val) in writes {
                if sig_id < self.state.signals.len() {
                    self.state.write_signal(sig_id, val);
                }
            }
        }

        Ok(())
    }

    pub(crate) fn evaluate_clock_domain(&mut self, domain: &ClockDomain) -> Result<(), SimError> {
        // Clone process bodies upfront untuk hindari borrow conflicts
        let num_sigs = self.state.signals.len();

        // Collect sequential process bodies
        let seq_bodies: Vec<(usize, Vec<IrStmt>)> = domain
            .sequential_processes
            .iter()
            .filter_map(|&pid| {
                if pid < self.design.top.processes.len() {
                    if let Process::Sequential { body, .. } = self.design.top.processes.get(pid)
                        .expect("process pid out of bounds in clock domain eval")
                    {
                        return Some((pid, body.clone()));
                    }
                }
                None
            })
            .collect();

        // Collect follower combinational process bodies
        let follower_bodies: Vec<(usize, Vec<IrStmt>)> = domain
            .follower_processes
            .iter()
            .filter_map(|&pid| {
                if pid < self.design.top.processes.len() {
                    match self.design.top.processes.get(pid)
                        .expect("process pid out of bounds in follower bodies")
                    {
                        Process::Combinational { body, .. }
                        | Process::CombReactive { body, .. } => {
                            Some((pid, body.clone()))
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            })
            .collect();

        // Fixed-point iteration: evaluate sequential + follower until stable
        let max_iter = 10; // Safety limit
        for _iter in 0..max_iter {
            let prev_snapshot: Vec<LogicVec> = (0..num_sigs)
                .map(|i| self.state.read_signal(i).clone())
                .collect();

            // Step 1: Evaluate all sequential processes in the domain
            for (_, body) in &seq_bodies {
                self.evaluate_stmt_block(body)?;
            }

            // Commit NBA from sequential process evaluations
            self.commit_nba();

            // Step 2: Evaluate all follower combinational processes
            for (_, body) in &follower_bodies {
                self.evaluate_stmt_block(body)?;
            }

            // Check if stable: no signal changes
            let mut changed = false;
            for i in 0..num_sigs.min(prev_snapshot.len()) {
                let cur = self.state.read_signal(i);
                if cur != &prev_snapshot[i] {
                    changed = true;
                    break;
                }
            }

            if !changed {
                break; // Converged
            }
        }

        Ok(())
    }

    fn initialize_time_zero(&mut self) -> Result<(), SimError> {
        let t = 0usize;
        let processes = self.design.top.processes.clone();

        // IEEE 1800: initial blocks execute FIRST, then always_comb evaluates.
        // Schedule initial blocks and always-with-delay first,
        // then combinational/reactive processes AFTER.
        // All in Active region, processed in FIFO order by the event loop.

        // Pass 1: Initial blocks (execute first at time 0)
        for (pid, process) in processes.iter().enumerate() {
            if matches!(process, Process::Initial { .. }) {
                self.push_event(t, RegionEvent {
                    region: EventRegion::Active,
                    event: EventKind::EvalProcess(pid),
                });
            }
        }

        // Pass 2: Combinational/Reactive processes (evaluate after initial)
        for (pid, process) in processes.iter().enumerate() {
            if matches!(
                process,
                Process::Combinational { .. } | Process::CombReactive { .. }
            ) {
                self.push_event(t, RegionEvent {
                    region: EventRegion::Active,
                    event: EventKind::EvalProcess(pid),
                });
            }
        }

        // Pass 3: AlwaysWithDelay (time-0 processes that schedule future events)
        for (pid, process) in processes.iter().enumerate() {
            if matches!(process, Process::AlwaysWithDelay { .. }) {
                self.push_event(t, RegionEvent {
                    region: EventRegion::Active,
                    event: EventKind::EvalProcess(pid),
                });
            }
        }

        // Sequential processes wait for edge events, not scheduled at time 0
        // Final processes execute only at $finish

        // Initialize coverage tracking
        // Initialize coverage tracking
        for cg in &self.design.covergroups {
            for cp in &cg.coverpoints {
                let key = format!("{}.{}", cg.name, cp.name);
                self.cover_total.insert(Symbol::intern(&key), 0);
                self.cover_hits.insert(Symbol::intern(&key), 0);
                self.cover_bins.insert(Symbol::intern(&key), HashMap::new());
            }
            for cross in &cg.crosses {
                let key = format!("{}.{}", cg.name, cross.name);
                self.cover_total.insert(Symbol::intern(&key), 0);
                self.cover_hits.insert(Symbol::intern(&key), 0);
                self.cover_bins.insert(Symbol::intern(&key), HashMap::new());
            }
        }
        Ok(())
    }

    pub fn annotate_sdf(&mut self, sdf: &SdfData) -> Result<(), SimError> {
        let mode = crate::simulator::sdf::get_timing_mode();

        // Apply cell delays to signals
        for (cell_name, cell_delay) in &sdf.cell_delays {
            // Find matching signals by cell name (partial match via hierarchy)
            // Take the first IOPATH delay as the primary cell delay
            if let Some(path_delay) = cell_delay.io_paths.values().next() {
                let rise_ns = path_delay.rise.get(mode);
                let fall_ns = path_delay.fall.get(mode);

                // Try to find signals matching this cell instance name
                for sig in &mut self.design.top.signals {
                    let sig_name = sig.name.as_str();
                    if sig_name.starts_with(cell_name) || cell_name.starts_with(sig_name) {
                        sig.delay_rise = Some((rise_ns * 1000.0) as u64); // convert ns to ps
                        sig.delay_fall = Some((fall_ns * 1000.0) as u64);
                    }
                }

                // Also store delays in the signal delay map for per-path lookup
                for (path_key, path_delay) in &cell_delay.io_paths {
                    let rise_ns = path_delay.rise.get(mode);
                    let fall_ns = path_delay.fall.get(mode);
                    let delay_key = format!("{}:{}", cell_name, path_key);
                    self.signal_delays.insert(
                        delay_key,
                        crate::simulator::state::SignalDelay {
                            rise: (rise_ns * 1000.0) as u64,
                            fall: (fall_ns * 1000.0) as u64,
                        },
                    );
                }
            }
        }

        // Apply net delays to signals
        for (net_name, net_delay) in &sdf.net_delays {
            let rise_ns = net_delay.rise.get(mode);
            let fall_ns = net_delay.fall.get(mode);
            for sig in &mut self.design.top.signals {
                if sig.name == *net_name || sig.name.ends_with(&format!(".{}", net_name)) {
                    sig.delay_rise = Some((rise_ns * 1000.0) as u64);
                    sig.delay_fall = Some((fall_ns * 1000.0) as u64);
                }
            }
        }

        // Store timing checks for later use
        self.sdf_timing_checks = sdf.timing_checks.clone();

        // Print summary of annotation
        let cell_count = sdf.cell_delays.len();
        let net_count = sdf.net_delays.len();
        let check_count = sdf.timing_checks.len();
        eprintln!(
            "SDF annotation: {} cells, {} nets, {} timing checks (mode={})",
            cell_count,
            net_count,
            check_count,
            mode.as_str()
        );

        Ok(())
    }

    fn execute_final_blocks(&mut self) -> Result<(), SimError> {
        let bodies: Vec<Vec<IrStmt>> = self
            .design
            .top
            .processes
            .iter()
            .filter_map(|p| {
                if let Process::Final { body, .. } = p {
                    Some(body.clone())
                } else {
                    None
                }
            })
            .collect();
        for body in &bodies {
            self.evaluate_stmt_block(body)?;
        }
        Ok(())
    }

}
