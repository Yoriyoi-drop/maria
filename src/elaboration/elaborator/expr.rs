use std::collections::HashMap;
use super::Elaborator;
use super::BUILTIN_UVM_CLASSES;
use super::super::util::*;
use crate::ast::types::{const_eval_simple, const_eval_with_params};
use crate::ast::*;
use crate::diagnostics::diagnostic::DiagCode;
use crate::error::SimError;

use crate::intern::Symbol;
use crate::ir::*;

impl Elaborator {
    pub(crate) fn build_hier_name(obj: &Expr, field: &str) -> String {
        match obj {
            Expr::Ident { name: prefix, .. } => format!("{}.{}", prefix, field),
            Expr::MemberAccess {
                obj: inner,
                field: inner_field,
            } => {
                format!("{}.{}", Self::build_hier_name(inner, inner_field.as_str()), field)
            }
            _ => String::new(),
        }
    }

    pub(crate) fn elaborate_expr(
        &self,
        expr: &Expr,
        signal_map: &HashMap<Symbol, SignalId>,
        signals: &[SignalInfo],
    ) -> Result<IrExpr, SimError> {
        match expr {
            Expr::Ident { name, .. } if name == "this" => Ok(IrExpr::This),
            Expr::Value(v) => {
                let lv = value_to_logicvec(v);
                let is_signed = matches!(
                    v,
                    Value::Binary {
                        is_signed: true,
                        ..
                    } | Value::Hex {
                        is_signed: true,
                        ..
                    } | Value::Octal {
                        is_signed: true,
                        ..
                    }
                );
                if is_signed {
                    Ok(IrExpr::Signed(Box::new(IrExpr::Const(lv))))
                } else {
                    Ok(IrExpr::Const(lv))
                }
            }
            Expr::FillLit(val) => Ok(IrExpr::FillLit(*val)),
            Expr::Ident { name, line, col } => {
                if name.starts_with("$") {
                    return Ok(IrExpr::SysFunc {
                        name: *name,
                        args: vec![],
                    });
                }
                // Check if this ident is a parameter (from param_vals or effective_params)
                if let Some(&val) = self.param_vals.get(name) {
                    return Ok(IrExpr::Const(LogicVec::from_u64(val as u64, 64)));
                }
                let sig_id = signal_map
                    .get(name)
                    .ok_or_else(|| self.elab_diag_at(DiagCode::UndefinedSignal, format!("signal '{}' not found", name), *line, *col))?;
                Ok(IrExpr::Signal(*sig_id, 0))
            }
            Expr::ScopedIdent { package, item } => {
                if let Some(pkg_items) = self.package_symbols.get(package) {
                    if let Some(pkg_item) = pkg_items.get(item) {
                        match pkg_item {
                            PackageItem::Param(p) => {
                                if let Some(expr) = &p.default {
                                    if let Ok(val) = const_eval_with_params(expr, &self.param_vals)
                                    {
                                        return Ok(IrExpr::Const(LogicVec::from_u64(
                                            val as u64, 64,
                                        )));
                                    }
                                }
                                return Err(self.elab_diag(DiagCode::ParamMismatch, format!(
                                    "package param '{}.{}' has no default",
                                    package, item
                                )));
                            }
                            _ => {
                                return Err(self.elab_diag(DiagCode::ModuleNotFound, format!(
                                    "'{}' is not a constant in package '{}'",
                                    item, package
                                )))
                            }
                        }
                    }
                }
                Err(self.elab_diag(DiagCode::ModuleNotFound, format!(
                    "'{}' not found in package '{}'",
                    item, package
                )))
            }
            Expr::RangeSelect {
                expr: inner,
                msb,
                lsb,
            } => {
                // Const-fold array/param range select (e.g. `pkg::ARR[3:0]`)
                if let Some(folded) = try_fold_const(expr, &self.param_vals)? {
                    return Ok(folded);
                }
                let inner_expr = self.elaborate_expr(inner, signal_map, signals)?;
                if let (Ok(msb_c), Ok(lsb_c)) = (
                    const_eval_params(msb, &self.param_vals),
                    const_eval_params(lsb, &self.param_vals),
                ) {
                    let msb_c = msb_c as usize;
                    let lsb_c = lsb_c as usize;
                    if let IrExpr::Signal(sid, _) = &inner_expr {
                        Ok(IrExpr::RangeSelect(*sid, msb_c, lsb_c))
                    } else {
                        Ok(IrExpr::ExprRangeSelect(Box::new(inner_expr), msb_c, lsb_c))
                    }
                } else {
                    Err(self.elab_diag(DiagCode::ModuleNotFound, "dynamic range select not supported"))
                }
            }
            Expr::BitSelect { expr: inner, index } => {
                // Const-fold array/param element select (e.g. `pkg::ARR[2]` via
                // flattened param_vals key `ARR[2]`). Falls through if not constant.
                if let Some(folded) = try_fold_const(expr, &self.param_vals)? {
                    return Ok(folded);
                }
                let inner_expr = self.elaborate_expr(inner, signal_map, signals)?;
                if let IrExpr::Signal(sid, _) = &inner_expr {
                    let sig = &signals[*sid];
                    // Check for multi-dim packed array: packed_dims.len() > 1
                    if sig.packed_dims.len() > 1 {
                        let outer_elem_width = sig.width / sig.packed_dims[0];
                        if let Ok(idx) = const_eval_params(index, &self.param_vals) {
                            let idx = idx as usize;
                            let lsb = idx * outer_elem_width;
                            let msb = lsb + outer_elem_width - 1;
                            Ok(IrExpr::RangeSelect(*sid, msb, lsb))
                        } else {
                            let index_expr = self.elaborate_expr(index, signal_map, signals)?;
                            let base_expr = IrExpr::BinaryOp(
                                BinaryIrOp::Mul,
                                Box::new(index_expr),
                                Box::new(IrExpr::Const(LogicVec::from_u64(
                                    outer_elem_width as u64,
                                    32,
                                ))),
                            );
                            Ok(IrExpr::ExprPartSelect(
                                Box::new(IrExpr::Signal(*sid, sig.width)),
                                Box::new(base_expr),
                                Box::new(IrExpr::Const(LogicVec::from_u64(
                                    outer_elem_width as u64,
                                    32,
                                ))),
                            ))
                        }
                    } else if sig.array_depth > 1 || sig.is_dynamic || sig.is_queue {
                        let index_expr = self.elaborate_expr(index, signal_map, signals)?;
                        Ok(IrExpr::ArrayIndex {
                            sig_id: *sid,
                            index: Box::new(index_expr),
                            elem_width: sig.elem_width,
                        })
                    } else if let Ok(idx) = const_eval_params(index, &self.param_vals) {
                        Ok(IrExpr::BitSelect(*sid, idx as usize))
                    } else {
                        // Dynamic index on flat signal — treat as array index
                        let index_expr = self.elaborate_expr(index, signal_map, signals)?;
                        Ok(IrExpr::ArrayIndex {
                            sig_id: *sid,
                            index: Box::new(index_expr),
                            elem_width: sig.elem_width,
                        })
                    }
                } else if let Ok(idx) = const_eval_params(index, &self.param_vals) {
                    Ok(IrExpr::ExprBitSelect(Box::new(inner_expr), idx as usize))
                } else {
                    Err(self.elab_diag(DiagCode::ModuleNotFound,
                        "dynamic bit-select on non-signal not supported",
                    ))
                }
            }
            Expr::Concat(exprs) => {
                if let Some(folded) = try_fold_const(expr, &self.param_vals)? {
                    return Ok(folded);
                }
                let parts: Result<Vec<IrExpr>, SimError> = exprs
                    .iter()
                    .map(|e| self.elaborate_expr(e, signal_map, signals))
                    .collect();
                Ok(IrExpr::Concat(parts?))
            }
            Expr::Replicate { count, expr: inner } => {
                if let Some(folded) = try_fold_const(expr, &self.param_vals)? {
                    return Ok(folded);
                }
                let c = const_eval_params(count, &self.param_vals)? as usize;
                let inner_expr = self.elaborate_expr(inner, signal_map, signals)?;
                Ok(IrExpr::Replicate(c, Box::new(inner_expr)))
            }
            Expr::UnaryOp { op, expr: inner } => {
                if let Some(folded) = try_fold_const(expr, &self.param_vals)? {
                    return Ok(folded);
                }
                let inner_expr = self.elaborate_expr(inner, signal_map, signals)?;
                let ir_op = map_unary_op(op)?;
                Ok(IrExpr::UnaryOp(ir_op, Box::new(inner_expr)))
            }
            Expr::BinaryOp { op, lhs, rhs } => {
                if let Some(folded) = try_fold_const(expr, &self.param_vals)? {
                    return Ok(folded);
                }
                let lhs_expr = self.elaborate_expr(lhs, signal_map, signals)?;
                let rhs_expr = self.elaborate_expr(rhs, signal_map, signals)?;
                let ir_op = map_binary_op(op)?;
                Ok(IrExpr::BinaryOp(
                    ir_op,
                    Box::new(lhs_expr),
                    Box::new(rhs_expr),
                ))
            }
            Expr::TernaryOp {
                cond,
                true_expr,
                false_expr,
            } => {
                if let Some(folded) = try_fold_const(expr, &self.param_vals)? {
                    return Ok(folded);
                }
                let ir_cond = self.elaborate_expr(cond, signal_map, signals)?;
                let ir_true = self.elaborate_expr(true_expr, signal_map, signals)?;
                let ir_false = self.elaborate_expr(false_expr, signal_map, signals)?;
                Ok(IrExpr::Cond(
                    Box::new(ir_cond),
                    Box::new(ir_true),
                    Box::new(ir_false),
                ))
            }
            Expr::PartSelect {
                expr: inner,
                base,
                width,
            } => {
                let inner_expr = self.elaborate_expr(inner, signal_map, signals)?;
                if let IrExpr::Signal(sid, _) = &inner_expr {
                    if let (Ok(base_c), Ok(width_c)) = (
                        const_eval_params(base, &self.param_vals),
                        const_eval_params(width, &self.param_vals),
                    ) {
                        let base = base_c as usize;
                        let width = width_c as usize;
                        if width > 0 {
                            Ok(IrExpr::RangeSelect(*sid, base + width - 1, base))
                        } else {
                            Ok(IrExpr::RangeSelect(*sid, base, base))
                        }
                    } else {
                        let base_expr = self.elaborate_expr(base, signal_map, signals)?;
                        let width_expr = self.elaborate_expr(width, signal_map, signals)?;
                        Ok(IrExpr::ExprPartSelect(
                            Box::new(inner_expr),
                            Box::new(base_expr),
                            Box::new(width_expr),
                        ))
                    }
                } else if let (Ok(base_c), Ok(width_c)) = (
                    const_eval_params(base, &self.param_vals),
                    const_eval_params(width, &self.param_vals),
                ) {
                    let base = base_c as usize;
                    let width = width_c as usize;
                    if width > 0 {
                        Ok(IrExpr::ExprRangeSelect(
                            Box::new(inner_expr),
                            base + width - 1,
                            base,
                        ))
                    } else {
                        Ok(IrExpr::ExprRangeSelect(Box::new(inner_expr), base, base))
                    }
                } else {
                    let base_expr = self.elaborate_expr(base, signal_map, signals)?;
                    let width_expr = self.elaborate_expr(width, signal_map, signals)?;
                    Ok(IrExpr::ExprPartSelect(
                        Box::new(inner_expr),
                        Box::new(base_expr),
                        Box::new(width_expr),
                    ))
                }
            }
            Expr::Paren(inner) => self.elaborate_expr(inner, signal_map, signals),
            Expr::FuncCall { name, args } if name.starts_with("$") => match name.as_str() {
                "$signed" => {
                    if args.len() != 1 {
                        return Err(self.elab_diag(DiagCode::ParamMismatch, "$signed requires exactly one argument"));
                    }
                    let inner = self.elaborate_expr(&args[0], signal_map, signals)?;
                    Ok(IrExpr::Signed(Box::new(inner)))
                }
                "$unsigned" => {
                    if args.len() != 1 {
                        return Err(self.elab_diag(DiagCode::ParamMismatch,
                            "$unsigned requires exactly one argument",
                        ));
                    }
                    self.elaborate_expr(&args[0], signal_map, signals)
                }
                "$clog2" => {
                    if let Some(arg) = args.first() {
                        let val = const_eval_params(arg, &self.param_vals)?;
                        if val <= 1 {
                            return Ok(IrExpr::Const(LogicVec::from_u64(0, 32)));
                        }
                        let n = val as u64;
                        let msb = (64 - n.leading_zeros()) as u64;
                        let result = if n.is_power_of_two() { msb - 1 } else { msb };
                        Ok(IrExpr::Const(LogicVec::from_u64(result, 32)))
                    } else {
                        Err(self.elab_diag(DiagCode::ParamMismatch, "$clog2 requires one argument"))
                    }
                }
                "$bits" => {
                    if let Some(arg) = args.first() {
                        // Signal: SignalInfo.width ALREADY includes unpacked array
                        // depth (total_width = elem_width * depth di-set saat elaborasi).
                        // Jangan kalikan lagi dgn array_depth — itu double-count.
                        let width = resolve_expr_signal(arg, signal_map)
                                .map(|sig_id| {
                                    let info = &signals[sig_id];
                                    info.width
                                })
                                .or_else(|| self.try_array_param_bits(arg))
                                .or_else(|| compute_expr_width(arg, signal_map, signals, &self.param_vals, &self.package_symbols).ok())
                                .ok_or_else(|| self.elab_diag(DiagCode::ModuleNotFound, "$bits argument must resolve to a signal or computable expression"))?;
                        Ok(IrExpr::Const(LogicVec::from_u64(width as u64, 32)))
                    } else {
                        Err(self.elab_diag(DiagCode::ParamMismatch, "$bits requires one argument"))
                    }
                }
                "$high" => {
                    if let Some(arg) = args.first() {
                        let sig_id = resolve_expr_signal(arg, signal_map).ok_or_else(|| {
                            self.elab_diag(DiagCode::ModuleNotFound, "$high argument must resolve to a signal")
                        })?;
                        let info = &signals[sig_id];
                        let high = info.msb.max(info.lsb);
                        Ok(IrExpr::Const(LogicVec::from_u64(high as u64, 32)))
                    } else {
                        Err(self.elab_diag(DiagCode::ParamMismatch, "$high requires one argument"))
                    }
                }
                "$low" => {
                    if let Some(arg) = args.first() {
                        let sig_id = resolve_expr_signal(arg, signal_map).ok_or_else(|| {
                            self.elab_diag(DiagCode::ModuleNotFound, "$low argument must resolve to a signal")
                        })?;
                        let info = &signals[sig_id];
                        let low = info.msb.min(info.lsb);
                        Ok(IrExpr::Const(LogicVec::from_u64(low as u64, 32)))
                    } else {
                        Err(self.elab_diag(DiagCode::ParamMismatch, "$low requires one argument"))
                    }
                }
                "$left" => {
                    if let Some(arg) = args.first() {
                        let sig_id = resolve_expr_signal(arg, signal_map).ok_or_else(|| {
                            self.elab_diag(DiagCode::ModuleNotFound, "$left argument must resolve to a signal")
                        })?;
                        let info = &signals[sig_id];
                        Ok(IrExpr::Const(LogicVec::from_u64(info.msb as u64, 32)))
                    } else {
                        Err(self.elab_diag(DiagCode::ParamMismatch, "$left requires one argument"))
                    }
                }
                "$right" => {
                    if let Some(arg) = args.first() {
                        let sig_id = resolve_expr_signal(arg, signal_map).ok_or_else(|| {
                            self.elab_diag(DiagCode::ModuleNotFound, "$right argument must resolve to a signal")
                        })?;
                        let info = &signals[sig_id];
                        Ok(IrExpr::Const(LogicVec::from_u64(info.lsb as u64, 32)))
                    } else {
                        Err(self.elab_diag(DiagCode::ParamMismatch, "$right requires one argument"))
                    }
                }
                "$size" => {
                    if let Some(arg) = args.first() {
                        let sig_id = resolve_expr_signal(arg, signal_map).ok_or_else(|| {
                            self.elab_diag(DiagCode::ModuleNotFound, "$size argument must resolve to a signal")
                        })?;
                        let info = &signals[sig_id];
                        // $size mengembalikan jumlah elemen pada dimensi pertama.
                        // Prioritas: unpacked array (array_depth) > packed
                        // multi-dimensi (packed_dims[0]) > lebar total.
                        let size = if info.array_depth > 1 {
                            info.array_depth
                        } else if info.packed_dims.len() > 1 {
                            info.packed_dims[0]
                        } else {
                            info.width
                        };
                        Ok(IrExpr::Const(LogicVec::from_u64(size as u64, 32)))
                    } else {
                        Err(self.elab_diag(DiagCode::ParamMismatch, "$size requires one argument"))
                    }
                }
                "$countones" => {
                    if let Some(arg) = args.first() {
                        let ir_arg = self.elaborate_expr(arg, signal_map, signals)?;
                        if let Ok(val) = const_eval_params(arg, &self.param_vals) {
                            let count = (0..64).filter(|i| (val >> i) & 1 == 1).count() as u64;
                            Ok(IrExpr::Const(LogicVec::from_u64(count, 32)))
                        } else {
                            Ok(IrExpr::SysFunc {
                                name: Symbol::intern("$countones"),
                                args: vec![ir_arg],
                            })
                        }
                    } else {
                        Err(self.elab_diag(DiagCode::ParamMismatch, "$countones requires one argument"))
                    }
                }
                "$onehot" => {
                    if let Some(arg) = args.first() {
                        let ir_arg = self.elaborate_expr(arg, signal_map, signals)?;
                        if let Ok(val) = const_eval_params(arg, &self.param_vals) {
                            let ones = (0..64).filter(|i| (val >> i) & 1 == 1).count();
                            Ok(IrExpr::Const(LogicVec::from_u64(
                                if ones == 1 { 1 } else { 0 },
                                1,
                            )))
                        } else {
                            Ok(IrExpr::SysFunc {
                                name: Symbol::intern("$onehot"),
                                args: vec![ir_arg],
                            })
                        }
                    } else {
                        Err(self.elab_diag(DiagCode::ParamMismatch, "$onehot requires one argument"))
                    }
                }
                "$isunknown" => {
                    if let Some(arg) = args.first() {
                        let ir_arg = self.elaborate_expr(arg, signal_map, signals)?;
                        if let Ok(val) = const_eval_params(arg, &self.param_vals) {
                            let has_xz = val as u8 >= 0xFE;
                            Ok(IrExpr::Const(LogicVec::from_u64(
                                if has_xz { 1 } else { 0 },
                                1,
                            )))
                        } else {
                            Ok(IrExpr::SysFunc {
                                name: Symbol::intern("$isunknown"),
                                args: vec![ir_arg],
                            })
                        }
                    } else {
                        Err(self.elab_diag(DiagCode::ParamMismatch, "$isunknown requires one argument"))
                    }
                }
                _ => {
                    let ir_args: Result<Vec<IrExpr>, SimError> = args
                        .iter()
                        .map(|a| self.elaborate_expr(a, signal_map, signals))
                        .collect();
                    Ok(IrExpr::SysFunc {
                        name: *name,
                        args: ir_args?,
                    })
                }
            },
            Expr::FuncCall { name, args } if name == "new" => {
                let ir_args: Result<Vec<IrExpr>, SimError> = args
                    .iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect();
                Ok(IrExpr::NewCall {
                    class_name: Symbol::intern(""),
                    args: ir_args?,
                })
            }
            Expr::String(s) => Ok(IrExpr::String(s.clone())),
            Expr::MethodCall {
                obj,
                method,
                args,
                with_clause,
            } => {
                let ir_obj = self.elaborate_expr(obj, signal_map, signals)?;
                let ir_args: Result<Vec<IrExpr>, SimError> = args
                    .iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect();
                let ir_with = match with_clause {
                    Some(wc) => Some(Box::new(self.elaborate_expr(wc, signal_map, signals)?)),
                    None => None,
                };
                Ok(IrExpr::MethodCall {
                    obj: Box::new(ir_obj),
                    method: *method,
                    args: ir_args?,
                    with_clause: ir_with,
                })
            }
            Expr::MemberAccess { obj, field } => {
                // Try to resolve as hierarchical signal reference first
                let hier_name = Self::build_hier_name(obj, field.as_str());
                if !hier_name.is_empty() {
                    if let Some(&sig_id) = signal_map.get(hier_name.as_str()) {
                        return Ok(IrExpr::Signal(sig_id, 0));
                    }
                }
                // Try struct/union member access: resolve obj signal, check struct_fields
                match self.elaborate_expr(obj, signal_map, signals) {
                    Ok(IrExpr::Signal(sig_id, _)) => {
                        let sig_info = &signals[sig_id];
                        // Check if this is a virtual interface variable
                        if let Some(ref iface_type) = sig_info.iface_type {
                            // Look up the interface definition to find field width
                            let field_width = if let Some(iface) = self
                                .design
                                .interfaces
                                .iter()
                                .find(|i| i.name == *iface_type)
                            {
                                if let Some(d) = iface
                                    .decls
                                    .iter()
                                    .find(|d| d.names.iter().any(|n| n.name == *field))
                                {
                                    let var = d.names.iter().find(|n| n.name == *field).unwrap();
                                    var.resolved_width(&HashMap::new()).unwrap_or(1)
                                } else {
                                    1
                                }
                            } else {
                                1
                            };
                            return Ok(IrExpr::VirtualIfaceAccess {
                                vif_name: sig_info.name,
                                field: *field,
                                field_width,
                            });
                        }
                        if !sig_info.struct_fields.is_empty() {
                            if let Some(f) =
                                sig_info.struct_fields.iter().find(|f| f.name == *field)
                            {
                                let lsb = f.offset;
                                let msb = f.offset + f.width - 1;
                                return Ok(IrExpr::RangeSelect(sig_id, lsb, msb));
                            }
                            return Err(self.elab_diag(DiagCode::ModuleNotFound, format!(
                                "field '{}' not found in struct type (width {})",
                                field, sig_info.width
                            )));
                        }
                        Ok(IrExpr::MemberAccess {
                            obj: Box::new(IrExpr::Signal(sig_id, 0)),
                            field: *field,
                        })
                    }
                    Ok(ir_obj) => Ok(IrExpr::MemberAccess {
                        obj: Box::new(ir_obj),
                        field: *field,
                    }),
                    Err(_) => {
            // If obj can't be elaborated (e.g., instance name), emit a HierRef
            // that the engine can resolve at runtime using the flattened signal list
            Ok(IrExpr::HierRef(Symbol::intern(&hier_name)))
                    }
                }
            }
            Expr::Null => Ok(IrExpr::Const(LogicVec::from_u64(0, 64))),
            Expr::Inside {
                expr: inner,
                range_list,
            } => {
                let inner_ir = self.elaborate_expr(inner, signal_map, signals)?;
                let mut list_ir = Vec::with_capacity(range_list.len());
                for item in range_list {
                    list_ir.push(self.elaborate_expr(item, signal_map, signals)?);
                }
                Ok(IrExpr::Inside {
                    expr: Box::new(inner_ir),
                    list: list_ir,
                })
            }
            Expr::StreamingConcat {
                op,
                slice_size,
                slices,
            } => {
                let mut ir_slices = Vec::new();
                for sl in slices {
                    ir_slices.push(self.elaborate_expr(sl, signal_map, signals)?);
                }
                let ir_slice_size = if let Some(ss) = slice_size {
                    match const_eval_params(ss, &self.param_vals) {
                        Ok(v) if v > 0 => Some(v as usize),
                        Ok(_) => {
                            return Err(self.elab_diag(DiagCode::ParamMismatch, "streaming slice_size must be > 0"))
                        }
                        Err(_) => {
                            return Err(self.elab_diag(DiagCode::ParamMismatch,
                                "slice_size must be a constant expression",
                            ))
                        }
                    }
                } else {
                    None
                };
                Ok(IrExpr::StreamingConcat {
                    op: op.clone(),
                    slice_size: ir_slice_size,
                    slices: ir_slices,
                })
            }
            Expr::Dist { expr: inner, items } => {
                let inner_ir = self.elaborate_expr(inner, signal_map, signals)?;
                let ir_items = items
                    .iter()
                    .map(|di| match di {
                        crate::ast::DistItem::Value(e, crate::ast::DistWeight::Item(w)) => {
                            let ev = self
                                .elaborate_expr(e, signal_map, signals)
                                .unwrap_or(IrExpr::Const(LogicVec::from_u64(0, 32)));
                            let lo = if let IrExpr::Const(ref lv) = ev {
                                Some(lv.to_u64() as i64)
                            } else {
                                None
                            };
                            crate::ir::IrDistItem {
                                range_lo: lo,
                                range_hi: lo,
                                weight_type: crate::ir::DistWeightType::Item,
                                weight: *w as i64,
                            }
                        }
                        crate::ast::DistItem::Value(e, crate::ast::DistWeight::Range(w)) => {
                            let ev = self
                                .elaborate_expr(e, signal_map, signals)
                                .unwrap_or(IrExpr::Const(LogicVec::from_u64(0, 32)));
                            let lo = if let IrExpr::Const(ref lv) = ev {
                                Some(lv.to_u64() as i64)
                            } else {
                                None
                            };
                            crate::ir::IrDistItem {
                                range_lo: lo,
                                range_hi: lo,
                                weight_type: crate::ir::DistWeightType::Range,
                                weight: *w as i64,
                            }
                        }
                        crate::ast::DistItem::Range(lo, hi, crate::ast::DistWeight::Item(w)) => {
                            let lo_v = const_eval_with_params(lo, &self.param_vals).ok();
                            let hi_v = const_eval_with_params(hi, &self.param_vals).ok();
                            crate::ir::IrDistItem {
                                range_lo: lo_v,
                                range_hi: hi_v,
                                weight_type: crate::ir::DistWeightType::Item,
                                weight: *w as i64,
                            }
                        }
                        crate::ast::DistItem::Range(lo, hi, crate::ast::DistWeight::Range(w)) => {
                            let lo_v = const_eval_with_params(lo, &self.param_vals).ok();
                            let hi_v = const_eval_with_params(hi, &self.param_vals).ok();
                            crate::ir::IrDistItem {
                                range_lo: lo_v,
                                range_hi: hi_v,
                                weight_type: crate::ir::DistWeightType::Range,
                                weight: *w as i64,
                            }
                        }
                    })
                    .collect::<Vec<_>>();
                Ok(IrExpr::Dist {
                    expr: Box::new(inner_ir),
                    items: ir_items,
                })
            }
            Expr::Cast { dtype, expr: inner } => {
                let inner_ir = self.elaborate_expr(inner, signal_map, signals)?;
                let cast_width = match                        parse_type_spec_str(dtype.as_str()) {
                    Some(dt) => self.resolve_type_width(&dt).unwrap_or(1),
                    None => 1,
                };
                Ok(IrExpr::Cast {
                    width: cast_width,
                    expr: Box::new(inner_ir),
                })
            }
            Expr::FuncCall { name, args } if name.starts_with("process::") => {
                let ir_args: Result<Vec<IrExpr>, SimError> = args
                    .iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect();
                Ok(IrExpr::SysFunc {
                    name: *name,
                    args: ir_args?,
                })
            }
            Expr::FuncCall { name, args }
                if name.ends_with("::new")
                    && (self
                        .design
                        .classes
                        .iter()
                        .any(|c| *name == format!("{}::new", c.name))
                        || BUILTIN_UVM_CLASSES
                            .iter()
                            .any(|c| *name == format!("{}::new", c))
                        || name.contains("#")) =>
            {
                let raw_name = name.strip_suffix("::new").unwrap().to_string();
                let class_name = if let Some(hash_pos) = raw_name.find('#') {
                    let base = &raw_name[..hash_pos];
                    let type_spec = &raw_name[hash_pos + 1..];
                    let specialized = format!("{}__param_{}", base, type_spec.replace(',', "_"));
                    let exists_in_design =
                        self.design.classes.iter().any(|c| c.name == specialized);
                    let exists_in_spec = self
                        .specialized_classes
                        .borrow()
                        .iter()
                        .any(|c| c.name == specialized);
                    if !exists_in_design && !exists_in_spec {
                        let orig = self.design.classes.iter().find(|c| c.name == base).cloned();
                        if let Some(mut spec) = orig {
                            let tp_name = spec.type_params.first().map(|tp| tp.name);
                            spec.name = Symbol::intern(&specialized);
                            if let Some(ref param_name) = tp_name {
                                let type_dt = parse_type_spec_str(type_spec);
                                if let Some(ref dt) = type_dt {
                                    spec = substitute_class_types(spec, param_name.as_str(), dt);
                                }
                            }
                            self.specialized_classes.borrow_mut().push(spec);
                        }
                    }
                    Symbol::intern(&specialized)
                } else if BUILTIN_UVM_CLASSES.contains(&raw_name.as_str()) {
                    Symbol::intern(&format!("__{}", raw_name))
                } else {
                    Symbol::intern(&raw_name)
                };
                let ir_args: Result<Vec<IrExpr>, SimError> = args
                    .iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect();
                Ok(IrExpr::NewCall {
                    class_name,
                    args: ir_args?,
                })
            }
            Expr::FuncCall { name, args } if name == "uvm_factory::set_type_override_by_type" => {
                let ir_args: Result<Vec<IrExpr>, SimError> = args
                    .iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect();
                Ok(IrExpr::SysFunc {
                    name: *name,
                    args: ir_args?,
                })
            }
            Expr::FuncCall { name, args }
                if name == "uvm_config_db::set"
                    || name == "uvm_config_db::get"
                    || name == "uvm_resource_db::set"
                    || name == "uvm_resource_db::get" =>
            {
                let ir_args: Result<Vec<IrExpr>, SimError> = args
                    .iter()
                    .map(|a| self.elaborate_expr(a, signal_map, signals))
                    .collect();
                // Use SysFunc variant for engine dispatch
                Ok(IrExpr::SysFunc {
                    name: *name,
                    args: ir_args?,
                })
            }                    Expr::FuncCall { name, args } if name != "new" && name.contains("::") => {
                        self.elaborate_package_func_call(name.as_str(), args, signal_map, signals)
            }
            Expr::FuncCall { name, args } if name != "new" => {
                let is_dpi = self
                    .design
                    .modules
                    .iter()
                    .flat_map(|m| m.items.iter())
                    .any(|item| matches!(item, ModuleItem::DpiImport(d) if d.name == *name));
                if is_dpi {
                    let ir_args: Result<Vec<IrExpr>, SimError> = args
                        .iter()
                        .map(|a| self.elaborate_expr(a, signal_map, signals))
                        .collect();
                    let return_width = self
                        .design
                        .modules
                        .iter()
                        .flat_map(|m| m.items.iter())
                        .filter_map(|item| {
                            if let ModuleItem::DpiImport(d) = item {
                                Some(d)
                            } else {
                                None
                            }
                        })
                        .find(|d| d.name == *name)
                        .and_then(|d| d.return_type.as_ref())
                        .map(|dt| dt.width())
                        .unwrap_or(32);
                    Ok(IrExpr::DpiCall {
                        name: *name,
                        args: ir_args?,
                        return_width,
                    })
                } else {
                    // Check if this is a module-level function (recursive, not inlined)
                    let func_exists = self.design.modules.iter().any(|m| {
                        m.items
                            .iter()
                            .any(|mi| matches!(mi, ModuleItem::Func(fd) if fd.name == *name))
                    });
                    if func_exists {
                        let ir_args: Result<Vec<IrExpr>, SimError> = args
                            .iter()
                            .map(|a| self.elaborate_expr(a, signal_map, signals))
                            .collect();
                        return Ok(IrExpr::FuncCall {
                            func_name: *name,
                            args: ir_args?,
                        });
                    }
                    // Plain-name package function via import (pkg::* / pkg::item)
                    if let Some(ir) =
                        self.elaborate_imported_package_func_call(name.as_str(), args, signal_map, signals)?
                    {
                        return Ok(ir);
                    }
                    Err(self.elab_diag(DiagCode::ModuleNotFound, format!(
                        "function '{}' not found (not a DPI import)",
                        name
                    )))
                }
            }              _ => Err(self.elab_diag(DiagCode::ModuleNotFound, "expression type not yet supported".to_string())),
        }
    }

