//! Static Timing Analysis (SYNTHESIS.md §16 — phase 5).
//!
//! Model delay deterministik & transparan (agar bisa dihitung ulang manual):
//!
//! ```text
//! cell_delay(kind) = base(kind) + 0.05 × fanout(output)
//!   LUT6            0.30 ns
//!   CARRY4          0.05 × width ns   (carry ripple per bit)
//!   Mux             0.12 ns
//!   Add/Sub/Mul/... 0.20 × ceil(width/4) ns   (netlist generik)
//!   lainnya         0.10 ns
//!   FF clk→q        0.50 ns  (startpoint, bukan transisi)
//! setup FF          0.20 ns  (TimingOptions)
//! ```
//!
//! Arrival di-propagasi dari startpoint (input port = `input_delay`, FF-Q =
//! `clk_to_q`, konstanta = 0) ke endpoint (FF-D, output port). Untuk tiap
//! endpoint:
//!
//! ```text
//! required = period × multicycle − setup          (FF-D)
//!          = period − output_delay                (output port)
//! slack    = required − arrival
//! WNS = min slack   TNS = Σ min(slack, 0)
//! ```
//!
//! False path (`constraint.false_paths`) di-skip dari WNS/TNS. Critical path
//! di-trace backward dari endpoint slack terkecil.

use maria_core::intern::Symbol;
use maria_netlist::cell::{CellInstance, CellKind};
use maria_netlist::net::{Netlist, PortDir};

use crate::constraint::Constraint;

/// Opsi analysis (delay FF).
#[derive(Debug, Clone)]
pub struct TimingOptions {
    /// Setup time FF (ns).
    pub setup_ns: f64,
    /// Clock-to-Q FF (ns).
    pub clk_to_q_ns: f64,
}

impl Default for TimingOptions {
    fn default() -> Self {
        TimingOptions {
            setup_ns: 0.2,
            clk_to_q_ns: 0.5,
        }
    }
}

/// Satu endpoint (FF-D atau output port).
#[derive(Debug, Clone)]
pub struct Endpoint {
    /// Nama signal tujuan (reg / port output).
    pub name: String,
    /// Startpoint jalur (nama net asal).
    pub from: String,
    /// arrival (ns).
    pub arrival_ns: f64,
    /// required (ns).
    pub required_ns: f64,
    /// slack (ns).
    pub slack_ns: f64,
    /// False path — di-skip dari WNS/TNS.
    pub false_path: bool,
    /// "ff" | "out".
    pub kind: &'static str,
}

/// Satu jalur (list cell dari startpoint → endpoint).
#[derive(Debug, Clone)]
pub struct Path {
    pub from: String,
    pub to: String,
    pub delay_ns: f64,
    pub cells: Vec<String>,
}

/// Hasil STA.
#[derive(Debug, Clone)]
pub struct TimingReport {
    pub wns_ns: f64,
    pub tns_ns: f64,
    pub period_ns: f64,
    pub endpoints: Vec<Endpoint>,
    /// Critical path terburuk (≤ 5).
    pub critical_paths: Vec<Path>,
    pub max_fanout: usize,
}

/// Delay sel (ns) — model deterministik, lihat doc modul.
fn cell_delay(c: &CellInstance, fanout: usize) -> f64 {
    let base = match &c.kind {
        CellKind::Lut { .. } => 0.30,
        CellKind::Carry4 => 0.05 * c.width.max(1) as f64,
        CellKind::Mux => 0.12,
        CellKind::Add
        | CellKind::Sub
        | CellKind::Mul
        | CellKind::Div
        | CellKind::Mod
        | CellKind::Shl
        | CellKind::Shr
        | CellKind::Sar => 0.20 * (c.width.max(1) as f64 / 4.0).ceil(),
        // FF bukan transisi (Q = startpoint) — delay 0 agar tak double-hitung.
        CellKind::Dff | CellKind::DffE | CellKind::DffR { .. } | CellKind::DffRE { .. } => 0.0,
        _ => 0.10,
    };
    base + 0.05 * fanout as f64
}

