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
    counter c;
    int result;
    initial begin
        c = new();
        result = c.get();
        #1 $finish;
    end
endmodule
"#;
    match compile_str(source) {
        Ok(design) => {
            println!("Design top signals:");
            for s in &design.top.signals {
                println!("  signal: {} width={} class={:?} array_depth={}", s.name, s.width, s.class_name, s.array_depth);
            }
            if let Some(cls) = design.classes.get("counter") {
                println!("Class 'counter': {} fields, {} methods", cls.fields.len(), cls.methods.len());
                for m in &cls.methods {
                    println!("  method: '{}' stmts={:?}", m.name, m.stmts);
                }
            } else {
                println!("Class 'counter' NOT FOUND in design.classes!");
                println!("Available classes: {:?}", design.classes.keys().collect::<Vec<_>>());
            }
        }
        Err(e) => eprintln!("Error: {}", e),
    }
}
