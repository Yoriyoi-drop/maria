use super::super::SimulationEngine;
use maria_core::error::SimError;
use maria_core::diagnostics::DiagCode;
use maria_ir::*;
use crate::simulator::types::*;
use crate::simulator::parallel;
use std::sync::Arc;

/// Apakah sensitivity process terpenuhi oleh perubahan signal `changed`.
/// Entry range (msb/lsb Some) hanya terpicu bila SLICE tsb berubah; entry
/// whole (None) terpicu pada perubahan apa pun.
fn sensitivity_triggered(
    sensitivity: &[SignalSensitivity],
    changed: &[(usize, LogicVec, LogicVec)],
) -> bool {
    changed.iter().any(|(id, old, new)| {
        sensitivity.iter().any(|s| {
            if s.sig_id != *id {
                return false;
            }
            match (s.msb, s.lsb) {
                (Some(m), Some(l)) => {
                    let (lo, hi) = (l.min(m), m.max(l));
                    let a: &[LogicVal] = old.bits.get(lo..=hi).unwrap_or(&[]);
                    let b: &[LogicVal] = new.bits.get(lo..=hi).unwrap_or(&[]);
                    a != b
                }
                _ => true,
            }
        })
    })
}

impl SimulationEngine {
    pub(crate) fn process_event(&mut self, event: EventKind, t: usize) -> Result<(), SimError> {
        self.current_time = t as u64;
        match event {
            EventKind::EvalProcess(pid) => {
                if pid >= self.design.top.processes.len() {
                    return Ok(());
                }
                // SIM-25: catat evaluasi process untuk performance dashboard
                self.sim_perf.counters.processes_evaluated += 1;
                let process = self.design.top.processes[pid].clone();

                // Set runtime context: process name + instance path
                let pname = match &process {
                    Process::Combinational { name, .. }
                    | Process::CombReactive { name, .. }
                    | Process::Sequential { name, .. }
                    | Process::Initial { name, .. }
                    | Process::Final { name, .. }
                    | Process::AlwaysWithDelay { name, .. } => name.as_str(),
                };
                self.current_process_name = Some(pname.to_string());
                self.current_instance_path = Some(self.design.top.name.to_string());

                let kind = match &process {
                    Process::Combinational { .. } => "always_comb",
                    Process::CombReactive { .. } => "always_comb_reactive",
                    Process::Sequential { .. } => "always_ff",
                    Process::Initial { .. } => "initial",
                    Process::Final { .. } => "final",
                    Process::AlwaysWithDelay { .. } => "always",
                };
                crate::dbg_sim!(
                    1,
                    "t={} delta={} eval pid={} kind={} '{}'",
                    t,
                    self.current_delta,
                    pid,
                    kind,
                    pname
                );

                match &process {
                    Process::Initial { body, .. } => {
                        if self.state.time == 0 {
                            self.disable_pending = None;
                            self.evaluate_block_with_delay(body)?;
                        }
                    }
                    Process::AlwaysWithDelay { delay, body, .. } => {
                        self.ensure_events(t);
                        self.disable_pending = None;
                        // IEEE 1800: `always #N stmt;` menunda N unit SEBELUM
                        // eksekusi pertama — body TIDAK dijalankan di t=0
                        // (hanya menjadwalkan evaluasi pertama di t=N). Tanpa
                        // ini `always #5 clk = ~clk;` meng-toggle clk di t=0
                        // (fase bergeser → hitungan salah). Evaluasi berikutnya
                        // (t>0) menjalankan body lalu menjadwalkan t+N.
                        if t > 0 {
                            self.evaluate_block_with_delay(body)?;
                        }
                        let next_t = t + *delay as usize;
                        self.ensure_events(next_t);
                        self.push_event(next_t, RegionEvent {
                            region: EventRegion::Active,
                            event: EventKind::EvalProcess(pid),
                        });
                    }
                    Process::Combinational { body, .. } => {
                        // Try MIR JIT for compiled-code execution path
                        // If use_mir_jit is false, always fall back to interpreted
                        if !self.use_mir_jit || !self.try_evaluate_mir_jit(pid, body)? {
                            self.evaluate_stmt_block(body)?;
                        }
                    }
                    Process::CombReactive { body, .. } => {
                        // Try MIR JIT for compiled-code execution path
                        // If use_mir_jit is false, always fall back to interpreted
                        if !self.use_mir_jit || !self.try_evaluate_mir_jit(pid, body)? {
                            self.evaluate_stmt_block(body)?;
                        }
                    }
                    Process::Sequential { body, .. } => {
                        // Try MIR JIT for always_ff blocks (edge-triggered)
                        // JIT handles the combinational body; scheduler handles edge wakeup
                        if !self.use_mir_jit || !self.try_evaluate_mir_jit(pid, body)? {
                            self.evaluate_stmt_block(body)?;
                        }
                    }
                    _ => {}
                }
            }
            EventKind::ContinueBlock(cont) => {
                self.ensure_events(t);
                // LANG-30: branch fork yang di-disable via `disable fork` —
                // skip eksekusi body, langsung decrement (branch mati).
                if let Some(fid) = cont.fork_id {
                    if fid < self.fork_groups.len() && self.fork_groups[fid].disabled {
                        self.fork_decrement(fid)?;
                        return Ok(());
                    }
                }
                // LANG-29: restore nama proses saat suspend — ContinueBlock
                // diproses di luar EvalProcess sehingga current_process_name
                // bisa menunjuk proses lain; `wait fork` (dan fitur berbasis
                // nama proses lain) butuh konteks yang benar setelah resume.
                if let Some(pn) = &cont.process_name {
                    self.current_process_name = Some(pn.clone());
                }
                let all_consumed =
                    self.evaluate_block_with_delay_fork(&cont.stmts_to_exec, cont.fork_id)?;
                // Detect natural process completion: when a continuation runs to completion (all_consumed)
                // and has a stored process_id, mark that process as Finished and trigger await continuations
                if all_consumed {
                    if let Some(pid) = cont.process_id {
                        if let Some(pi) = self.process_map.get_mut(&pid) {
                            if pi.status == ProcessStatus::Running {
                                pi.status = ProcessStatus::Finished;
                                let conts = std::mem::take(&mut pi.await_continuations);
                                for c in conts {
                                    self.evaluate_block_with_delay(&c)?;
                                }
                            }
                        }
                    }
                }
                if let Some(fid) = cont.fork_id {
                    if all_consumed {
                        self.fork_decrement(fid)?;
                    }
                }
            }
            EventKind::ContinueAstBlock(stmts, fork_id, this_opt, method_opt) => {
                // LANG-30: branch AST fork yang di-disable — skip eksekusi.
                if let Some(fid) = fork_id {
                    if fid < self.fork_groups.len() && self.fork_groups[fid].disabled {
                        self.fork_decrement(fid)?;
                        return Ok(());
                    }
                }
                // F18: kontinuasi task/method UVM setelah delay — restore konteks
                // `this` + method yang disimpan saat suspend (sama seperti pola
                // PendingAstEventControl). Tanpa ini, body task yang dijalankan
                // via execute_phases/run_test (`run_phase` dkk) kehilangan
                // current_this → `this.field` error "used outside of class method".
                self.ensure_events(t);
                let old_this = self.current_this;
                let old_method = self.current_method;
                self.current_this = this_opt;
                self.current_method = method_opt;
                if std::env::var("DBG_UVM").is_ok() {
                    eprintln!("[DBG-F26] resume ContinueAstBlock fid={:?} nstmts={}", fork_id, stmts.len());
                }
                let all_consumed = self.evaluate_ast_block_with_delay_fork(&stmts, fork_id)?;
                // F35 review: return di branch fork (illegal SV tapi parseable)
                // menandai ast_return_pending — clear di sini agar tidak bocor
                // ke evaluasi blok lain.
                self.ast_return_pending = false;
                if std::env::var("DBG_UVM").is_ok() {
                    eprintln!("[DBG-F26] resume done fid={:?} consumed={}", fork_id, all_consumed);
                }
                // F21: fork_decrement SEBELUM restore konteks — bila branch ini
                // adalah branch TERAKHIR yang selesai, fork_finish mengeksekusi
                // continuation AST setelah join/join_any, yang masih milik
                // method ini (butuh current_this/current_method yang sama).
                // Sebelumnya restore terjadi duluan → cont AST dieksekusi tanpa
                // konteks → field class gagal resolve (warning RT0001 + 0).
                if let Some(fid) = fork_id {
                    if all_consumed {
                        self.fork_decrement(fid)?;
                    }
                }
                if all_consumed {
                    self.current_this = old_this;
                    self.current_method = old_method;
                }
                // Re-suspend: pertahankan konteks — ContinueAstBlock berikutnya
                // sudah menyimpan this/method baru di titik suspend-nya.
            }
            // WAV-13: commit tertunda dari write signal ber-annotasi SDF delay
            // (dijadwalkan `write_lvalue` saat delay_rise/delay_fall > 0).
            // Commit memakai helper yang sama dengan jalur write langsung
            // (multi-driver resolution + record_signal_change) sehingga proses
            // sensitive ikut terpicu pada waktu yang benar (t+delay).
            EventKind::SdfDelayedWrite { sig_id, value } => {
                if sig_id < self.design.top.signals.len() {
                    self.commit_delayed_signal_write(sig_id, value)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn process_pending_waits(&mut self, deltas: &[SignalId]) -> Result<bool, SimError> {
        let mut matched = false;
        let mut remaining = Vec::new();
        let waits = std::mem::take(&mut self.pending_waits);
        for (deps, stmts) in waits {
            if deltas.iter().any(|d| deps.contains(d)) {
                matched = true;
                self.evaluate_block_with_delay(&stmts)?;
            } else {
                remaining.push((deps, stmts));
            }
        }
        let newly_pushed = std::mem::take(&mut self.pending_waits);
        remaining.extend(newly_pushed);
        self.pending_waits = remaining;
        Ok(matched)
    }

    /// Nilai sinyal di AWAL delta berjalan — baseline "sebelum perubahan".
    /// `signal_snapshot` di-refresh tiap delta pass (Sched-04), jadi saat
    /// `process_pending_events` dipanggil nilainya = state sebelum write pada
    /// delta ini. Dipakai untuk deteksi level & edge blocking event control.
    fn snapshot_value(&self, id: SignalId) -> LogicVec {
        self.signal_snapshot
            .as_ref()
            .and_then(|s| s.get(id).cloned())
            .unwrap_or_else(|| LogicVec::new(1))
    }

    /// Resume blocking event control `@(sig)` saat signal berubah.
    /// - Level (edge None): nilai BERUBAH pada delta ini membangunkan.
    /// - Edge: hanya edge yang sesuai (via snapshot awal-delta) yang membangunkan.
    ///
    /// Baseline = nilai awal delta (`signal_snapshot`), BUKAN nilai saat arm:
    /// - `@(posedge clk)` yang di-arm saat clk sudah 1 tetap menangkap posedge
    ///   berikutnya (0→1) — nilai saat arm (1) tidak membatalkan deteksi edge.
    /// - `@(ev)` yang di-arm setelah `-> ev` pada delta yang sama tetap fire
    ///   karena snapshot awal-delta (x) != nilai saat ini (1).
    pub(crate) fn process_pending_events(&mut self, deltas: &[SignalId]) -> Result<bool, SimError> {
        let mut matched = false;
        let mut remaining = Vec::new();
        let pending = std::mem::take(&mut self.pending_events);
        for pe in pending {
            let fire = pe.sigs.iter().any(|(sid, edge)| {
                match edge {
                    None => {
                        // Level: fire hanya jika nilai BERUBAH dalam delta ini
                        // (snapshot awal != sekarang). Cegah re-fire untuk write
                        // nilai sama; tetap memenuhi `@(ev)` pada delta yang sama.
                        if !deltas.contains(sid) {
                            return false;
                        }
                        self.snapshot_value(*sid) != *self.state.read_signal(*sid)
                    }
                    Some(ClockEdge::PosEdge(id)) => {
                        if !deltas.contains(id) {
                            return false;
                        }
                        let new = self.state.read_signal(*id);
                        self.snapshot_value(*id).to_bool() != Some(true)
                            && new.to_bool() == Some(true)
                    }
                    Some(ClockEdge::NegEdge(id)) => {
                        if !deltas.contains(id) {
                            return false;
                        }
                        let new = self.state.read_signal(*id);
                        self.snapshot_value(*id).to_bool() != Some(false)
                            && new.to_bool() == Some(false)
                    }
                    // F27: clock/edge hierarkis (`@(posedge b.clk)` via port
                    // interface) — resolve Symbol path via hier_signal_map.
                    Some(ClockEdge::PosEdgeHier(s)) => {
                        let id = match self.design.hier_signal_map.get(s) {
                            Some(&sid) => sid,
                            None => return false,
                        };
                        if !deltas.contains(&id) {
                            return false;
                        }
                        let new = self.state.read_signal(id);
                        self.snapshot_value(id).to_bool() != Some(true)
                            && new.to_bool() == Some(true)
                    }
                    Some(ClockEdge::NegEdgeHier(s)) => {
                        let id = match self.design.hier_signal_map.get(s) {
                            Some(&sid) => sid,
                            None => return false,
                        };
                        if !deltas.contains(&id) {
                            return false;
                        }
                        let new = self.state.read_signal(id);
                        self.snapshot_value(id).to_bool() != Some(false)
                            && new.to_bool() == Some(false)
                    }
                }
            });
            if fire {
                // LANG-27: guard `iff (cond)` — lanjutkan continuation hanya
                // bila kondisi guard benar saat event terpenuhi. Jika kondisi
                // salah, tunggu event berikutnya (entry tetap hidup).
                let guard_ok = match &pe.iff {
                    Some(cond) => {
                        match self.evaluate_expr(cond) {
                            Ok(v) => v.to_bool().unwrap_or(false),
                            Err(_) => false,
                        }
                    }
                    None => true,
                };
                if guard_ok {
                    matched = true;
                    self.evaluate_block_with_delay_fork(&pe.continuation, None)?;
                    continue;
                }
            }
            remaining.push(pe);
        }
        // Resume tadi bisa mendaftarkan pending event baru (mis. `forever @(sig)`
        // yang re-suspend). Jangan timpa — gabungkan agar event baru tetap hidup
        // dan membangunkan pada perubahan sinyal berikutnya.
        let newly_pushed = std::mem::take(&mut self.pending_events);
        remaining.extend(newly_pushed);
        self.pending_events = remaining;
        Ok(matched)
    }

    /// Resume blocking event control `@(sig)` di jalur AST (task/method UVM),
    /// dengan restore konteks method (this/locals/method).
    pub(crate) fn process_pending_ast_events(
        &mut self,
        deltas: &[SignalId],
    ) -> Result<bool, SimError> {
        let mut matched = false;
        let mut remaining = Vec::new();
        let pending = std::mem::take(&mut self.pending_ast_events);
        for pe in pending {
            let fire = pe.sigs.iter().any(|(sid, edge)| {
                match edge {
                    None => {
                        if !deltas.contains(sid) {
                            return false;
                        }
                        self.snapshot_value(*sid) != *self.state.read_signal(*sid)
                    }
                    Some(ClockEdge::PosEdge(id)) => {
                        if !deltas.contains(id) {
                            return false;
                        }
                        let new = self.state.read_signal(*id);
                        self.snapshot_value(*id).to_bool() != Some(true)
                            && new.to_bool() == Some(true)
                    }
                    Some(ClockEdge::NegEdge(id)) => {
                        if !deltas.contains(id) {
                            return false;
                        }
                        let new = self.state.read_signal(*id);
                        self.snapshot_value(*id).to_bool() != Some(false)
                            && new.to_bool() == Some(false)
                    }
                    // F27: clock/edge hierarkis (`@(posedge b.clk)` via port
                    // interface) — resolve Symbol path via hier_signal_map.
                    Some(ClockEdge::PosEdgeHier(s)) => {
                        let id = match self.design.hier_signal_map.get(s) {
                            Some(&sid) => sid,
                            None => return false,
                        };
                        if !deltas.contains(&id) {
                            return false;
                        }
                        let new = self.state.read_signal(id);
                        self.snapshot_value(id).to_bool() != Some(true)
                            && new.to_bool() == Some(true)
                    }
                    Some(ClockEdge::NegEdgeHier(s)) => {
                        let id = match self.design.hier_signal_map.get(s) {
                            Some(&sid) => sid,
                            None => return false,
                        };
                        if !deltas.contains(&id) {
                            return false;
                        }
                        let new = self.state.read_signal(id);
                        self.snapshot_value(id).to_bool() != Some(false)
                            && new.to_bool() == Some(false)
                    }
                }
            });
            if !fire {
                remaining.push(pe);
                continue;
            }
            // LANG-27: guard `iff (cond)` di jalur AST — lanjutkan hanya bila
            // kondisi benar. Jika salah, tunggu event berikutnya.
            let guard_ok = match &pe.iff {
                Some(cond) => {
                    match self.evaluate_ast_expr(cond) {
                        Ok(v) => v.to_bool().unwrap_or(false),
                        Err(_) => false,
                    }
                }
                None => true,
            };
            if !guard_ok {
                remaining.push(pe);
                continue;
            }
            // Restore konteks method task sebelum resume continuation.
            let old_this = self.current_this;
            let old_method = self.current_method;
            let _old_locals = std::mem::replace(&mut self.method_locals, pe.locals.clone());
            self.current_this = pe.this;
            self.current_method = pe.method;
            matched = true;
            let completed = self.evaluate_ast_block_with_delay_fork(&pe.continuation, None)?;
            // F35 review: return di continuation (illegal SV) menandai
            // ast_return_pending — clear di sini agar tidak bocor.
            self.ast_return_pending = false;
            if completed {
                // Task selesai — truncate frame locals task; kembalikan konteks.
                let keep = pe.base_len.saturating_sub(1).min(self.method_locals.len());
                self.method_locals.truncate(keep);
                self.current_this = old_this;
                self.current_method = old_method;
            } else {
                // Task masih re-suspend — pertahankan locals task (old_locals dibuang).
            }
        }
        // Gabungkan pending AST event baru yang didaftarkan saat resume
        // (mis. `forever @(sig)` yang re-suspend), jangan timpa.
        let newly_pushed = std::mem::take(&mut self.pending_ast_events);
        remaining.extend(newly_pushed);
        self.pending_ast_events = remaining;
        Ok(matched)
    }

    pub(crate) fn process_pending_wait_orders(&mut self, deltas: &[SignalId]) -> Result<bool, SimError> {
        let mut any_done = false;
        let mut remaining = Vec::new();
        let orders = std::mem::take(&mut self.pending_wait_orders);
        'order: for mut order in orders {
            let mut changed_in_order = Vec::new();
            for d in deltas {
                if let Some(pos) = order.events.iter().position(|e| e == d) {
                    changed_in_order.push(pos);
                }
            }
            changed_in_order.sort();
            for &pos in &changed_in_order {
                if pos == order.expected_idx {
                    order.expected_idx += 1;
                    if order.expected_idx == order.events.len() {
                        if !order.continuation.is_empty() {
                            self.evaluate_block_with_delay(&order.continuation)?;
                        }
                        any_done = true;
                        continue 'order;
                    }
                } else if pos > order.expected_idx {
                    if !order.failure_stmts.is_empty() {
                        self.evaluate_stmt_block(&order.failure_stmts)?;
                    }
                    any_done = true;
                    continue 'order;
                }
            }
            remaining.push(order);
        }
        for item in remaining {
            self.pending_wait_orders.push(item);
        }
        Ok(any_done)
    }

    pub(crate) fn trigger_sensitive_processes(
        &mut self,
        changed: &[(usize, LogicVec, LogicVec)],
        _t: usize,
    ) -> Result<(), SimError> {
        let processes = self.design.top.processes.clone();
        // Collect triggered combinational processes for potential parallel execution
        // Skip fused processes — they're evaluated as part of clock domain fusion
        let mut comb_indices: Vec<usize> = Vec::new();
        for (pid, process) in processes.iter().enumerate() {
            if let Process::Combinational { sensitivity, .. } = process {
                // Skip if this process is fused into a clock domain
                if self.use_cycle_fusion
                    && self.clock_analysis.as_ref()
                        .map(|a| a.fused_processes.contains(&pid))
                        .unwrap_or(false)
                {
                    continue;
                }
                let should_trigger = sensitivity.is_empty()
                    || sensitivity_triggered(sensitivity, changed);
                if should_trigger {
                    comb_indices.push(pid);
                }
            }
        }

        // If enough processes to parallelize and config allows it, use parallel eval
        crate::dbg_sim!(
            2,
            "t={} delta={} trigger_sensitive: {} comb process(es), {} changed",
            self.current_time,
            self.current_delta,
            comb_indices.len(),
            changed.len()
        );
        if comb_indices.len() >= self.parallel_config.min_processes_parallel
            && self.parallel_config.parallel_processes
        {
            use rayon::prelude::*;
            let signal_count = self.state.signals.len();

            // ── SIM-28: snapshot SPARSE ──
            // Bangun cache akses per-process sekali (lazy). Bila ada process
            // dengan akses tak-resolve (HierRef dll) → mode FULL (snapshot
            // seluruh sinyal). Bila semua ter-resolve → mode SPARSE: base
            // hanya berisi UNION sinyal yang diakses process terpicu —
            // per-process setup O(0) (overlay kosong), bukan clone O(S).
            if !self.comb_access_ready {
                self.comb_access = self
                    .design
                    .top
                    .processes
                    .iter()
                    .map(crate::scheduler::sim_dag::analyze_process_access)
                    .collect();
                self.comb_access_ready = true;
            }
            let needs_full = comb_indices.iter().any(|&pid| {
                self.comb_access
                    .get(pid)
                    .map(|a| a.has_unresolved)
                    .unwrap_or(true)
            });

            let dbg_t = self.current_time;
            let dbg_delta = self.current_delta;
            if needs_full {
                // ── Mode FULL: snapshot seluruh sinyal (Arc, clone sekali) ──
                let snapshot: Vec<Arc<LogicVec>> = (0..signal_count)
                    .map(|i| Arc::new(self.state.read_signal(i).clone()))
                    .collect();
                let identity: Vec<Option<usize>> = (0..signal_count).map(Some).collect();
                crate::dbg_sim!(2, "  sparse=off (unresolved access) full-snapshot {} sig", signal_count);
                let results: Vec<Result<Vec<(SignalId, LogicVec)>, SimError>> = comb_indices
                    .par_iter()
                    .map(|&pid| {
                        let process = &processes[pid];
                        if let Process::Combinational { body, .. } = process {
                            crate::dbg_sim!(3, "t={} delta={} par-eval pid={}", dbg_t, dbg_delta, pid);
                            let mut overlay = std::collections::HashMap::new();
                            let mut view = parallel::SignalView::new(&snapshot, &identity, &mut overlay);
                            let mut writes = Vec::new();
                            match parallel::evaluate_stmt_block_parallel(
                                body,
                                &mut view,
                                &mut writes,
                                &self.design.top.signals,
                            ) {
                                Ok(()) => Ok(writes),
                                Err(e) => Err(SimError::with_diag(DiagCode::InternalError, format!("parallel eval error: {}", e))),
                            }
                        } else {
                            Ok(Vec::new())
                        }
                    })
                    .collect();
                for result in results {
                    let writes = result?;
                    for (sig_id, val) in writes {
                        self.state.write_signal(sig_id, val);
                    }
                }
            } else {
                // ── Mode SPARSE: base = union sinyal yang diakses ──
                let mut needed = vec![false; signal_count];
                for &pid in &comb_indices {
                    if let Some(a) = self.comb_access.get(pid) {
                        for &r in &a.reads {
                            if r < signal_count {
                                needed[r] = true;
                            }
                        }
                        for &w in &a.writes {
                            if w < signal_count {
                                needed[w] = true;
                            }
                        }
                    }
                }
                let mut id_map: Vec<Option<usize>> = vec![None; signal_count];
                let mut base: Vec<Arc<LogicVec>> = Vec::new();
                for i in 0..signal_count {
                    if needed[i] {
                        id_map[i] = Some(base.len());
                        base.push(Arc::new(self.state.read_signal(i).clone()));
                    }
                }
                crate::dbg_sim!(
                    2,
                    "  sparse: base {} sig dari {} (union akses), {} process",
                    base.len(),
                    signal_count,
                    comb_indices.len()
                );
                let results: Vec<Result<Vec<(SignalId, LogicVec)>, SimError>> = comb_indices
                    .par_iter()
                    .map(|&pid| {
                        let process = &processes[pid];
                        if let Process::Combinational { body, .. } = process {
                            crate::dbg_sim!(3, "t={} delta={} par-eval pid={}", dbg_t, dbg_delta, pid);
                            let mut overlay = std::collections::HashMap::new();
                            let mut view = parallel::SignalView::new(&base, &id_map, &mut overlay);
                            let mut writes = Vec::new();
                            match parallel::evaluate_stmt_block_parallel(
                                body,
                                &mut view,
                                &mut writes,
                                &self.design.top.signals,
                            ) {
                                Ok(()) => Ok(writes),
                                Err(e) => Err(SimError::with_diag(DiagCode::InternalError, format!("parallel eval error: {}", e))),
                            }
                        } else {
                            Ok(Vec::new())
                        }
                    })
                    .collect();
                for result in results {
                    let writes = result?;
                    for (sig_id, val) in writes {
                        self.state.write_signal(sig_id, val);
                    }
                }
            }
        } else {
            // Sequential path: evaluate triggered comb processes inline
            for &pid in &comb_indices {
                let process = &processes[pid];
                if let Process::Combinational { body, .. } = process {
                    crate::dbg_sim!(
                        3,
                        "t={} delta={} seq-eval pid={}",
                        self.current_time,
                        self.current_delta,
                        pid
                    );
                    self.evaluate_stmt_block(body)?;
                }
            }
        }

        // Handle CombReactive, Sequential, and other process types (always sequential)
        for (pid, process) in processes.iter().enumerate() {
            match process {
                Process::CombReactive { sensitivity, .. } => {
                    let should_trigger = sensitivity.is_empty()
                        || sensitivity_triggered(sensitivity, changed);
                    if should_trigger {
                        self.reactive_events.push(EventKind::EvalProcess(pid));
                    }
                }
                Process::Sequential {
                    clock,
                    reset,
                    body,
                    iff,
                    ..
                } => {
                    let clock_trigger = match clock {
                        // F27: *Hier = clock lewat port interface (`posedge
                        // b.clk`) — resolve Symbol path via hier_signal_map.
                        ClockEdge::PosEdge(_) | ClockEdge::PosEdgeHier(_) => {
                            let sid = self.clock_edge_signal(clock).unwrap_or(usize::MAX);
                            changed.iter().any(|(id, old, new)| {
                                *id == sid
                                    && old.to_bool() != Some(true)
                                    && new.to_bool() == Some(true)
                            })
                        }
                        ClockEdge::NegEdge(_) | ClockEdge::NegEdgeHier(_) => {
                            let sid = self.clock_edge_signal(clock).unwrap_or(usize::MAX);
                            changed.iter().any(|(id, old, new)| {
                                *id == sid
                                    && old.to_bool() != Some(false)
                                    && new.to_bool() == Some(false)
                            })
                        }
                    };
                    // LANG-27: guard `iff (cond)` — proses hanya dijalankan bila
                    // kondisi benar saat edge clock terjadi. Kondisi dievaluasi
                    // di nilai sesudah edge (delta ini). Guard HANYA untuk
                    // edge clock — reset async harus tetap memicu walau iff
                    // false (kalau tidak, reset terlewat → FF stuck X).
                    let trigger = clock_trigger
                        && match iff {
                            Some(cond) => {
                                match self.evaluate_expr(cond) {
                                    Ok(v) => v.to_bool().unwrap_or(false),
                                    Err(_) => false,
                                }
                            }
                            None => true,
                        };
                    // F40 fix: async reset edge (negedge rst_n / posedge rst)
                    // memicu proses LANGSUNG — dulu reset diabaikan di sini
                    // (`reset: _reset`) sehingga `always_ff @(posedge clk or
                    // negedge rst_n)` tidak pernah fire saat reset, dan FF
                    // tanpa init tetap X/z selamanya. Body always_ff berisi
                    // branch `if (!rst_n)` yang mengeksekusi reset value.
                    let trigger = trigger
                        || reset
                            .as_ref()
                            .filter(|r| r.r#async)
                            .map(|r| {
                                let sid = r.signal;
                                // polarity: true = aktif-high (posedge),
                                // false = aktif-low (negedge) — diisi
                                // elaborator dari event sensitivity.
                                let active_high = r.polarity;
                                changed.iter().any(|(id, old, new)| {
                                    *id == sid
                                        && if active_high {
                                            old.to_bool() != Some(true)
                                                && new.to_bool() == Some(true)
                                        } else {
                                            old.to_bool() != Some(false)
                                                && new.to_bool() == Some(false)
                                        }
                                })
                            })
                            .unwrap_or(false);
                    if trigger {
                        // ── Cycle-Based Fusion: jika process ini termasuk fused domain,
                        // evaluasi SEMUA process dalam domain sekaligus (sequential + follower comb).
                        // Skip event queue overhead untuk process sinkronus murni. ──
                        // Clone domain upfront untuk hindari borrow conflict
                        // antara self.clock_analysis (immutable) dan self.evaluate_clock_domain (&mut).
                        let fused_domain = if self.use_cycle_fusion {
                            self.clock_analysis.as_ref()
                                .and_then(|a| {
                                    if a.fused_processes.contains(&pid) {
                                        a.domains.iter().find(|d| d.sequential_processes.contains(&pid)).cloned()
                                    } else {
                                        None
                                    }
                                })
                        } else {
                            None
                        };
                        if let Some(domain) = fused_domain {
                            self.evaluate_clock_domain(&domain)?;
                            continue; // Skip individual eval
                        }
                        // Fallback: evaluate only this sequential process
                        self.evaluate_stmt_block(body)?;
                    }
                }
                // Skip fused combinational processes — they're evaluated
                // as part of their clock domain's follower set
                Process::Combinational { .. }
                    if self.use_cycle_fusion
                    && self.clock_analysis.as_ref()
                        .map(|a| a.fused_processes.contains(&pid))
                        .unwrap_or(false) => {}
                _ => {}
            }
        }
        Ok(())
    }

    pub(crate) fn commit_nba(&mut self) {
        let pending = std::mem::take(&mut self.nba_pending);
        for (lvalue, val) in pending {
            if !self.is_forced(&lvalue) {
                let _ = self.write_lvalue(&lvalue, val);
            }
        }
    }

    /// SIM-14: Push NBA write ke pending list dengan deteksi write conflict.
    /// Jika signal yang sama sudah punya NBA pending → warning RT1006.
    pub(crate) fn push_nba_pending(&mut self, lvalue: IrLValue, val: LogicVec) {
        if let Some(new_id) = self.signal_id_from_lvalue(&lvalue) {
            for (existing_lvalue, _) in &self.nba_pending {
                if let Some(existing_id) = self.signal_id_from_lvalue(existing_lvalue) {
                    if existing_id == new_id {
                        self.emit_warning(
                            maria_core::diagnostics::DiagCode::NbaWriteConflict,
                            format!(
                                "NBA write conflict on signal id={} at time {}: \
                                 multiple non-blocking assignments to the same signal in one delta cycle",
                                new_id, self.state.time
                            ),
                        );
                        break;
                    }
                }
            }
        }
        self.nba_pending.push((lvalue, val));
    }

    pub(crate) fn signal_id_from_lvalue(&self, lvalue: &IrLValue) -> Option<SignalId> {
        match lvalue {
            IrLValue::Signal(id, _) => Some(*id),
            IrLValue::RangeSelect(id, _, _) => Some(*id),
            IrLValue::BitSelect(id, _) => Some(*id),
            IrLValue::ArrayIndex { sig_id, .. } => Some(*sig_id),
            IrLValue::ArrayRangeSelect { sig_id, .. } => Some(*sig_id),
            IrLValue::ArrayBitSelect { sig_id, .. } => Some(*sig_id),
            IrLValue::ExprPartSelect { sig_id, .. } => Some(*sig_id),
            IrLValue::ObjectField { sig_id, .. } => Some(*sig_id),
            IrLValue::HierRef(name) | IrLValue::HierRefIndex { name, .. } => {
                self.find_signal(name.as_str())
            }
            IrLValue::Concat(_) => None,
        }
    }

    pub(crate) fn is_forced(&self, lvalue: &IrLValue) -> bool {
        self.signal_id_from_lvalue(lvalue)
            .is_some_and(|id| self.forced_signals.contains(&id))
    }

}