    fn elaborate_package_func_call(
        &self,
        name: &str,
        args: &[Expr],
        signal_map: &HashMap<Symbol, SignalId>,
        signals: &[SignalInfo],
    ) -> Result<IrExpr, SimError> {
        let (pkg_name, func_name) = name
            .split_once("::")
            .ok_or_else(|| self.elab_diag(DiagCode::ModuleNotFound, format!("invalid function name '{}'", name)))?;
        self.elaborate_package_func(pkg_name, func_name, args, signal_map, signals)
    }

    /// Kumpulkan set import aktif: import $unit (`design.unit_imports`) + import
    /// di body module saat ini (`ModuleItem::Import`). Dipakai oleh resolusi
    /// plain-name package (fungsi & konstanta/array).
    fn collect_import_sets(&self) -> Vec<(Symbol, Symbol)> {
        let mut import_sets: Vec<(Symbol, Symbol)> = self.design.unit_imports.clone();
        if let Some(mod_name) = self.current_module {
            if let Some(module) = self.design.modules.iter().find(|m| m.name == mod_name) {
                for item in &module.items {
                    if let ModuleItem::Import { package, item: import_item } = item {
                        import_sets.push((*package, *import_item));
                    }
                }
            }
        }
        import_sets
    }

    /// Cari package yang mengimpor fungsi plain-name (via `import pkg::*`/`import pkg::item`
    /// di $unit atau body module), lalu resolve fungsi package tersebut.
    fn elaborate_imported_package_func_call(
        &self,
        name: &str,
        args: &[Expr],
        signal_map: &HashMap<Symbol, SignalId>,
        signals: &[SignalInfo],
    ) -> Result<Option<IrExpr>, SimError> {
        let import_sets = self.collect_import_sets();
        for (package, import_item) in import_sets {
            let Some(pkg_items) = self.package_symbols.get(&package) else {
                continue;
            };
            let matched = if import_item.as_str() == "*" {
                matches!(pkg_items.get(name), Some(PackageItem::Function(_)))
            } else {
                import_item == name
                    && matches!(pkg_items.get(name), Some(PackageItem::Function(_)))
            };
            if matched {
                let ir = self.elaborate_package_func(package.as_str(), name, args, signal_map, signals)?;
                return Ok(Some(ir));
            }
        }
        Ok(None)
    }

