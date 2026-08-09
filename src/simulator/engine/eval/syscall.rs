use super::super::SimulationEngine;
use crate::error::SimError;
use crate::ir::*;
use crate::ast::*;
use crate::simulator::util::*;
use rand::Rng;

impl SimulationEngine {
    pub(crate) fn check_timing_constraints(&mut self) -> Result<(), SimError> {
        let current_time = self.state.time;
        let signal_names: Vec<(String, SignalId)> = self
            .design
            .top
            .signals
            .iter()
            .enumerate()
            .map(|(i, s)| (s.name.to_string(), i))
            .collect();
        let items = self.design.specify_items.clone();
        for item in &items {
            match item {
                SpecifyItem::SetupCheck {
                    data,
                    ref_event,
                    ref_edge,
                    limit,
                } => {
                    // SIM-24: setup violation dicek saat ref edge terjadi (ref_chg ==
                    // current_time), arah edge harus cocok dengan ref_edge (posedge/
                    // negedge), lalu bandingkan kapan data terakhir berubah. Fix false
                    // positive: sebelumnya fire di KEDUA edge ref (negedge juga memicu
                    // posedge-check).
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let (Expr::Ident { name: data_sig, .. }, Expr::Ident { name: ref_sig, .. }) =
                        (data, ref_event)
                    {
                        if let (Some((_, dsid)), Some((_, rsid))) = (
                            signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()),
                            signal_names.iter().find(|(n, _)| n.as_str() == ref_sig.as_str()),
                        ) {
                            if let (Some(&data_chg), Some(&ref_chg)) = (
                                self.signal_last_change.get(dsid),
                                self.signal_last_change.get(rsid),
                            ) {
                                // Arah edge ref harus cocok dengan spesifikasi (kalau ada)
                                let dir_ok = match ref_edge {
                                    Some(e) => self.signal_last_dir.get(rsid) == Some(e),
                                    None => true,
                                };
                                if dir_ok
                                    && ref_chg == current_time
                                    && data_chg < ref_chg
                                    && ref_chg - data_chg <= limit_val
                                {
                                    self.emit_warning(
                                        crate::diagnostics::DiagCode::TimingViolation,
                                        format!("$setup violation: data '{}' changed {}ns before ref (limit={}ns)",
                                            data_sig, ref_chg - data_chg, limit_val),
                                    );
                                }
                            }
                        }
                    }
                }
                SpecifyItem::HoldCheck {
                    ref_event,
                    ref_edge,
                    data,
                    limit,
                } => {
                    // SIM-24: hold violation dicek saat data BERUBAH pada step ini,
                    // dan ref edge (arah sesuai ref_edge) terjadi dalam `limit` sebelumnya.
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let (Expr::Ident { name: ref_sig, .. }, Expr::Ident { name: data_sig, .. }) =
                        (ref_event, data)
                    {
                        if let (Some((_, rsid)), Some((_, dsid))) = (
                            signal_names.iter().find(|(n, _)| n.as_str() == ref_sig.as_str()),
                            signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()),
                        ) {
                            if let (Some(&ref_chg), Some(&data_chg)) = (
                                self.signal_last_change.get(rsid),
                                self.signal_last_change.get(dsid),
                            ) {
                                let dir_ok = match ref_edge {
                                    Some(e) => self.signal_last_dir.get(rsid) == Some(e),
                                    None => true,
                                };
                                if dir_ok
                                    && data_chg == current_time
                                    && ref_chg < data_chg
                                    && data_chg - ref_chg <= limit_val
                                {
                                    self.emit_warning(
                                        crate::diagnostics::DiagCode::TimingViolation,
                                        format!("$hold violation: data '{}' changed {}ns after ref (limit={}ns)",
                                            data_sig, data_chg - ref_chg, limit_val),
                                    );
                                }
                            }
                        }
                    }
                }
                SpecifyItem::SetupHoldCheck {
                    ref_event,
                    ref_edge,
                    data,
                    setup_limit,
                    hold_limit,
                } => {
                    let setup_val = const_eval_simple(setup_limit).unwrap_or(0) as u64;
                    let hold_val = const_eval_simple(hold_limit).unwrap_or(0) as u64;
                    if let (Expr::Ident { name: ref_sig, .. }, Expr::Ident { name: data_sig, .. }) =
                        (ref_event, data)
                    {
                        if let (Some((_, rsid)), Some((_, dsid))) = (
                            signal_names.iter().find(|(n, _)| n.as_str() == ref_sig.as_str()),
                            signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()),
                        ) {
                            if let (Some(&ref_chg), Some(&data_chg)) = (
                                self.signal_last_change.get(rsid),
                                self.signal_last_change.get(dsid),
                            ) {
                                let dir_ok = match ref_edge {
                                    Some(e) => self.signal_last_dir.get(rsid) == Some(e),
                                    None => true,
                                };
                                // Setup: ref edge step ini, data berubah dalam setup window sebelum edge
                                if dir_ok
                                    && ref_chg == current_time
                                    && data_chg < ref_chg
                                    && ref_chg - data_chg <= setup_val
                                {
                                    self.emit_warning(
                                        crate::diagnostics::DiagCode::TimingViolation,
                                        format!("$setuphold (setup) violation: data '{}' changed {}ns before ref (setup={}ns)",
                                            data_sig, ref_chg - data_chg, setup_val),
                                    );
                                }
                                // Hold: data berubah step ini, ref edge dalam hold window sebelumnya
                                if dir_ok
                                    && data_chg == current_time
                                    && ref_chg < data_chg
                                    && data_chg - ref_chg <= hold_val
                                {
                                    self.emit_warning(
                                        crate::diagnostics::DiagCode::TimingViolation,
                                        format!("$setuphold (hold) violation: data '{}' changed {}ns after ref (hold={}ns)",
                                            data_sig, data_chg - ref_chg, hold_val),
                                    );
                                }
                            }
                        }
                    }
                }
                SpecifyItem::RecoveryCheck {
                    data,
                    ref_event: _ref_event,
                    ref_edge: _,
                    limit,
                } => {
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let Expr::Ident { name: data_sig, .. } = data {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()) {
                            if let Some(&last_change) = self.signal_last_change.get(sid) {
                                let delta = current_time - last_change;
                                if delta > 0 && delta <= limit_val {
                                    let key = ("$recovery".to_string(), *sid);
                                    if self.timing_reported.get(&key) != Some(&last_change) {
                                        self.timing_reported.insert(key, last_change);
                                        self.emit_warning(
                                            crate::diagnostics::DiagCode::TimingViolation,
                                            format!("$recovery violation: signal '{}' changed {}ns before ref (limit={}ns)", data_sig, delta, limit_val),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                SpecifyItem::RemovalCheck {
                    ref_event: _ref_event,
                    ref_edge: _,
                    data,
                    limit,
                } => {
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let Expr::Ident { name: data_sig, .. } = data {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()) {
                            if let Some(&last_change) = self.signal_last_change.get(sid) {
                                let delta = current_time - last_change;
                                if delta > 0 && delta <= limit_val {
                                    let key = ("$removal".to_string(), *sid);
                                    if self.timing_reported.get(&key) != Some(&last_change) {
                                        self.timing_reported.insert(key, last_change);
                                        self.emit_warning(
                                            crate::diagnostics::DiagCode::TimingViolation,
                                            format!("$removal violation: signal '{}' changed {}ns before ref (limit={}ns)", data_sig, delta, limit_val),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                SpecifyItem::RecoveryRemovalCheck {
                    ref_event: _ref_event,
                    ref_edge: _,
                    data,
                    recovery_limit,
                    removal_limit,
                } => {
                    let recov_val = const_eval_simple(recovery_limit).unwrap_or(0) as u64;
                    let remov_val = const_eval_simple(removal_limit).unwrap_or(0) as u64;
                    if let Expr::Ident { name: data_sig, .. } = data {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()) {
                            if let Some(&last_change) = self.signal_last_change.get(sid) {
                                let delta = current_time - last_change;
                                if delta > 0 && delta <= recov_val {
                                    let key = ("$recrem-recov".to_string(), *sid);
                                    if self.timing_reported.get(&key) != Some(&last_change) {
                                        self.timing_reported.insert(key, last_change);
                                        self.emit_warning(
                                            crate::diagnostics::DiagCode::TimingViolation,
                                            format!("$recrem (recovery) violation: signal '{}' changed {}ns before ref (recov={}ns)", data_sig, delta, recov_val),
                                        );
                                    }
                                }
                                if delta > 0 && delta <= remov_val {
                                    let key = ("$recrem-remov".to_string(), *sid);
                                    if self.timing_reported.get(&key) != Some(&last_change) {
                                        self.timing_reported.insert(key, last_change);
                                        self.emit_warning(
                                            crate::diagnostics::DiagCode::TimingViolation,
                                            format!("$recrem (removal) violation: signal '{}' changed {}ns before ref (remov={}ns)", data_sig, delta, remov_val),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                SpecifyItem::PeriodCheck {
                    ref_event,
                    ref_edge,
                    limit,
                } => {
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let Expr::Ident { name: ref_sig, .. } = ref_event {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == ref_sig.as_str()) {
                            // Dedupe: hanya fire saat edge baru terjadi pada step ini
                            // (last_change == current_time) dan period antar-edge < limit.
                            if let (Some(&last_change), Some(&prev_change)) = (
                                self.signal_last_change.get(sid),
                                self.signal_prev_change.get(sid),
                            ) {
                                let dir_ok = match ref_edge {
                                    Some(e) => self.signal_last_dir.get(sid) == Some(e),
                                    None => true,
                                };
                                if dir_ok
                                    && last_change == current_time
                                    && prev_change < last_change
                                    && last_change - prev_change < limit_val
                                {
                                    let delta = last_change - prev_change;
                                    self.emit_warning(
                                        crate::diagnostics::DiagCode::TimingViolation,
                                        format!("$period violation: signal '{}' period {}ns < minimum {}ns", ref_sig, delta, limit_val),
                                    );
                                }
                            }
                        }
                    }
                }
                SpecifyItem::WidthCheck {
                    ref_event,
                    ref_edge,
                    limit,
                    threshold: _threshold,
                } => {
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let Expr::Ident { name: ref_sig, .. } = ref_event {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == ref_sig.as_str()) {
                            // Dedupe: fire sekali saat pulse berakhir (edge baru terjadi),
                            // lebar pulse = last_change - prev_change. Bila ref_edge
                            // dispesifikasikan, pulse berakhir pada edge KEBALIKAN arah
                            // (mis. $width(posedge clk) mengukur pulse high yang berakhir
                            // di negedge) — filter ini mencegah false positive pulse low.
                            let dir_ok = match ref_edge {
                                Some(crate::ast::types::EdgeKind::PosEdge) => {
                                    self.signal_last_dir.get(sid) == Some(&crate::ast::types::EdgeKind::NegEdge)
                                }
                                Some(crate::ast::types::EdgeKind::NegEdge) => {
                                    self.signal_last_dir.get(sid) == Some(&crate::ast::types::EdgeKind::PosEdge)
                                }
                                None => true,
                            };
                            if let (Some(&last_change), Some(&prev_change)) = (
                                self.signal_last_change.get(sid),
                                self.signal_prev_change.get(sid),
                            ) {
                                if dir_ok
                                    && last_change == current_time
                                    && prev_change < last_change
                                    && last_change - prev_change < limit_val
                                {
                                    let delta = last_change - prev_change;
                                    self.emit_warning(
                                        crate::diagnostics::DiagCode::TimingViolation,
                                        format!("$width violation: signal '{}' pulse width {}ns < minimum {}ns", ref_sig, delta, limit_val),
                                    );
                                }
                            }
                        }
                    }
                }
                SpecifyItem::SkewCheck {
                    ref_event,
                    ref_edge: _,
                    data,
                    limit,
                } => {
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let Expr::Ident { name: data_sig, .. } = data {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()) {
                            if let Some(&data_change) = self.signal_last_change.get(sid) {
                                if let Expr::Ident { name: ref_sig, .. } = &ref_event {
                                    if let Some((_, rsid)) =
                                        signal_names.iter().find(|(n, _)| n.as_str() == ref_sig.as_str())
                                    {
                                        if let Some(&ref_change) = self.signal_last_change.get(rsid)
                                        {
                                            let skew = data_change.abs_diff(ref_change);
                                            // Dedupe: hanya fire saat salah satu sinyal berubah
                                            if skew > limit_val
                                                && (data_change == current_time || ref_change == current_time)
                                            {
                                                let key = ("$skew".to_string(), *sid);
                                                if self.timing_reported.get(&key) != Some(&current_time) {
                                                    self.timing_reported.insert(key, current_time);
                                                    self.emit_warning(
                                                        crate::diagnostics::DiagCode::TimingViolation,
                                                        format!("$skew violation: skew {}ns > max {}ns between '{}' and '{}'", skew, limit_val, data_sig, ref_sig),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                SpecifyItem::TimeskewCheck {
                    ref_event,
                    ref_edge: _,
                    data,
                    limit,
                    threshold: _threshold,
                } => {
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let Expr::Ident { name: data_sig, .. } = data {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()) {
                            if let Some(&data_change) = self.signal_last_change.get(sid) {
                                if let Expr::Ident { name: ref_sig, .. } = &ref_event {
                                    if let Some((_, rsid)) =
                                        signal_names.iter().find(|(n, _)| n.as_str() == ref_sig.as_str())
                                    {
                                        if let Some(&ref_change) = self.signal_last_change.get(rsid)
                                        {
                                            let skew = data_change.abs_diff(ref_change);
                                            if skew > limit_val
                                                && (data_change == current_time || ref_change == current_time)
                                            {
                                                let key = ("$timeskew".to_string(), *sid);
                                                if self.timing_reported.get(&key) != Some(&current_time) {
                                                    self.timing_reported.insert(key, current_time);
                                                    self.emit_warning(
                                                        crate::diagnostics::DiagCode::TimingViolation,
                                                        format!("$timeskew violation: skew {}ns > max {}ns between '{}' and '{}'", skew, limit_val, data_sig, ref_sig),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                SpecifyItem::NochangeCheck {
                    ref_event: _ref_event,
                    ref_edge: _,
                    data,
                    start_limit,
                    end_limit,
                } => {
                    let start_val = const_eval_simple(start_limit).unwrap_or(0) as u64;
                    let end_val = const_eval_simple(end_limit).unwrap_or(0) as u64;
                    if let Expr::Ident { name: data_sig, .. } = data {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()) {
                            if let Some(&last_change) = self.signal_last_change.get(sid) {
                                let delta = current_time - last_change;
                                if delta > 0 && delta >= start_val && delta <= end_val {
                                    let key = ("$nochange".to_string(), *sid);
                                    if self.timing_reported.get(&key) != Some(&last_change) {
                                        self.timing_reported.insert(key, last_change);
                                        self.emit_warning(
                                            crate::diagnostics::DiagCode::TimingViolation,
                                            format!("$nochange violation: signal '{}' changed within window [{}ns, {}ns] (delta={}ns)", data_sig, start_val, end_val, delta),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }


    pub(crate) fn evaluate_dpi_call(
        &mut self,
        name: &str,
        args: &[IrExpr],
        return_width: usize,
    ) -> Result<LogicVec, SimError> {
        // Check if we have a matching DPI import
        // Clone immediately to avoid borrow conflicts with self.call_
        let dpi_info = self
            .design
            .dpi_imports
            .iter()
            .find(|d| d.name == name)
            .cloned();
        let is_task = dpi_info.as_ref().map(|d| d.is_task).unwrap_or(false);
        
        if is_task {
            return Ok(LogicVec::new(0));
        }
        
        let arg_vals: Vec<LogicVec> = args
            .iter()
            .map(|a| self.evaluate_expr(a))
            .collect::<Result<_, _>>()?;
        
        // Known built-in DPI functions (work without external libraries)
        match name {
            "svBitToInt" | "svToInt" => {
                if let Some(val) = arg_vals.first() {
                    return Ok(LogicVec::from_u64(val.to_u64(), return_width));
                }
                Ok(LogicVec::from_u64(0, return_width))
            }
            "svBitToLong" | "svToLong" => {
                if let Some(val) = arg_vals.first() {
                    return Ok(LogicVec::from_u64(val.to_u64(), return_width));
                }
                Ok(LogicVec::from_u64(0, return_width))
            }
            "svToShortReal" | "svToReal" => {
                if let Some(val) = arg_vals.first() {
                    return Ok(val.clone());
                }
                Ok(LogicVec::from_u64(0, return_width))
            }
            "svIntToBit" | "svToBit" | "svToLogic" => {
                if let Some(val) = arg_vals.first() {
                    return Ok(val.clone());
                }
                Ok(LogicVec::from_u64(0, return_width))
            }
            "svBitToBitVal" | "svBitToLogicVal" => {
                if let Some(val) = arg_vals.first() {
                    return Ok(val.clone());
                }
                Ok(LogicVec::from_u64(0, return_width))
            }
            "svRandomize" | "sv$random" | "svUrandom" | "svUrandomRange" => {
                let r: u64 = self.rng.gen();
                Ok(LogicVec::from_u64(r, return_width))
            }
            "$test$plusargs" | "svTestPlusArgs" => {
                // Handled in SysFunc dispatch — fallback here
                Ok(LogicVec::from_u64(0, return_width))
            }
            "$value$plusargs" | "svValuePlusArgs" => Ok(LogicVec::from_u64(0, return_width)),
            _ => {
                // Try to resolve via DPI engine (dynamic library loading)
                if dpi_info.is_some() {
                    return self.call_dpi_function(name, &arg_vals, return_width, is_task);
                }
                // Unknown DPI — return 0 (F20: via DiagSink agar punya lokasi)
                self.emit_warning(
                    crate::diagnostics::DiagCode::DpiError,
                    format!("DPI function '{}' not found in imports, returning 0", name),
                );
                Ok(LogicVec::from_u64(0, return_width))
            }
        }
    }

    /// Call a DPI function via the DPI engine (dynamic library resolution).
    #[cfg(feature = "dpi")]
    fn call_dpi_function(
        &mut self,
        name: &str,
        arg_vals: &[LogicVec],
        return_width: usize,
        _is_task: bool,
    ) -> Result<LogicVec, SimError> {
        use crate::simulator::dpi::*;
        use std::sync::Mutex;

        // Global DPI engine (lazy initialized)
        fn dpi_engine() -> &'static Mutex<Option<DpiEngine>> {
            use std::sync::OnceLock;
            static ENGINE: OnceLock<Mutex<Option<DpiEngine>>> = OnceLock::new();
            ENGINE.get_or_init(|| Mutex::new(Some(DpiEngine::new())))
        }

        // Find matching IrDpiImport and register library
        // Clone the info first to avoid borrow conflicts
        let dpi_info = self.design.dpi_imports.iter()
            .find(|d| d.name.as_str() == name)
            .cloned();

        if let Some(ref info) = dpi_info {
            let mut engine_guard = dpi_engine().lock().unwrap();
            if let Some(ref mut engine) = *engine_guard {
                // Try to resolve and call the function
                match engine.resolve_function(info) {
                    Ok(()) => {
                        // Create scope from instance path (cloned to avoid borrow)
                        let scope_path = self.current_instance_path.clone().unwrap_or_default();
                        let scope = crate::simulator::dpi::current_scope_from_path(&scope_path);
                        return engine.call_function(name, arg_vals, &scope);
                    }
                    Err(e) => {
                        self.emit_warning(
                            crate::diagnostics::DiagCode::DpiError,
                            format!("DPI '{}' not found in loaded libraries: {}", name, e),
                        );
                    }
                }
            }
        }

        // Fallback
        Ok(LogicVec::from_u64(0, return_width.max(1)))
    }

    /// Non-DPI fallback when dpi feature is not compiled in
    #[cfg(not(feature = "dpi"))]
    fn call_dpi_function(
        &mut self,
        name: &str,
        _arg_vals: &[LogicVec],
        return_width: usize,
        _is_task: bool,
    ) -> Result<LogicVec, SimError> {
        self.emit_warning(
            crate::diagnostics::DiagCode::DpiError,
            format!("DPI function '{}' not available (compile with --features dpi)", name),
        );
        Ok(LogicVec::from_u64(0, return_width.max(1)))
    }


    pub(crate) fn handle_ast_syscall(
        &mut self,
        name: &str,
        args: &[crate::ast::Expr],
    ) -> Result<(), SimError> {
        if name == "display" || name == "write" {
            let ir_args: Vec<IrExpr> = args
                .iter()
                .map(|a| match a {
                    // String format harus tetap IrExpr::String agar format_display
                    // mengenalinya sebagai format (bukan diubah jadi Const biner).
                    crate::ast::Expr::String(s) => IrExpr::String(s.clone()),
                    _ => IrExpr::Const(self.evaluate_ast_expr(a).unwrap_or(LogicVec::new(32))),
                })
                .collect();
            let msg = self.format_display(&ir_args);
            if name == "display" {
                println!("{}", msg);
            } else {
                print!("{}", msg);
            }
        } else if name == "finish" {
            self.running = false;
        } else if name == "info" || name == "warning" || name == "error" || name == "fatal" {
            // $info / $warning / $error / $fatal (LRM 1800 §20.2) — jalur AST.
            // Format argumen seperti $display; $fatal menghentikan simulasi.
            let ir_args: Vec<IrExpr> = args
                .iter()
                .map(|a| match a {
                    crate::ast::Expr::String(s) => IrExpr::String(s.clone()),
                    _ => IrExpr::Const(self.evaluate_ast_expr(a).unwrap_or(LogicVec::new(32))),
                })
                .collect();
            let msg = self.format_severity_message(&ir_args);
            self.emit_severity(name, &msg);
        }
        Ok(())
    }

}
