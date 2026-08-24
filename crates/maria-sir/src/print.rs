//! Render SIR ke teks terbaca manusia (`maria synth --dump-sir`).
//!
//! Format deterministik (bisa di-diff): port → register → node → wire,
//! dengan resolusi nama nilai (value → port/wire/const/reg).

use std::fmt::Write;

use crate::sir::*;

/// Render seluruh modul SIR.
pub fn render_sir(m: &SirModule) -> String {
    let mut s = String::new();
    writeln!(s, "── SIR module: {} ──", m.name.as_str()).unwrap();
    writeln!(s, "  ports  in={} out={}", m.inputs.len(), m.outputs.len()).unwrap();
    writeln!(s, "  nodes  {}", m.nodes.len()).unwrap();
    writeln!(s, "  regs   {}", m.registers.len()).unwrap();

    writeln!(s, "\nPorts:").unwrap();
    for p in &m.inputs {
        writeln!(
            s,
            "  {:<12} {:<5} [{}:0]",
            p.name.as_str(),
            p.dir.name(),
            p.width.saturating_sub(1)
        )
        .unwrap();
    }
    for p in &m.outputs {
        writeln!(
            s,
            "  {:<12} {:<5} [{}:0]  (driven by {})",
            p.name.as_str(),
            p.dir.name(),
            p.width.saturating_sub(1),
            value_label(m, p.value)
        )
        .unwrap();
    }

    writeln!(s, "\nRegisters:").unwrap();
    for (i, r) in m.registers.iter().enumerate() {
        let reset_txt = match &r.reset {
            Some(rs) => format!(
                "rst={}({:#x},{},{})",
                value_label(m, rs.signal),
                rs.value.to_u64(),
                if rs.polarity { "high" } else { "low" },
                if rs.r#async { "async" } else { "sync" }
            ),
            None => "rst=-".into(),
        };
        let en_txt = match r.enable {
            Some(e) => format!("ce={}", value_label(m, e)),
            None => "ce=-".into(),
        };
        writeln!(
            s,
            "  r{i}  {:<10} [{}:0]  d={}  q={}  clk={}  {}  {}",
            r.name.as_str(),
            r.width.saturating_sub(1),
            value_label(m, r.d),
            value_label(m, r.q),
            value_label(m, r.clock),
            reset_txt,
            en_txt
        )
        .unwrap();
    }

    writeln!(s, "\nNodes:").unwrap();
    for (i, n) in m.nodes.iter().enumerate() {
        let args: Vec<String> = n.inputs.iter().map(|v| value_label(m, *v)).collect();
        match &n.kind {
            SirNodeKind::Slice { msb, lsb } => {
                writeln!(
                    s,
                    "  n{i}  {:<8} [{msb}:{lsb}] <- {}",
                    n.kind.name(),
                    args.join(", ")
                )
                .unwrap();
            }
            _ => {
                writeln!(
                    s,
                    "  n{i}  {:<8} [{}:0] <- {}",
                    n.kind.name(),
                    n.width.saturating_sub(1),
                    args.join(", ")
                )
                .unwrap();
            }
        }
    }

    if !m.wires.is_empty() {
        writeln!(s, "\nWires:").unwrap();
        for w in &m.wires {
            let flags = [
                (w.is_clock, "clock"),
                (w.is_reset, "reset"),
                (w.is_io, "io"),
            ]
            .iter()
            .filter(|(f, _)| *f)
            .map(|(_, n)| *n)
            .collect::<Vec<_>>()
            .join(",");
            writeln!(
                s,
                "  {:<12} [{}:0] = {}  ({})",
                w.name.as_str(),
                w.width.saturating_sub(1),
                value_label(m, w.value),
                flags
            )
            .unwrap();
        }
    }

    s
}