    /// Hitung total bit untuk referensi array utuh (param package) dalam `$bits`,
    /// mis. `$bits(pkg::ARR)` atau `$bits(ARR)` via `import pkg::*`.
    /// Mengembalikan `elem_width * num_elements` bila ter-resolve, selain itu None.
    fn try_array_param_bits(&self, arg: &Expr) -> Option<usize> {
        let candidates: Vec<(Symbol, Symbol)> = match arg {
            Expr::ScopedIdent { package, item } => vec![(*package, *item)],
            Expr::Ident { name, .. } => {
                let mut out = Vec::new();
                for (package, import_item) in self.collect_import_sets() {
                    let Some(pkg_items) = self.package_symbols.get(&package) else {
                        continue;
                    };
                    let matched = if import_item.as_str() == "*" {
                        matches!(pkg_items.get(name), Some(PackageItem::Param(_)))
                    } else {
                        import_item == *name
                            && matches!(pkg_items.get(name), Some(PackageItem::Param(_)))
                    };
                    if matched {
                        out.push((package, *name));
                    }
                }
                out
            }
            _ => return None,
        };

        for (package, item) in candidates {
            let qname = Symbol::intern(&format!("{}::{}", package.as_str(), item.as_str()));
            let Some(elems) = self.pkg_const_arrays.get(&qname) else { continue };
            if elems.is_empty() {
                continue;
            }
            let Some(pkg_items) = self.package_symbols.get(&package) else { continue };
            let elem_width: usize = match pkg_items.get(&item) {
                Some(PackageItem::Param(p)) => {
                    if let Some((msb, lsb)) = &p.range {
                        match (
                            const_eval_with_params(msb, &self.param_vals),
                            const_eval_with_params(lsb, &self.param_vals),
                        ) {
                            (Ok(m), Ok(l)) => m.abs_diff(l) as usize + 1,
                            _ => p.dtype.as_ref().map(|d| d.width()).unwrap_or(32),
                        }
                    } else {
                        p.dtype.as_ref().map(|d| d.width()).unwrap_or(32)
                    }
                }
                _ => continue,
            };
            return Some(elem_width * elems.len());
        }
        None
    }

