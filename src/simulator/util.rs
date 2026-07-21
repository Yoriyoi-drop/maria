use crate::ast::*;
use crate::ir::*;
use crate::simulator::state::SimulationState;
use crate::error::SimError;
use std::collections::HashMap;

pub fn map_ast_binary_op(op: &BinaryOp) -> Result<BinaryIrOp, String> {
    match op {
        BinaryOp::Add => Ok(BinaryIrOp::Add),
        BinaryOp::Sub => Ok(BinaryIrOp::Sub),
        BinaryOp::Mul => Ok(BinaryIrOp::Mul),
        BinaryOp::Div => Ok(BinaryIrOp::Div),
        BinaryOp::Mod => Ok(BinaryIrOp::Mod),
        BinaryOp::Power => Ok(BinaryIrOp::Power),
        BinaryOp::Eq => Ok(BinaryIrOp::Eq),
        BinaryOp::Neq => Ok(BinaryIrOp::Neq),
        BinaryOp::CaseEq => Ok(BinaryIrOp::CaseEq),
        BinaryOp::CaseNeq => Ok(BinaryIrOp::CaseNeq),
        BinaryOp::EqWild => Ok(BinaryIrOp::Eq),
        BinaryOp::NeqWild => Ok(BinaryIrOp::Neq),
        BinaryOp::Lt => Ok(BinaryIrOp::Lt),
        BinaryOp::Le => Ok(BinaryIrOp::Le),
        BinaryOp::Gt => Ok(BinaryIrOp::Gt),
        BinaryOp::Ge => Ok(BinaryIrOp::Ge),
        BinaryOp::BitAnd => Ok(BinaryIrOp::BitAnd),
        BinaryOp::BitOr => Ok(BinaryIrOp::BitOr),
        BinaryOp::BitXor => Ok(BinaryIrOp::BitXor),
        BinaryOp::BitXnor => Ok(BinaryIrOp::BitXnor),
        BinaryOp::Shl => Ok(BinaryIrOp::Shl),
        BinaryOp::Shr => Ok(BinaryIrOp::Shr),
        BinaryOp::Sshl => Ok(BinaryIrOp::Sshl),
        BinaryOp::Sshr => Ok(BinaryIrOp::Sshr),
        BinaryOp::LogicalAnd => Ok(BinaryIrOp::LogicalAnd),
        BinaryOp::LogicalOr => Ok(BinaryIrOp::LogicalOr),
    }
}

pub fn map_ast_unary_op(op: &UnaryOp) -> Result<UnaryIrOp, String> {
    match op {
        UnaryOp::Plus => Ok(UnaryIrOp::Plus),
        UnaryOp::Minus => Ok(UnaryIrOp::Minus),
        UnaryOp::BitNot => Ok(UnaryIrOp::BitNot),
        UnaryOp::Not => Ok(UnaryIrOp::Not),
        UnaryOp::ReductionAnd => Ok(UnaryIrOp::RedAnd),
        UnaryOp::ReductionNand => Ok(UnaryIrOp::RedNand),
        UnaryOp::ReductionOr => Ok(UnaryIrOp::RedOr),
        UnaryOp::ReductionNor => Ok(UnaryIrOp::RedNor),
        UnaryOp::ReductionXor => Ok(UnaryIrOp::RedXor),
        UnaryOp::ReductionXnor => Ok(UnaryIrOp::RedXnor),
    }
}

pub fn extract_signal_deps(expr: &IrExpr) -> Vec<SignalId> {
    let mut deps = Vec::new();
    extract_signal_deps_inner(expr, &mut deps);
    deps
}

