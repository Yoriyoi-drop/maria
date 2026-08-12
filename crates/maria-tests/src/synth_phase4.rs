//! E2E phase 4 — tech mapping (SYNTHESIS.md §5/§12): netlist hasil mapping
//! LUT6/CARRY4/FF disimulasikan engine Maria dan hasilnya DIBANDINGKAN
//! dengan simulasi RTL asli. Loop verifikasi tertutup (kriteria fase 3/4):
//! `sim netlist = sim RTL`.
//!
//! Flow per kasus: compile RTL → lower ke SIR → pass optimizer → tech_map
//! (LUT cut + AIG decomposition + carry chain) → emit `netlist.v` → sim
//! netlist (dengan testbench yang sama) → bandingkan signal dengan sim RTL.

use std::collections::HashMap;

/// Path absolut contoh RTL di `examples/synth/` (cwd test = crate root).
fn example(name: &str) -> String {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/synth");
    std::fs::read_to_string(std::path::Path::new(dir).join(name))
        .unwrap_or_else(|e| panic!("baca examples/synth/{name}: {e}"))
}

/// Pipeline lengkap RTL → netlist tech-mapped (untuk tes).
fn rtl_to_mapped_netlist(source: &str) -> String {
    let ir = maria_api::compile_str(source).expect("compile RTL");
    let lower = maria_sir::lower(&ir);
    assert!(
        lower.skipped.is_empty(),
        "SIR lowering tidak boleh skip: {:?}",
        lower.skipped
    );
    let mut pipeline = maria_synth::SynthPipeline::with_preset("fpga").expect("preset");
    let (sir_opt, _results) = pipeline.run(lower.module).expect("optimizer SIR");
    let res = maria_synth::tech_map(&sir_opt, &maria_tech::FpgaX7Arch);
    assert!(
        res.skipped.is_empty(),
        "tech map tidak boleh skip: {:?}",
        res.skipped
    );
    let check = maria_netlist::verify_dag(&res.netlist);
    assert!(check.ok, "netlist mapped harus DAG bersih: {:?}", check);
    maria_netlist::emit_verilog(&res.netlist)
}

/// Hasil mapping langsung (tanpa emit) untuk cek utilisasi.
fn mapped_result(source: &str) -> maria_synth::TechMapResult {
    let ir = maria_api::compile_str(source).expect("compile RTL");
    let lower = maria_sir::lower(&ir);
    let mut pipeline = maria_synth::SynthPipeline::with_preset("fpga").unwrap();
    let (sir_opt, _) = pipeline.run(lower.module).unwrap();
    maria_synth::tech_map(&sir_opt, &maria_tech::FpgaX7Arch)
}

/// Signal final (u64) dari simulasi source (top module).
fn sim_final(source: &str, max_time: u64) -> HashMap<String, u64> {
    let sigs = maria_api::simulate_signals(source, max_time).expect("simulasi");
    sigs.into_iter()
        .map(|(name, lv)| (name, lv.to_u64()))
        .collect()
}

/// Testbench counter 8-bit — instansiasi `counter`, drive clk/rst/enable,
/// `count` jadi signal top (bisa dibaca hasil sim).
const TB_COUNTER: &str = r#"
module tb_counter();
  logic clk = 0;
  logic rst_n = 0;
  logic enable = 0;
  logic [7:0] count;
  counter dut(.clk(clk), .rst_n(rst_n), .enable(enable), .count(count));
  always #5 clk = ~clk;
  initial begin
    #10 rst_n = 1;
    enable = 1;
    #300;
    $finish;
  end
endmodule
"#;

#[test]
fn tech_mapped_counter_sim_equals_rtl() {
    let rtl = example("counter.sv");
    let netlist_v = rtl_to_mapped_netlist(&rtl);

    let rtl_sim = sim_final(&format!("{rtl}\n{TB_COUNTER}"), 310);
    let net_sim = sim_final(&format!("{netlist_v}\n{TB_COUNTER}"), 310);

    let rtl_count = rtl_sim.get("count").expect("count di RTL");
    let net_count = net_sim.get("count").expect("count di netlist");
    assert_eq!(
        net_count, rtl_count,
        "count netlist != count RTL — netlist={net_count} rtl={rtl_count}"
    );
    assert_eq!(*rtl_count, 30, "30 hitungan setelah reset");
}

/// Testbench alu_opt (kombinasional) — drive a/b, baca y/z.
const TB_ALU: &str = r#"
module tb_alu();
  logic [7:0] a = 8'h35;
  logic [7:0] b = 8'h4A;
  logic [7:0] y, z;
  alu_opt dut(.a(a), .b(b), .y(y), .z(z));
  initial begin
    #10;
    $finish;
  end