/// STA penuh atas netlist dengan constraint.
pub fn analyze(nl: &Netlist, c: &Constraint, opts: &TimingOptions) -> TimingReport {
    let n = nl.nets.len();
    let mut arrival = vec![f64::NEG_INFINITY; n];

    // ── Startpoint ──
    // Input port: arrival = input_delay. Konstanta: 0. FF-Q: clk_to_q.
    let port_dir: Vec<(Symbol, PortDir)> =
        nl.ports.iter().map(|p| (p.name, p.dir.clone())).collect();
    for (id, net) in nl.nets.iter().enumerate() {
        let is_input_port = port_dir
            .iter()
            .any(|(name, dir)| *name == net.name && matches!(dir, PortDir::Input | PortDir::Inout));
        if is_input_port {
            arrival[id] = c.input_delay_ns;
        } else if net.const_value.is_some() {
            arrival[id] = 0.0;
        }
    }
    for cell in &nl.cells {
        if cell.kind.is_sequential() {
            for o in &cell.outputs {
                arrival[o.net] = opts.clk_to_q_ns;
            }
        }
    }

    // ── Propagate arrival (fixed-point — DAG) ──
    for _ in 0..=nl.cells.len() {
        let mut changed = false;
        for c in &nl.cells {
            if c.kind.is_sequential() {
                continue; // Q = startpoint; D = endpoint (di bawah)
            }
            let mut in_max = f64::NEG_INFINITY;
            for pin in &c.inputs {
                let a = arrival[pin.net];
                if a > in_max {
                    in_max = a;
                }
            }
            if !in_max.is_finite() {
                continue; // input belum siap (menunggu driver lain)
            }
            let fanout = nl.nets[c.outputs.first().map(|o| o.net).unwrap_or(0)]
                .loads
                .len();
            let out = in_max + cell_delay(c, fanout);
            for o in &c.outputs {
                if out > arrival[o.net] {
                    arrival[o.net] = out;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    // ── Endpoint ──
    let period = c.period_ns();
    let mut endpoints: Vec<Endpoint> = Vec::new();

    // FF-D: untuk tiap FF, pin "d" (data). from = nama net driver jalur.
    for cell in &nl.cells {
        if !cell.kind.is_sequential() {
            continue;
        }
        for pin in &cell.inputs {
            if pin.pin != "d" {
                continue;
            }
            let arr = arrival[pin.net];
            let from = nl.nets[pin.net].name.as_str().to_string();
            let to = cell.name.as_str().to_string();
            let cycles = c.cycle_multiplier(&from, &to).max(1);
            let required = period * cycles as f64 - opts.setup_ns;
            let fp = c.is_false_path(&from, &to);
            endpoints.push(Endpoint {
                name: to,
                from,
                arrival_ns: arr,
                required_ns: required,
                slack_ns: required - arr,
                false_path: fp,
                kind: "ff",
            });
        }
    }
    // Output port: required = period − output_delay.
    for (id, net) in nl.nets.iter().enumerate() {
        let is_output_port = port_dir.iter().any(|(name, dir)| {
            *name == net.name && matches!(dir, PortDir::Output | PortDir::Inout)
        });
        if !is_output_port {
            continue;
        }
        let arr = arrival[id];
        let from = net
            .driver
            .as_ref()
            .map(|_d| nl.nets[id].name.as_str().to_string())
            .unwrap_or_else(|| net.name.as_str().to_string());
        let required = period - c.output_delay_ns;
        let fp = c.is_false_path(&from, net.name.as_str());
        endpoints.push(Endpoint {
            name: net.name.as_str().to_string(),
            from,
            arrival_ns: arr,
            required_ns: required,
            slack_ns: required - arr,
            false_path: fp,
            kind: "out",
        });
    }

    // ── WNS / TNS (false path di-skip) ──
    let mut wns = f64::INFINITY;
    let mut tns = 0.0;
    for e in &endpoints {
        if e.false_path {
            continue;
        }
        if e.slack_ns < wns {
            wns = e.slack_ns;
        }
        if e.slack_ns < 0.0 {
            tns += e.slack_ns;
        }
    }
    if wns.is_infinite() {
        wns = 0.0; // tidak ada endpoint
    }

    // ── Critical path (trace backward dari endpoint slack terkecil) ──
    let mut critical_paths: Vec<Path> = Vec::new();
    let mut sorted: Vec<&Endpoint> = endpoints.iter().filter(|e| !e.false_path).collect();
    sorted.sort_by(|a, b| a.slack_ns.total_cmp(&b.slack_ns));
    for e in sorted.iter().take(5) {
        let mut cells = Vec::new();
        // Net tujuan endpoint: cari net yang namanya == endpoint.name
        // (FF D / output port). FF: cell.name; out: port net.
        let end_net = nl
            .nets
            .iter()
            .position(|net| net.name.as_str() == e.name)
            .or_else(|| {
                // FF D: cari net yang di-drive cell... endpoint ini FF → cari
                // input pin "d" pada cell bernama e.name.
                nl.cells
                    .iter()
                    .find(|c| c.name.as_str() == e.name)
                    .and_then(|c| c.inputs.iter().find(|p| p.pin == "d").map(|p| p.net))
            });
        let mut net = end_net;
        let mut steps = 0;
        while let Some(nid) = net {
            if steps > 64 {
                break;
            }
            let driver = nl.nets[nid].driver.clone();
            match driver {
                Some(d) => {
                    let c = &nl.cells[d.cell];
                    cells.push(c.name.as_str().to_string());
                    if c.kind.is_sequential() {
                        break; // FF-Q = startpoint
                    }
                    // Pilih input pin dengan arrival terbesar.
                    let best = c
                        .inputs
                        .iter()
                        .max_by(|a, b| arrival[a.net].total_cmp(&arrival[b.net]))
                        .map(|p| p.net);
                    match best {
                        Some(p) => {
                            if p == nid {
                                break; // guard cycle (seharusnya tak terjadi)
                            }
                            net = Some(p);
                        }
                        None => break,
                    }
                    steps += 1;
                }
                None => break, // startpoint: port input / konstanta
            }
        }
        let from = cells
            .last()
            .map(|c| {
                // nama net yang di-drive sel terakhir = startpoint
                nl.cells
                    .iter()
                    .find(|cc| cc.name.as_str() == c)
                    .and_then(|cc| cc.outputs.first())
                    .map(|o| nl.nets[o.net].name.as_str().to_string())
                    .unwrap_or_else(|| c.clone())
            })
            .unwrap_or_else(|| e.from.clone());
        critical_paths.push(Path {
            from,
            to: e.name.clone(),
            delay_ns: e.arrival_ns,
            cells,
        });
    }

    let max_fanout = nl.nets.iter().map(|net| net.loads.len()).max().unwrap_or(0);
    TimingReport {
        wns_ns: wns,
        tns_ns: tns,
        period_ns: period,
        endpoints,
        critical_paths,
        max_fanout,
    }
}

/// Render report teks (timing.rpt / stdout).
pub fn render_timing_report(r: &TimingReport, constraint_name: &str) -> String {
    let mut s = String::new();
    s.push_str("── Timing Report (STA)\n");
    s.push_str(&format!("  constraint   {}\n", constraint_name));
    s.push_str(&format!("  period       {:.2} ns\n", r.period_ns));
    s.push_str(&format!(
        "  WNS          {}{:.2} ns\n",
        if r.wns_ns < 0.0 { "" } else { "+" },
        r.wns_ns
    ));
    s.push_str(&format!(
        "  TNS          {}{:.2} ns\n",
        if r.tns_ns < 0.0 { "" } else { "+" },
        r.tns_ns
    ));
    s.push_str(&format!("  max fanout   {}\n", r.max_fanout));
    s.push_str(&format!(
        "  endpoints    {} ({} ff, {} out)\n",
        r.endpoints.len(),
        r.endpoints.iter().filter(|e| e.kind == "ff").count(),
        r.endpoints.iter().filter(|e| e.kind == "out").count()
    ));
    if r.endpoints.iter().any(|e| e.false_path) {
        s.push_str(&format!(
            "  ({} false path di-skip)\n",
            r.endpoints.iter().filter(|e| e.false_path).count()
        ));
    }
    s.push('\n');
    for p in &r.critical_paths {
        s.push_str(&format!(
            "  critical path ({} ns):\n    {}\n    ↓\n",
            format!("{:.2}", p.delay_ns),
            p.from
        ));
        for (i, c) in p.cells.iter().rev().enumerate() {
            let arrow = if i + 1 < p.cells.len() {
                "    ↓"
            } else {
                "    └"
            };
            s.push_str(&format!("    {}\n{}\n", c, arrow));
        }
        s.push_str(&format!("    {}\n", p.to));
        s.push('\n');
    }
    if r.critical_paths.is_empty() {
        s.push_str("  (tidak ada path kombinasional — design hanya FF?) \n");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use maria_core::intern::Symbol;
    use maria_netlist::cell::{CellInstance, PinConn};
    use maria_netlist::net::Netlist;

    /// y = ~(a & b): LUT1(and) → LUT2(not) → FF-D. Dua LUT.
    fn lut_chain_ff() -> Netlist {
        let mut nl = Netlist::new(Symbol::intern("top"));
        nl.add_port(Symbol::intern("a"), PortDir::Input, 1);
        nl.add_port(Symbol::intern("b"), PortDir::Input, 1);
        nl.add_port(Symbol::intern("clk"), PortDir::Input, 1);
        nl.add_port(Symbol::intern("q"), PortDir::Output, 1);
        let a = nl.add_net(Symbol::intern("a"), 1);
        let b = nl.add_net(Symbol::intern("b"), 1);
        let clk = nl.add_net(Symbol::intern("clk"), 1);
        let t1 = nl.add_net(Symbol::intern("t1"), 1);
        let t2 = nl.add_net(Symbol::intern("t2"), 1);
        let d = nl.add_net(Symbol::intern("d"), 1);
        let q = nl.add_net(Symbol::intern("q"), 1);
        nl.nets[t1].is_io = true;
        nl.nets[t2].is_io = true;
        nl.nets[d].is_io = true;
        nl.nets[q].is_io = true;

        let mut l1 = CellInstance::new(Symbol::intern("u1"), CellKind::Lut { init: 0x8 }, 1);
        l1.inputs = vec![
            PinConn {
                net: a,
                pin: "i0".into(),
                bit: None,
            },
            PinConn {
                net: b,
                pin: "i1".into(),
                bit: None,
            },
        ];
        l1.outputs = vec![PinConn {
            net: t1,
            pin: "o".into(),
            bit: None,
        }];
        let mut l2 = CellInstance::new(Symbol::intern("u2"), CellKind::Lut { init: 0x1 }, 1);
        l2.inputs = vec![PinConn {
            net: t1,
            pin: "i0".into(),
            bit: None,
        }];
        l2.outputs = vec![PinConn {
            net: t2,
            pin: "o".into(),
            bit: None,
        }];
        let mut ff = CellInstance::new(Symbol::intern("q_reg"), CellKind::Dff, 1);
        ff.inputs = vec![
            PinConn {
                net: clk,
                pin: "c".into(),
                bit: None,
            },
            PinConn {
                net: t2,
                pin: "d".into(),
                bit: None,
            },
        ];
        ff.outputs = vec![PinConn {
            net: q,
            pin: "q".into(),
            bit: None,
        }];
        nl.add_cell(l1);
        nl.add_cell(l2);
        nl.add_cell(ff);
        nl
    }

    #[test]
    fn wns_critical_path_manual() {
        let nl = lut_chain_ff();
        let c = Constraint {
            clocks: vec![crate::constraint::ClockSpec {
                name: "clk".into(),
                period_ns: 10.0,
            }],
            ..Default::default()
        };
        let r = analyze(&nl, &c, &TimingOptions::default());
        // a→LUT1 (0.30 + 0.05×1 fanout = 0.35) → LUT2 (0.30 + 0.05×1 = 0.35)
        // arrival FF-D = 0.70; required = 10 − 0.2 = 9.80; slack = 9.10.
        assert!(
            (r.wns_ns - 9.10).abs() < 1e-6,
            "WNS = 9.10, dapat {}",
            r.wns_ns
        );
        assert!((r.tns_ns).abs() < 1e-9, "TNS = 0");
        // Critical path: LUT2 → LUT1 (dari endpoint FF-D).
        let cp = &r.critical_paths[0];
        assert_eq!(cp.to, "q_reg");
        assert_eq!(cp.cells, vec!["u2", "u1"], "path harus u2 → u1");
        assert!((cp.delay_ns - 0.70).abs() < 1e-6);
    }

    /// input → LUT → output port (tanpa FF) — meniru alu (endpoint out).
    fn lut_to_output() -> Netlist {
        let mut nl = Netlist::new(Symbol::intern("top"));
        nl.add_port(Symbol::intern("a"), PortDir::Input, 1);
        nl.add_port(Symbol::intern("y"), PortDir::Output, 1);
        let a = nl.add_net(Symbol::intern("a"), 1);
        let y = nl.add_net(Symbol::intern("y"), 1);
        nl.nets[y].is_io = true;
        let mut l = CellInstance::new(Symbol::intern("u0"), CellKind::Lut { init: 0x1 }, 1);
        l.inputs = vec![PinConn {
            net: a,
            pin: "i0".into(),
            bit: None,
        }];
        l.outputs = vec![PinConn {
            net: y,
            pin: "o".into(),
            bit: None,
        }];
        nl.add_cell(l);
        nl
    }

    #[test]
    fn output_port_endpoint_gets_arrival() {
        let nl = lut_to_output();
        let c = Constraint {
            clocks: vec![crate::constraint::ClockSpec {
                name: "clk".into(),
                period_ns: 10.0,
            }],
            input_delay_ns: 2.0,
            output_delay_ns: 1.0,
            ..Default::default()
        };
        let r = analyze(&nl, &c, &TimingOptions::default());
        assert_eq!(r.endpoints.len(), 1, "satu endpoint (output y)");
        let e = &r.endpoints[0];
        assert_eq!(e.name, "y");
        // arrival = input_delay 2.0 + LUT 0.30 (fanout 0 — port output) = 2.30.
        assert!(
            (e.arrival_ns - 2.30).abs() < 1e-6,
            "arrival y = 2.30, dapat {}",
            e.arrival_ns
        );
        // required = 10 − 1 = 9 → slack = 9 − 2.30 = 6.70.
        assert!(
            (e.slack_ns - 6.70).abs() < 1e-6,
            "slack = 6.70, dapat {}",
            e.slack_ns
        );
        assert!(
            (r.wns_ns - 6.70).abs() < 1e-6,
            "WNS = 6.70, dapat {}",
            r.wns_ns
        );
        assert!(!r.critical_paths.is_empty(), "harus ada critical path ke y");
        assert_eq!(r.critical_paths[0].to, "y");
    }

    #[test]
    fn violation_reports_negative_wns() {
        let nl = lut_chain_ff();
        let c = Constraint {
            clocks: vec![crate::constraint::ClockSpec {
                name: "clk".into(),
                period_ns: 0.5,
            }],
            ..Default::default()
        };
        let r = analyze(&nl, &c, &TimingOptions::default());
        // required = 0.5 − 0.2 = 0.30; arrival 0.70 → slack = −0.40.
        assert!(
            (r.wns_ns - -0.40).abs() < 1e-6,
            "WNS negatif, dapat {}",
            r.wns_ns
        );
        assert!((r.tns_ns - -0.40).abs() < 1e-6, "TNS = WNS (satu endpoint)");
    }

    #[test]
    fn false_path_skipped_from_wns() {
        let nl = lut_chain_ff();
        let c = Constraint {
            clocks: vec![crate::constraint::ClockSpec {
                name: "clk".into(),
                period_ns: 0.5,
            }],
            false_paths: vec![crate::constraint::PathSpec {
                from: None,
                to: Some("q_reg".into()),
                cycles: 0,
            }],
            ..Default::default()
        };
        let r = analyze(&nl, &c, &TimingOptions::default());
        // False path q_reg di-skip dari WNS. Endpoint tersisa: output port `q`
        // (FF-Q → q): arrival = clk_to_q 0.5, required = 0.5 → slack 0.0.
        assert!(
            (r.wns_ns - 0.0).abs() < 1e-9,
            "false path di-skip, WNS = {}",
            r.wns_ns
        );
        assert_eq!(
            r.critical_paths[0].to, "q",
            "critical path tersisa ke output q"
        );
        assert!(r.critical_paths.iter().all(|p| p.to != "q_reg"));
    }
}