pub fn extract_signal_deps_inner(expr: &IrExpr, deps: &mut Vec<SignalId>) {
    match expr {
        IrExpr::Signal(id, _) => {
            if !deps.contains(id) {
                deps.push(*id);
            }
        }
        IrExpr::RangeSelect(id, _, _) | IrExpr::BitSelect(id, _) | IrExpr::ArrayIndex { sig_id: id, .. } => {
            if !deps.contains(id) {
                deps.push(*id);
            }
        }
        IrExpr::ExprRangeSelect(e, _, _) | IrExpr::ExprBitSelect(e, _) => {
            extract_signal_deps_inner(e, deps);
        }
        IrExpr::ExprPartSelect(e1, e2, e3) => {
            extract_signal_deps_inner(e1, deps);
            extract_signal_deps_inner(e2, deps);
            extract_signal_deps_inner(e3, deps);
        }
        IrExpr::Concat(exprs) => {
            for e in exprs {
                extract_signal_deps_inner(e, deps);
            }
        }
        IrExpr::Replicate(_, e) => {
            extract_signal_deps_inner(e, deps);
        }
        IrExpr::UnaryOp(_, e) => {
            extract_signal_deps_inner(e, deps);
        }
        IrExpr::BinaryOp(_, e1, e2) => {
            extract_signal_deps_inner(e1, deps);
            extract_signal_deps_inner(e2, deps);
        }
        IrExpr::Cond(c, t, e) => {
            extract_signal_deps_inner(c, deps);
            extract_signal_deps_inner(t, deps);
            extract_signal_deps_inner(e, deps);
        }
        IrExpr::Signed(e) => {
            extract_signal_deps_inner(e, deps);
        }
        IrExpr::MethodCall { obj, args, .. } => {
            extract_signal_deps_inner(obj, deps);
            for a in args {
                extract_signal_deps_inner(a, deps);
            }
        }
        IrExpr::MemberAccess { obj, .. } => {
            extract_signal_deps_inner(obj, deps);
        }
        IrExpr::NewCall { args, .. } => {
            for a in args {
                extract_signal_deps_inner(a, deps);
            }
        }
        IrExpr::SysFunc { args, .. } => {
            for a in args {
                extract_signal_deps_inner(a, deps);
            }
        }
        IrExpr::DpiCall { args, .. } => {
            for a in args {
                extract_signal_deps_inner(a, deps);
            }
        }
        IrExpr::HierRef(_) => {}
        IrExpr::Inside { expr, list } => {
            extract_signal_deps_inner(expr, deps);
            for item in list {
                extract_signal_deps_inner(item, deps);
            }
        }
        IrExpr::Cast { expr, .. } => {
            extract_signal_deps_inner(expr, deps);
        }
        IrExpr::Dist { expr, .. } => {
            extract_signal_deps_inner(expr, deps);
        }
        IrExpr::StreamingConcat { slices, .. } => {
            for e in slices {
                extract_signal_deps_inner(e, deps);
            }
        }
        IrExpr::UdpLookup { args, .. } => {
            for a in args {
                extract_signal_deps_inner(a, deps);
            }
        }
        IrExpr::VifBinding { .. } => {}
        IrExpr::VirtualIfaceAccess { .. } => {}
        IrExpr::FuncCall { args, .. } => {
            for a in args {
                extract_signal_deps_inner(a, deps);
            }
        }
        IrExpr::Const(_) | IrExpr::FillLit(_) | IrExpr::String(_) | IrExpr::This => {}
    }
}

pub fn is_signed_expr(expr: &IrExpr, signals: &[SignalInfo]) -> bool {
    match expr {
        IrExpr::Signed(_) => true,
        IrExpr::Signal(id, _) | IrExpr::BitSelect(id, _) | IrExpr::RangeSelect(id, ..) => {
            signals.get(*id).map(|s| s.is_signed).unwrap_or(false)
        }
        IrExpr::ArrayIndex { sig_id, .. } => {
            signals.get(*sig_id).map(|s| s.is_signed).unwrap_or(false)
        }
        _ => false,
    }
}

// ─── Display formatting ────────────────────────────────────────────────────

pub fn logicvec_to_string(lv: &LogicVec) -> String {
    let mut s = String::new();
    let mut i = 0;
    while i + 7 < lv.width {
        let mut byte = 0u8;
        for j in 0..8 {
            if lv.bits[i + j] == LogicVal::One {
                byte |= 1 << j;
            }
        }
        s.push(byte as char);
        i += 8;
    }
    // Remaining bits (last partial byte)
    if i < lv.width {
        let mut byte = 0u8;
        for j in 0..(lv.width - i) {
            if lv.bits[i + j] == LogicVal::One {
                byte |= 1 << j;
            }
        }
        if byte != 0 {
            s.push(byte as char);
        }
    }
    s
}

