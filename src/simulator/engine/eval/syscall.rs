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
                    ref_event: _ref_event,
                    limit,
                } => {
                    // _ref_event is parsed but runtime edge detection is simplified
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let Expr::Ident { name: data_sig, .. } = data {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()) {
                            if let Some(&last_change) = self.signal_last_change.get(sid) {
                                let delta = current_time - last_change;
                                if delta <= limit_val && delta > 0 {
                                    eprintln!("TIMING WARNING: $setup violation: data '{}' changed {}ns before ref (limit={}ns)",
                                        data_sig, delta, limit_val);
                                }
                            }
                        }
                    }
                }
                SpecifyItem::HoldCheck {
                    ref_event: _ref_event,
                    data,
                    limit,
                } => {
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let Expr::Ident { name: data_sig, .. } = data {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()) {
                            if let Some(&last_change) = self.signal_last_change.get(sid) {
                                let delta = current_time - last_change;
                                if delta > 0 && delta <= limit_val {
                                    eprintln!("TIMING WARNING: $hold violation: data '{}' changed {}ns before ref (limit={}ns)",
                                        data_sig, delta, limit_val);
                                }
                            }
                        }
                    }
                }
                SpecifyItem::SetupHoldCheck {
                    ref_event: _ref_event,
                    data,
                    setup_limit,
                    hold_limit,
                } => {
                    let setup_val = const_eval_simple(setup_limit).unwrap_or(0) as u64;
                    let hold_val = const_eval_simple(hold_limit).unwrap_or(0) as u64;
                    if let Expr::Ident { name: data_sig, .. } = data {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()) {
                            if let Some(&last_change) = self.signal_last_change.get(sid) {
                                let delta = current_time - last_change;
                                if delta > 0 && delta <= setup_val {
                                    eprintln!("TIMING WARNING: $setuphold (setup) violation: data '{}' changed {}ns before ref (setup={}ns)",
                                        data_sig, delta, setup_val);
                                }
                                if delta > 0 && delta <= hold_val {
                                    eprintln!("TIMING WARNING: $setuphold (hold) violation: data '{}' changed {}ns before ref (hold={}ns)",
                                        data_sig, delta, hold_val);
                                }
                            }
                        }
                    }
                }
                SpecifyItem::RecoveryCheck {
                    data,
                    ref_event: _ref_event,
                    limit,
                } => {
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let Expr::Ident { name: data_sig, .. } = data {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()) {
                            if let Some(&last_change) = self.signal_last_change.get(sid) {
                                let delta = current_time - last_change;
                                if delta > 0 && delta <= limit_val {
                                    eprintln!("TIMING WARNING: $recovery violation: signal '{}' changed {}ns before ref (limit={}ns)", data_sig, delta, limit_val);
                                }
                            }
                        }
                    }
                }
                SpecifyItem::RemovalCheck {
                    ref_event: _ref_event,
                    data,
                    limit,
                } => {
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let Expr::Ident { name: data_sig, .. } = data {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == data_sig.as_str()) {
                            if let Some(&last_change) = self.signal_last_change.get(sid) {
                                let delta = current_time - last_change;
                                if delta > 0 && delta <= limit_val {
                                    eprintln!("TIMING WARNING: $removal violation: signal '{}' changed {}ns before ref (limit={}ns)", data_sig, delta, limit_val);
                                }
                            }
                        }
                    }
                }
                SpecifyItem::RecoveryRemovalCheck {
                    ref_event: _ref_event,
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
                                    eprintln!("TIMING WARNING: $recrem (recovery) violation: signal '{}' changed {}ns before ref (recov={}ns)", data_sig, delta, recov_val);
                                }
                                if delta > 0 && delta <= remov_val {
                                    eprintln!("TIMING WARNING: $recrem (removal) violation: signal '{}' changed {}ns before ref (remov={}ns)", data_sig, delta, remov_val);
                                }
                            }
                        }
                    }
                }
                SpecifyItem::PeriodCheck { ref_event, limit } => {
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let Expr::Ident { name: ref_sig, .. } = ref_event {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == ref_sig.as_str()) {
                            if let Some(&last_change) = self.signal_last_change.get(sid) {
                                let delta = current_time - last_change;
                                if delta > 0 && delta < limit_val {
                                    eprintln!("TIMING WARNING: $period violation: signal '{}' period {}ns < minimum {}ns", ref_sig, delta, limit_val);
                                }
                            }
                        }
                    }
                }
                SpecifyItem::WidthCheck {
                    ref_event,
                    limit,
                    threshold: _threshold,
                } => {
                    let limit_val = const_eval_simple(limit).unwrap_or(0) as u64;
                    if let Expr::Ident { name: ref_sig, .. } = ref_event {
                        if let Some((_, sid)) = signal_names.iter().find(|(n, _)| n.as_str() == ref_sig.as_str()) {
                            if let Some(&last_change) = self.signal_last_change.get(sid) {
                                let delta = current_time - last_change;
                                if delta > 0 && delta < limit_val {
                                    eprintln!("TIMING WARNING: $width violation: signal '{}' pulse width {}ns < minimum {}ns", ref_sig, delta, limit_val);
                                }
                            }
                        }
                    }
                }
                SpecifyItem::SkewCheck {
                    ref_event,
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
                                            let skew = if data_change > ref_change {
                                                data_change - ref_change
                                            } else {
                                                ref_change - data_change
                                            };
                                            if skew > limit_val {
                                                eprintln!("TIMING WARNING: $skew violation: skew {}ns > max {}ns between '{}' and '{}'", skew, limit_val, data_sig, ref_sig);
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
                                            let skew = if data_change > ref_change {
                                                data_change - ref_change
                                            } else {
                                                ref_change - data_change
                                            };
                                            if skew > limit_val {
                                                eprintln!("TIMING WARNING: $timeskew violation: skew {}ns > max {}ns between '{}' and '{}'", skew, limit_val, data_sig, ref_sig);
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
                                    eprintln!("TIMING WARNING: $nochange violation: signal '{}' changed within window [{}ns, {}ns] (delta={}ns)", data_sig, start_val, end_val, delta);
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
                return Ok(LogicVec::from_u64(0, return_width));
            }
            "svBitToLong" | "svToLong" => {
                if let Some(val) = arg_vals.first() {
                    return Ok(LogicVec::from_u64(val.to_u64(), return_width));
                }
                return Ok(LogicVec::from_u64(0, return_width));
            }
            "svToShortReal" | "svToReal" => {
                if let Some(val) = arg_vals.first() {
                    return Ok(val.clone());
                }
                return Ok(LogicVec::from_u64(0, return_width));
            }
            "svIntToBit" | "svToBit" | "svToLogic" => {
                if let Some(val) = arg_vals.first() {
                    return Ok(val.clone());
                }
                return Ok(LogicVec::from_u64(0, return_width));
            }
            "svBitToBitVal" | "svBitToLogicVal" => {
                if let Some(val) = arg_vals.first() {
                    return Ok(val.clone());
                }
                return Ok(LogicVec::from_u64(0, return_width));
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
                // Unknown DPI — return 0
                eprintln!("warning: DPI function '{}' not found in imports, returning 0", name);
                Ok(LogicVec::from_u64(0, return_width))
            }
        }
    }

    /// Call a DPI function via the DPI engine (dynamic library resolution).
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
                        eprintln!("warning: DPI '{}' not found in loaded libraries: {}", name, e);
                    }
                }
            }
        }

        // Fallback
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
                .map(|a| IrExpr::Const(self.evaluate_ast_expr(a).unwrap_or(LogicVec::new(32))))
                .collect();
            let msg = format_display(
                &self.state,
                &self.design.top.signals,
                &self.design.hier_signal_map,
                &self.assoc_data,
                &ir_args,
            );
            print!("{}", msg);
        } else if name == "finish" {
            self.running = false;
        }
        Ok(())
    }

}