    fn elaborate_package_func(
        &self,
        pkg_name: &str,
        func_name: &str,
        args: &[Expr],
        signal_map: &HashMap<Symbol, SignalId>,
        signals: &[SignalInfo],
    ) -> Result<IrExpr, SimError> {
        let func = self
            .package_symbols
            .get(pkg_name)
            .and_then(|items| items.get(func_name))
            .and_then(|item| {
                if let PackageItem::Function(f) = item {
                    Some(f)
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                self.elab_diag(DiagCode::ModuleNotFound, format!(
                    "function '{}' not found in package '{}'",
                    func_name, pkg_name
                ))
            })?;

        // Find return expression
        let ret_expr = func
            .stmts
            .iter()
            .find_map(|s| {
                if let Stmt::Return(Some(e)) = s {
                    Some(e.clone())
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                self.elab_diag(DiagCode::ModuleNotFound, format!("function '{}::{}' has no return expression", pkg_name, func_name))
            })?;

        // Substitute formal parameters with actual arguments
        let mut result = *ret_expr;

        // First: resolve package-scoped identifiers (e.g. MuBi4True → constant value)
        let pkg_symbols = self.package_symbols.get(pkg_name);
        if let Some(items) = pkg_symbols {
            // Collect all enum member names and their values from typedefs
            let mut enum_member_values: HashMap<Symbol, Expr> = HashMap::new();
            for item in items.values() {
                if let PackageItem::Typedef(td) = item {
                    if let DataType::EnumType { members, .. } = &td.dtype {
                        for (member_name, member_expr) in members {
                            if let Some(expr) = member_expr {
                                enum_member_values.insert(*member_name, expr.clone());
                            }
                        }
                    }
                }
            }
            for (item_name, item) in items {
                if let PackageItem::Param(p) = item {
                    if let Some(expr) = &p.default {
                        result = Self::substitute_ident_in_expr(result, item_name.as_str(), expr.clone());
                    }
                }
            }
            // Substitute enum member names with their constant values
            for (member_name, member_value) in &enum_member_values {
                result = Self::substitute_ident_in_expr(result, member_name.as_str(), member_value.clone());
            }
        }

        // Then: substitute formal parameters with actual arguments
        for (i, param) in func.ports.iter().enumerate() {
            if let Some(arg) = args.get(i) {
                result = Self::substitute_ident_in_expr(result, param.name.as_str(), arg.clone());
            }
        }

        self.elaborate_expr(&result, signal_map, signals)
    }

    fn substitute_ident_in_expr(expr: Expr, target: &str, replacement: Expr) -> Expr {
        match expr {
            Expr::Ident { ref name, .. } if name == target => replacement,
            Expr::Ident { .. } => expr,
            Expr::Value(_) | Expr::String(_) | Expr::Null | Expr::FillLit(_) => expr,
            Expr::BinaryOp { op, lhs, rhs } => Expr::BinaryOp {
                op,
                lhs: Box::new(Self::substitute_ident_in_expr(
                    *lhs,
                    target,
                    replacement.clone(),
                )),
                rhs: Box::new(Self::substitute_ident_in_expr(
                    *rhs,
                    target,
                    replacement.clone(),
                )),
            },
            Expr::UnaryOp { op, expr: inner } => Expr::UnaryOp {
                op,
                expr: Box::new(Self::substitute_ident_in_expr(
                    *inner,
                    target,
                    replacement.clone(),
                )),
            },
            Expr::Paren(inner) => Expr::Paren(Box::new(Self::substitute_ident_in_expr(
                *inner,
                target,
                replacement.clone(),
            ))),
            Expr::Concat(exprs) => Expr::Concat(
                exprs
                    .into_iter()
                    .map(|e| Self::substitute_ident_in_expr(e, target, replacement.clone()))
                    .collect(),
            ),
            Expr::Replicate { count, expr: inner } => Expr::Replicate {
                count: Box::new(Self::substitute_ident_in_expr(
                    *count,
                    target,
                    replacement.clone(),
                )),
                expr: Box::new(Self::substitute_ident_in_expr(
                    *inner,
                    target,
                    replacement.clone(),
                )),
            },
            Expr::RangeSelect {
                expr: inner,
                msb,
                lsb,
            } => Expr::RangeSelect {
                expr: Box::new(Self::substitute_ident_in_expr(
                    *inner,
                    target,
                    replacement.clone(),
                )),
                msb: Box::new(Self::substitute_ident_in_expr(
                    *msb,
                    target,
                    replacement.clone(),
                )),
                lsb: Box::new(Self::substitute_ident_in_expr(
                    *lsb,
                    target,
                    replacement.clone(),
                )),
            },
            Expr::BitSelect { expr: inner, index } => Expr::BitSelect {
                expr: Box::new(Self::substitute_ident_in_expr(
                    *inner,
                    target,
                    replacement.clone(),
                )),
                index: Box::new(Self::substitute_ident_in_expr(
                    *index,
                    target,
                    replacement.clone(),
                )),
            },
            Expr::PartSelect {
                expr: inner,
                base,
                width,
            } => Expr::PartSelect {
                expr: Box::new(Self::substitute_ident_in_expr(
                    *inner,
                    target,
                    replacement.clone(),
                )),
                base: Box::new(Self::substitute_ident_in_expr(
                    *base,
                    target,
                    replacement.clone(),
                )),
                width: Box::new(Self::substitute_ident_in_expr(
                    *width,
                    target,
                    replacement.clone(),
                )),
            },
            Expr::ScopedIdent { package, item } => {
                if package == target {
                    match &replacement {
                        Expr::Ident { name, .. } => Expr::ScopedIdent {
                            package: *name,
                            item,
                        },
                        _ => Expr::ScopedIdent { package, item },
                    }
                } else {
                    Expr::ScopedIdent { package, item }
                }
            }
            Expr::Cast { dtype, expr: inner } => Expr::Cast {
                expr: Box::new(Self::substitute_ident_in_expr(
                    *inner,
                    target,
                    replacement.clone(),
                )),
                dtype,
            },
            Expr::MemberAccess { obj, field } => Expr::MemberAccess {
                obj: Box::new(Self::substitute_ident_in_expr(
                    *obj,
                    target,
                    replacement.clone(),
                )),
                field,
            },
            Expr::TernaryOp {
                cond,
                true_expr,
                false_expr,
            } => Expr::TernaryOp {
                cond: Box::new(Self::substitute_ident_in_expr(
                    *cond,
                    target,
                    replacement.clone(),
                )),
                true_expr: Box::new(Self::substitute_ident_in_expr(
                    *true_expr,
                    target,
                    replacement.clone(),
                )),
                false_expr: Box::new(Self::substitute_ident_in_expr(
                    *false_expr,
                    target,
                    replacement.clone(),
                )),
            },
            Expr::FuncCall { name: n, args: a } => Expr::FuncCall {
                name: n,
                args: a
                    .into_iter()
                    .map(|e| Self::substitute_ident_in_expr(e, target, replacement.clone()))
                    .collect(),
            },
            Expr::MethodCall {
                obj,
                method,
                args,
                with_clause,
            } => Expr::MethodCall {
                obj: Box::new(Self::substitute_ident_in_expr(
                    *obj,
                    target,
                    replacement.clone(),
                )),
                method,
                args: args
                    .into_iter()
                    .map(|e| Self::substitute_ident_in_expr(e, target, replacement.clone()))
                    .collect(),
                with_clause,
            },
            Expr::Inside {
                expr: inner,
                range_list,
            } => Expr::Inside {
                expr: Box::new(Self::substitute_ident_in_expr(
                    *inner,
                    target,
                    replacement.clone(),
                )),
                range_list: range_list
                    .into_iter()
                    .map(|e| Self::substitute_ident_in_expr(e, target, replacement.clone()))
                    .collect(),
            },
            Expr::StreamingConcat {
                op,
                slice_size,
                slices,
            } => Expr::StreamingConcat {
                op,
                slice_size: slice_size.map(|ss| {
                    Box::new(Self::substitute_ident_in_expr(
                        *ss,
                        target,
                        replacement.clone(),
                    ))
                }),
                slices: slices
                    .into_iter()
                    .map(|e| Self::substitute_ident_in_expr(e, target, replacement.clone()))
                    .collect(),
            },
            Expr::Dist { expr: inner, items } => Expr::Dist {
                expr: Box::new(Self::substitute_ident_in_expr(
                    *inner,
                    target,
                    replacement.clone(),
                )),
                items,
            },
        }
    }

    pub(crate) fn elaborate_expr_to_signal(
        &self,
        expr: &Expr,
        signal_map: &HashMap<Symbol, SignalId>,
    ) -> Result<SignalId, SimError> {
        match expr {
            Expr::Ident { name, line, col } => signal_map
                .get(name)
                .ok_or_else(|| self.elab_diag_at(DiagCode::UndefinedSignal, format!("signal '{}' not found", name), *line, *col))
                .copied(),
            Expr::MethodCall { .. } => Err(self.elab_diag(DiagCode::ModuleNotFound,
                "method calls cannot resolve to a signal",
            )),
            Expr::MemberAccess { .. } => Err(self.elab_diag(DiagCode::ModuleNotFound,
                "member access cannot resolve to a signal",
            )),
            _ => Err(self.elab_diag(DiagCode::ModuleNotFound, "expected simple signal identifier")),
        }
    }

    /// Create a signal from a port connection expression.
    /// For simple identifiers, resolves directly.
    /// For compound expressions (e.g. ~clk_i), creates an implicit wire + continuous assign.
    pub(crate) fn instance_port_expr_to_signal(
        &self,
        expr: &Expr,
        signal_map: &HashMap<Symbol, SignalId>,
        signals: &mut Vec<SignalInfo>,
        next_id: &mut SignalId,
        processes: &mut Vec<Process>,
        hint_name: &str,
    ) -> Result<SignalId, SimError> {
        // Try simple signal resolution first
        if let Ok(sid) = self.elaborate_expr_to_signal(expr, signal_map) {
            return Ok(sid);
        }
        // For compound expressions, create an implicit wire
        let ir_expr = self.elaborate_expr(expr, signal_map, signals)?;
        let width_val = compute_expr_width(
            expr,
            signal_map,
            signals,
            &self.param_vals,
            &self.package_symbols,
        ).map_err(|e| self.elab_diag(crate::diagnostics::diagnostic::DiagCode::WidthMismatch, format!("width computation failed for port '{}': {}", hint_name, e)))?;
        let width = if width_val > 0 { width_val } else { 1 };
        if width > 1_000_000 {
            eprintln!("[DBG-WIDTH] port '{}' huge width {} expr={:?} in module {}", hint_name, width, expr, self.current_module.map(|s| s.as_str()).unwrap_or("?"));
            if let Expr::Concat(items) = expr {
                for it in items {
                    let w = compute_expr_width(it, signal_map, signals, &self.param_vals, &self.package_symbols).unwrap_or(0);
                    eprintln!("[DBG-WIDTH]   item {:?} width={}", it, w);
                }
            }
        }
        // Create a unique implicit signal name
        let sig_name = format!("__port_{}", hint_name.replace('.', "_"));
        let sid = *next_id;
        *next_id += 1;
        signals.push(SignalInfo {
            name: Symbol::intern(&sig_name),
            width,
            kind: SignalKind::Wire,
            net_type: NetType::Wire,
            multi_driver: false,
            init_val: crate::ir::LogicVec::fill(crate::ir::LogicVal::Z, width),
            array_depth: 1,
            elem_width: width,
            array_dims: vec![],
            class_name: None,
            is_string: false,
            is_mailbox: false,
            is_semaphore: false,
            is_real: false,
            is_2state: false,
            is_dynamic: false,
            is_queue: false,
            is_associative: false,
            is_signed: false,
            is_const: false,
            msb: width - 1,
            lsb: 0,
            struct_fields: vec![],
            packed_dims: vec![],
            delay_rise: None,
            delay_fall: None,
            iface_type: None,
            iface_modport: None,
        });
        // Add a continuous assignment process
        let sensitivity = collect_sensitivity(expr, signal_map);
        processes.push(Process::Combinational {
            name: Symbol::intern(&format!("port_assign_{}", hint_name.replace('.', "_"))),
            sensitivity,
            body: vec![IrStmt::BlockingAssign {
                lhs: IrLValue::Signal(sid, 0),
                rhs: ir_expr,
                delay: None,
            }],
        });
        Ok(sid)
    }

    pub(crate) fn resolve_typedef_width(&self, dtype: &DataType, range: Option<&ExprRange>) -> usize {
        if let Some(er) = range {
            if let (Ok(msb), Ok(lsb)) = (const_eval_simple(&er.msb), const_eval_simple(&er.lsb)) {
                return if msb >= lsb {
                    (msb - lsb + 1) as usize
                } else {
                    (lsb - msb + 1) as usize
                };
            }
        }
        match dtype {
            DataType::UserDefined(name) => self.typedef_map.get(name).copied().unwrap_or(64),
            DataType::Signed(inner) => self.resolve_typedef_width(inner, None),
            _ => dtype.width(),
        }
    }
}

// CATATAN: impl DataType { fn width() } dan impl DeclKind { fn default_width() }
// sudah dipindahkan ke src/ast/types.rs karena method-method ini adalah
// bagian dari definisi tipe AST, bukan tanggung jawab elaborator.
// Lihat src/ast/types.rs untuk implementasinya.
//
// CATATAN: parse_type_spec_str() sudah dipindahkan ke src/elaboration/util/type_util.rs
// dan di-re-export via util/mod.rs.