pub fn eval_display_arg(
    state: &SimulationState,
    signals: &[SignalInfo],
    hier_map: &HashMap<String, SignalId>,
    assoc_data: &HashMap<SignalId, HashMap<LogicVec, LogicVec>>,
    arg: &IrExpr,
) -> Result<LogicVec, SimError> {
    match arg {
        IrExpr::HierRef(name) => {
            // Resolve hierarchical name via hier_map
            if let Some(&hid) = hier_map.get(name) {
                Ok(state.read_signal(hid).clone())
            } else {
                // Try finding direct signal match
                if let Some((idx, _)) = signals.iter().enumerate().find(|(_, s)| s.name == *name) {
                    Ok(state.read_signal(idx).clone())
                } else {
                    Ok(LogicVec::from_u64(0, 32))
                }
            }
        }
        IrExpr::Const(v) => Ok(v.clone()),
        IrExpr::String(s) => Ok(LogicVec {
            bits: s.bytes().flat_map(|b| (0..8).map(move |i| if (b >> i) & 1 == 1 { LogicVal::One } else { LogicVal::Zero })).collect(),
            width: s.len() * 8,
        }),
        IrExpr::Signal(id, _) | IrExpr::RangeSelect(id, _, _) | IrExpr::BitSelect(id, _) | IrExpr::ArrayIndex { sig_id: id, .. } => {
            return match arg {
                IrExpr::RangeSelect(id, msb, lsb) => {
                    let val = state.read_signal(*id);
                    let (start, end) = if msb > lsb { (*lsb, *msb) } else { (*msb, *lsb) };
                    let mut bits = val.bits[start..=end.min(val.width - 1)].to_vec();
                    if msb > lsb { bits.reverse(); }
                    Ok(LogicVec { width: bits.len(), bits })
                }
                IrExpr::BitSelect(id, idx) => {
                    let val = state.read_signal(*id);
                    let bit = val.bits.get(*idx).copied().unwrap_or(LogicVal::X);
                    Ok(LogicVec { width: 1, bits: vec![bit] })
                }
                IrExpr::ArrayIndex { sig_id, index, elem_width } => {
                    let idx_val = eval_display_arg(state, signals, hier_map, assoc_data, index)?;
                    let idx_u64 = idx_val.to_u64() as usize;
                    let val = state.read_signal(*sig_id);
                    if let Some(assoc) = assoc_data.get(sig_id) {
                        if let Some(v) = assoc.get(&idx_val) {
                            return Ok(v.clone());
                        }
                    }
                    let start = idx_u64 * elem_width;
                    let mut bits = Vec::with_capacity(*elem_width);
                    for i in start..start + elem_width {
                        bits.push(val.bits.get(i).copied().unwrap_or(LogicVal::X));
                    }
                    Ok(LogicVec { width: *elem_width, bits })
                }
                _ => {
                    let val = state.read_signal(*id);
                    Ok(val.clone())
                }
            }
        }
        IrExpr::SysFunc { name, args } if name == "sformatf" => {
            let msg = format_display(state, signals, hier_map, assoc_data, args);
            Ok(string_to_logicvec(&msg))
        }
        _ => Ok(LogicVec::from_u64(0, 32)),
    }
}

pub fn format_display(
    state: &SimulationState,
    signals: &[SignalInfo],
    hier_map: &HashMap<String, SignalId>,
    assoc_data: &HashMap<SignalId, HashMap<LogicVec, LogicVec>>,
    ir_args: &[IrExpr],
) -> String {
    let (fmt_str, start_idx) = if let Some(IrExpr::String(s)) = ir_args.first() {
        (s.clone(), 1)
    } else {
        let mut parts = Vec::new();
        for arg in ir_args {
            if let Ok(val) = eval_display_arg(state, signals, hier_map, assoc_data, arg) {
                parts.push(format!("{}", val));
            }
        }
        return parts.join(" ");
    };

    let value_args: Vec<LogicVec> = ir_args[start_idx..].iter()
        .filter_map(|a| eval_display_arg(state, signals, hier_map, assoc_data, a).ok())
        .collect();

    let mut value_idx = 0usize;
    let mut result = String::new();
    let mut chars = fmt_str.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let mut zero_fill = false;
            let mut width = 0usize;
            if let Some(&next) = chars.peek() {
                if next == '0' {
                    zero_fill = true;
                    chars.next();
                }
                while let Some(&next) = chars.peek() {
                    if next.is_ascii_digit() {
                        width = width * 10 + next.to_digit(10).unwrap() as usize;
                        chars.next();
                    } else {
                        break;
                    }
                }
            }
            match chars.next() {
                Some('d') => {
                    if let Some(val) = value_args.get(value_idx) {
                        let s = format!("{}", val.to_u64());
                        if width > s.len() {
                            let pad = if zero_fill { '0' } else { ' ' };
                            for _ in 0..(width - s.len()) { result.push(pad); }
                        }
                        result.push_str(&s);
                    }
                    value_idx += 1;
                }
                Some('b') => {
                    if let Some(val) = value_args.get(value_idx) {
                        let s = format!("{}", val);
                        let trimmed = s.trim_start_matches('0');
                        let s = if trimmed.is_empty() { "0" } else { trimmed };
                        if width > s.len() {
                            let pad = if zero_fill { '0' } else { ' ' };
                            for _ in 0..(width - s.len()) { result.push(pad); }
                        }
                        result.push_str(s);
                    }
                    value_idx += 1;
                }
                Some('h') => {
                    if let Some(val) = value_args.get(value_idx) {
                        let s = format!("{:x}", val.to_u64());
                        if width > s.len() {
                            let pad = if zero_fill { '0' } else { ' ' };
                            for _ in 0..(width - s.len()) { result.push(pad); }
                        }
                        result.push_str(&s);
                    }
                    value_idx += 1;
                }
                Some('f') => {
                    if let Some(val) = value_args.get(value_idx) {
                        let s = format!("{}", f64::from_bits(val.to_u64()));
                        result.push_str(&s);
                    }
                    value_idx += 1;
                }
                Some('s') => {
                    if let Some(val) = value_args.get(value_idx) {
                        result.push_str(&logicvec_to_string(val));
                    }
                    value_idx += 1;
                }
                Some(c2) => {
                    result.push('%');
                    if zero_fill { result.push('0'); }
                    if width > 0 { result.push_str(&format!("{}", width)); }
                    result.push(c2);
                }
                None => {
                    result.push('%');
                }
            }
        } else if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some(c2) => { result.push('\\'); result.push(c2); }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