endmodule
"#;

#[test]
fn tech_mapped_alu_opt_sim_equals_rtl() {
    let rtl = example("alu_opt.sv");
    let netlist_v = rtl_to_mapped_netlist(&rtl);

    let rtl_sim = sim_final(&format!("{rtl}\n{TB_ALU}"), 10);
    let net_sim = sim_final(&format!("{netlist_v}\n{TB_ALU}"), 10);

    // y = a|b = 0x35|0x4A = 0x7F; z = a + (b<<2) = 53 + 296 = 349 → 8-bit 93.
    let (ry, rz) = (rtl_sim["y"], rtl_sim["z"]);
    let (ny, nz) = (net_sim["y"], net_sim["z"]);
    assert_eq!(ny, ry, "y netlist != y RTL: {ny} vs {ry}");
    assert_eq!(nz, rz, "z netlist != z RTL: {nz} vs {rz}");
    assert_eq!(ry, 127, "y = a|b = 0x7F");
    assert_eq!(rz, 93, "z = a+(b<<2) = 0x5D");
}

/// LUT count sesuai ekspektasi (deliverable phase 4: `alu.sv → LUT count`).
#[test]
fn tech_mapped_alu_opt_lut_count() {
    let rtl = example("alu_opt.sv");
    let res = mapped_result(&rtl);
    // y = a|b → 8 LUT (OR per bit); z = a + (b<<2) → 2 CARRY4 (8-bit, ceil(8/4)).
    // SHL konstanta → wiring murni (tanpa LUT).
    assert_eq!(res.lut_count, 8, "a|b 8-bit → 8 LUT");
    assert_eq!(res.carry4_count, 2, "adder 8-bit → 2 CARRY4 slice");
    assert_eq!(res.ff_count, 0, "alu_opt tanpa FF");
}

/// Utilisasi counter: FF 8, CARRY4 2, EQ count==99 + mux.
#[test]
fn tech_mapped_counter_lut_count() {
    let rtl = example("counter.sv");
    let res = mapped_result(&rtl);
    assert_eq!(res.ff_count, 8, "counter 8-bit → 8 FF bit");
    assert_eq!(res.carry4_count, 2, "count+1 8-bit → 2 CARRY4 slice");
    assert_eq!(res.lut_count, 9, "EQ(4) + AND-reduce(3) + MUX reset(2)");
}

/// Netlist hasil mapping deterministik: dua run → netlist identik.
#[test]
fn tech_mapped_netlist_is_deterministic() {
    let rtl = example("counter.sv");
    let a = rtl_to_mapped_netlist(&rtl);
    let b = rtl_to_mapped_netlist(&rtl);
    assert_eq!(a, b, "netlist mapped harus deterministik");
}

/// Netlist hasil mapping memuat modul sel (CARRY4 / LUT / FF reset).
#[test]
fn tech_mapped_netlist_verilog_has_cell_modules() {
    let rtl = example("counter.sv");
    let netlist_v = rtl_to_mapped_netlist(&rtl);
    assert!(netlist_v.contains("module CARRY4"), "modul CARRY4");
    assert!(netlist_v.contains("always_comb"), "LUT always_comb");
    assert!(netlist_v.contains("module DFFR_"), "FF reset");
}

/// Smoke: `maria synth --emit-netlist --tech-map` lewat tools (file tertulis).
#[test]
fn synth_tool_emit_mapped_netlist_smoke() {
    let dir = std::env::temp_dir().join("maria_synth_p4_smoke");
    std::fs::create_dir_all(&dir).ok();
    let prefix = dir.join("counter_p4");
    let counter_path = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/synth/counter.sv"
    ))
    .to_string_lossy()
    .to_string();
    let args = maria_api::tools::synth::SynthArgs {
        targets: &[counter_path],
        incdirs: &[],
        defines: &[],
        top: Some("counter"),
        output: Some(prefix.to_string_lossy().to_string()),
        check_only: false,
        device: "generic".to_string(),
        preset: "fpga".to_string(),
        emit_mvnet: false,
        dump_sir: false,
        dump_sir_opt: false,
        dump_netlist: false,
        emit_netlist: true,
        tech_map: true,
        report_util: None,
        quiet: true,
    };
    maria_api::tools::synth::run(&args).expect("maria synth --emit-netlist --tech-map");
    assert!(dir.join("counter_p4.tech.v").exists(), "tech netlist file");
    assert!(dir.join("counter_p4.tech.mvnet").exists(), "tech mvnet file");
}