/// Nama untuk sebuah nilai (resolusi tabel nilai).
pub fn value_label(m: &SirModule, v: ValueId) -> String {
    match m.values.get(v) {
        Some(SirValue::Port(p)) => {
            let name = if *p < m.inputs.len() {
                m.inputs[*p].name.as_str().to_string()
            } else {
                m.outputs[p - m.inputs.len()].name.as_str().to_string()
            };
            format!("{}", name)
        }
        Some(SirValue::Const(lv)) => format!("{}'h{:x}", lv.width, lv.to_u64()),
        Some(SirValue::Node(n)) => format!("n{}", n),
        Some(SirValue::Reg(r)) => format!("q({})", m.registers[*r].name.as_str()),
        None => "<?>".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maria_core::{intern::Symbol, LogicVec};

    fn sample() -> SirModule {
        // Bangun langsung (tanpa lower) untuk tes render murni.
        let mut m = SirModule::new(Symbol::intern("top"));
        let _ = m.add_value(SirValue::Const(LogicVec::from_u64(99, 8)));
        let _ = m.add_value(SirValue::Const(LogicVec::from_u64(0, 1)));
        m
    }

    #[test]
    fn render_is_deterministic_and_readable() {
        let m = sample();
        let s1 = render_sir(&m);
        let s2 = render_sir(&m);
        assert_eq!(s1, s2, "dump harus deterministik");
        assert!(s1.contains("SIR module: top"));
    }

    #[test]
    fn render_after_lower_counter() {
        // Pakai lowering nyata: dump counter 8-bit.
        let ir = lower_tests::counter_ir();
        let out = crate::lower::lower(&ir);
        let s = render_sir(&out.module);
        assert!(
            s.contains("q(count)"),
            "dump harus memuat nilai register: {s}"
        );
        assert!(s.contains("ADD"), "dump harus memuat node ADD: {s}");
        assert!(s.contains("MUX"), "dump harus memuat node MUX: {s}");
    }
}

/// Design IR untuk tes render (salinan dari lower.rs test helper).
#[cfg(test)]
mod lower_tests {
    use maria_core::intern::Symbol;
    use maria_core::LogicVec;
    use maria_ir::{
        BinaryIrOp, ClockEdge, IrDesign, IrExpr, IrLValue, IrStmt, Process, ResetInfo, SignalKind,
    };

    pub fn counter_ir() -> IrDesign {
        let mut ir = IrDesign {
            top: maria_ir::IrModule {
                name: Symbol::intern("counter"),
                ..Default::default()
            },
            modules: Default::default(),
            classes: Default::default(),
            covergroups: Vec::new(),
            dpi_imports: Vec::new(),
            hier_signal_map: Default::default(),
            udp_defs: Vec::new(),
            specify_items: Vec::new(),
            timescale: None,
            module_functions: Default::default(),
            source_lines: None,
            source_file: None,
            pkg_scoped_consts: Default::default(),
            coverage_exclusions: Vec::new(),
            stmt_lines: std::collections::HashMap::new(),
            net_aliases: std::collections::HashMap::new(),
        };
        use maria_ir::SignalInfo;
        let sig = |name: &str, width: usize, kind: SignalKind| SignalInfo {
            name: Symbol::intern(name),
            width,
            kind,
            ..Default::default()
        };
        ir.top.signals.push(sig("clk", 1, SignalKind::Input));
        ir.top.signals.push(sig("rst_n", 1, SignalKind::Input));
        ir.top.signals.push(sig("count", 8, SignalKind::Output));
        ir.top.inputs = vec![0, 1];
        ir.top.outputs = vec![2];
        let rhs = IrExpr::Cond(
            Box::new(IrExpr::BinaryOp(
                BinaryIrOp::Eq,
                Box::new(IrExpr::Signal(2, 8)),
                Box::new(IrExpr::Const(LogicVec::from_u64(99, 8))),
            )),
            Box::new(IrExpr::Const(LogicVec::from_u64(0, 8))),
            Box::new(IrExpr::BinaryOp(
                BinaryIrOp::Add,
                Box::new(IrExpr::Signal(2, 8)),
                Box::new(IrExpr::Const(LogicVec::from_u64(1, 8))),
            )),
        );
        ir.top.processes.push(Process::Sequential {
            name: Symbol::intern("ff_count"),
            clock: ClockEdge::PosEdge(0),
            reset: Some(ResetInfo {
                signal: 1,
                polarity: false,
                r#async: true,
                value: LogicVec::from_u64(0, 8),
            }),
            body: vec![IrStmt::NonBlockingAssign {
                lhs: IrLValue::Signal(2, 8),
                rhs,
                delay: None,
            }],
            iff: None,
        });
        ir
    }
}
