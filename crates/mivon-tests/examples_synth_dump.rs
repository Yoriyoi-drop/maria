fn main() {
    let src = std::fs::read_to_string("examples/synth/alu_opt.sv").unwrap();
    let ir = mivon_api::compile_str(&src).expect("compile");
    let lower = mivon_sir::lower(&ir);
    let mut pipeline = mivon_synth::SynthPipeline::with_preset("fpga").expect("preset");
    let (sir_opt, _) = pipeline.run(lower.module).expect("opt");
    let res = mivon_synth::tech_map(&sir_opt, &mivon_tech::FpgaX7Arch);
    let v = mivon_netlist::emit_verilog(&res.netlist);
    println!("{}", v);
}
