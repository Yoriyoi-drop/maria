//! E2E phase 5 — timing & area (SYNTHESIS.md §15-17): STA atas netlist
//! tech-mapped; WNS/TNS/critical path dihitung ulang MANUAL dari model delay
//! deterministik maria-timing. Kriteria fase 5: "WNS/TNS/critical path benar
//! (path manual dihitung ulang)".
//!
//! Model delay: LUT6=0.30ns, CARRY4=0.05×width, CONCAT/BUF=0.10, fanout
//! penalty 0.05/load; clk→q 0.50, setup 0.20; input/output delay dari
//! constraint.

/// Path absolut contoh RTL di `examples/synth/`.
fn example(name: &str) -> String {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/synth");
    std::fs::read_to_string(std::path::Path::new(dir).join(name))
        .unwrap_or_else(|e| panic!("baca examples/synth/{name}: {e}"))
}

/// Pipeline lengkap RTL → netlist tech-mapped (LUT6/CARRY4/FF).
fn rtl_to_netlist(source: &str) -> maria_netlist::Netlist {
    let ir = maria_api::compile_str(source).expect("compile RTL");
    let lower = maria_sir::lower(&ir);
    assert!(
        lower.skipped.is_empty(),
        "SIR lowering tidak boleh skip: {:?}",
        lower.skipped
    );
    let mut pipeline = maria_synth::SynthPipeline::with_preset("fpga").expect("preset");
    let (sir_opt, _results) = pipeline.run(lower.module).expect("optimizer SIR");
    maria_synth::tech_map(&sir_opt, &maria_tech::FpgaX7Arch).netlist
}

#[test]
fn phase5_alu_timing_matches_manual() {
    let nl = rtl_to_netlist(&example("alu.sv"));
    let c = maria_timing::Constraint {
        clocks: vec![maria_timing::ClockSpec { name: "clk".into(), period_ns: 10.0 }],
        input_delay_ns: 2.0,
        output_delay_ns: 1.0,
        ..Default::default()
    };
    let r = maria_timing::analyze(&nl, &c, &maria_timing::TimingOptions::default());
    // alu: satu endpoint output `y`. Jalur kritis manual:
    //   input_delay 2.0 → CARRY4 (0.05×8 + 0.05×8 fanout = 0.80)
    //   → LUT (0.30 + 0.05×1 = 0.35) → CONCAT n9 (0.10 + 0.05×1 = 0.15)
    //   → BUF out_y (0.10 + 0.05×0 = 0.10)
    //   arrival = 2.0 + 0.80 + 0.35 + 0.15 + 0.10 = 3.40
    //   required = 10 − 1 = 9 → WNS = 9 − 3.40 = 5.60.
    assert!(
        (r.wns_ns - 5.60).abs() < 1e-6,
        "WNS alu = 5.60 (manual), dapat {:.3}",
        r.wns_ns
    );
    assert!((r.tns_ns).abs() < 1e-9, "TNS alu = 0 (tidak ada violation)");
    assert_eq!(r.critical_paths.len(), 1);
    let cp = &r.critical_paths[0];
    assert_eq!(cp.to, "y");
    assert!(
        (cp.delay_ns - 3.40).abs() < 1e-6,
        "critical path alu = 3.40 (manual), dapat {:.3}",
        cp.delay_ns
    );
    // Path harus melewati CARRY4 (carry chain) + LUT + concat + buffer.
    let cells = cp.cells.join(",");
    assert!(cells.contains("carry0"), "path lewat carry chain: {cells}");
    assert!(cells.contains("lut"), "path lewat LUT: {cells}");

    // Area: 8 LUT (case → 1 LUT6 per bit) + 2 CARRY4 slice.
    let a = maria_timing::estimate_area(&nl);
    assert_eq!(a.lut, 8, "alu → 8 LUT6");
    assert_eq!(a.carry4, 2, "a+b 8-bit → 2 CARRY4 slice");
    assert_eq!(a.ff, 0);
    assert!((a.area_units - 12.35).abs() < 1e-9, "area = 8+4+0.1+0.25 = 12.35, dapat {}", a.area_units);
}

#[test]
fn phase5_counter_ff_endpoints_and_wns() {
    let nl = rtl_to_netlist(&example("counter.sv"));
    let c = maria_timing::Constraint {
        clocks: vec![maria_timing::ClockSpec { name: "clk".into(), period_ns: 10.0 }],
        ..Default::default()
    };
    let r = maria_timing::analyze(&nl, &c, &maria_timing::TimingOptions::default());
    // counter 8-bit → 8 FF bit (endpoint FF-D) + 1 output port.
    assert_eq!(r.endpoints.iter().filter(|e| e.kind == "ff").count(), 8);
    assert_eq!(r.endpoints.iter().filter(|e| e.kind == "out").count(), 1);
    // Periode 10ns cukup → WNS positif, tidak ada violation.
    assert!(r.wns_ns > 0.0, "WNS counter harus positif, dapat {}", r.wns_ns);
    assert!((r.tns_ns).abs() < 1e-9);
    // Critical path harus berakhir di FF-D.
    assert!(r.critical_paths.iter().any(|p| p.to.starts_with("ff_")), "path ke FF");
    // Area: 8 FF bit.
    let a = maria_timing::estimate_area(&nl);
    assert_eq!(a.ff, 8, "counter 8-bit → 8 FF bit");
    assert_eq!(a.lut, 28, "eq(99) 4 LUT + mux 8 + enable: LUT=28");
}

#[test]
fn phase5_constraint_false_path_skipped() {
    // Constraint `.mcs` dengan false_path — jalur dari `rst` dikecualikan
    // dari WNS/TNS (parser + is_false_path terintegrasi).
    let text = "clock clk { period = 0.5ns; }\nfalse_path { from = rst; }\n";
    let c = maria_timing::parse_constraints(text);
    assert!(c.is_false_path("rst_n", "q"), "rst_n harus false path");
    assert!(!c.is_false_path("a", "q"), "a bukan false path");
    assert_eq!(c.cycle_multiplier("reg_a", "reg_b"), 1, "tanpa multicycle → 1");
}
