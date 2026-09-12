#[test]
fn dump_ir() {
    let src = std::fs::read_to_string("/tmp/genx.sv").unwrap();
    let ir = maria_api::compile_str_analyze(&src).unwrap();
    for (pid, p) in ir.top.processes.iter().enumerate() {
        eprintln!("-- proc {} {:?}", pid, p);
    }
    assert!(true);
}