pub fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

// ─── Signal utilities ───────────────────────────────────────────────────

pub fn signal_is_2state(signals: &[SignalInfo], id: SignalId) -> bool {
    signals.get(id).map(|s| s.is_2state).unwrap_or(false)
}

pub fn sanitize_for_2state(signals: &[SignalInfo], id: SignalId, val: &mut LogicVec) {
    if !signal_is_2state(signals, id) { return; }
    for bit in val.bits.iter_mut() {
        if *bit == LogicVal::X || *bit == LogicVal::Z {
            *bit = LogicVal::Zero;
        }
    }
}

pub fn resolve_net_values(net_type: NetType, current: &LogicVec, incoming: &LogicVec) -> LogicVec {
    let width = current.width.max(incoming.width);
    let mut bits = Vec::with_capacity(width);
    for i in 0..width {
        let cur = current.bits.get(i).copied().unwrap_or(LogicVal::Z);
        let inc = incoming.bits.get(i).copied().unwrap_or(LogicVal::Z);
        bits.push(net_type.resolve_bit(cur, inc));
    }
    LogicVec { bits, width }
}

pub fn read_hex_file(filename: &str, elem_width: usize, array_depth: usize, start: Option<usize>, end: Option<usize>) -> Result<Vec<LogicVec>, SimError> {
    let content = std::fs::read_to_string(filename).map_err(|e| SimError::waveform(format!("cannot read {}: {}", filename, e)))?;
    let start_addr = start.unwrap_or(0);
    let end_addr = end.unwrap_or(array_depth - 1);
    let len = end_addr - start_addr + 1;
    let mut data = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') { continue; }
        let val = i64::from_str_radix(line, 16).map_err(|e| SimError::waveform(format!("bad hex value '{}': {}", line, e)))?;
        data.push(LogicVec::from_u64(val as u64, elem_width));
        if data.len() >= len { break; }
    }
    Ok(data)
}

pub fn read_bin_file(filename: &str, elem_width: usize, array_depth: usize, start: Option<usize>, end: Option<usize>) -> Result<Vec<LogicVec>, SimError> {
    let content = std::fs::read_to_string(filename).map_err(|e| SimError::waveform(format!("cannot read {}: {}", filename, e)))?;
    let start_addr = start.unwrap_or(0);
    let end_addr = end.unwrap_or(array_depth - 1);
    let len = end_addr - start_addr + 1;
    let mut data = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') { continue; }
        let val = i64::from_str_radix(line, 2).map_err(|e| SimError::waveform(format!("bad binary value '{}': {}", line, e)))?;
        data.push(LogicVec::from_u64(val as u64, elem_width));
        if data.len() >= len { break; }
    }
    Ok(data)
}

pub fn string_to_logicvec(s: &str) -> LogicVec {
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
