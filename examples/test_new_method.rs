use maria::compile_str;
fn main() {
    let source = r#"
class counter;
    int count;
    function new();
        count = 0;
    endfunction
    function void inc();
        count = count + 1;
    endfunction
    function int get();
        return count;
    endfunction
endclass
module tb;
    initial begin
        #1 $finish;
    end
endmodule
"#;
    match compile_str(source) {
        Ok(design) => {
            if let Some(cls) = design.classes.get("counter") {
                println!(
                    "Class 'counter': {} fields, {} methods",
                    cls.fields.len(),
                    cls.methods.len()
                );
                for m in &cls.methods {
                    println!(
                        "  method: '{}' (virtual={}) stmts={:?}",
                        m.name, m.virtual_flag, m.stmts
                    );
                }
            }
        }
        Err(e) => eprintln!("Error: {}", e),
    }
}
