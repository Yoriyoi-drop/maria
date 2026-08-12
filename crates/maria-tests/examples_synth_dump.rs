fn main() {
    let src = std::fs::read_to_string("examples/synth/alu_opt.sv").unwrap();
    let ir = maria_api::compile_str(&src).expect("compile");
    let lower = maria_sir::lower(&ir);
    let mut pipeline = maria_synth::SynthPipeline::with_preset("fpga").expect("preset");
    let (sir_opt, _) = pipeline.run(lower.module).expect("opt");
    let res = maria_synth::tech_map(&sir_opt, &maria_tech::FpgaX7Arch);
    let v = maria_netlist::emit_verilog(&res.netlist);
    println!("{}", v);
}
