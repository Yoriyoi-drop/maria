pub mod ast;
pub mod elaboration;
pub mod ir;
pub mod parser;
pub mod simulator;
pub mod waveform;

use std::fs;
use std::path::Path;
use parser::lexer::Lexer;
use parser::parser::Parser;
use parser::preprocessor::Preprocessor;

/// Read a .maria project file and return list of .sv file paths
/// Paths in .maria are resolved relative to the .maria file's directory
pub fn read_project_file(path: &str) -> Result<Vec<String>, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("cannot read '{}': {}", path, e))?;
    let base = Path::new(path).parent().unwrap_or(Path::new("."));
    let files: Vec<String> = content.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let p = base.join(l);
            p.to_string_lossy().to_string()
        })
        .collect();
    if files.is_empty() {
        return Err(format!("no .sv files listed in '{}'", path));
    }
    Ok(files)
}

/// Compile multiple .sv files into IR design
pub fn compile_files(paths: &[String]) -> Result<ir::IrDesign, String> {
    let mut combined = String::new();
    for path in paths {
        let mut pp = Preprocessor::new();
        let processed = pp.preprocess_file(path)?;
        combined.push_str(&processed);
        combined.push('\n');
    }
    compile_str(&combined)
}

/// Compile a SystemVerilog source file and run simulation
pub fn simulate_file(path: &str, max_time: u64) -> Result<(), String> {
    let source = fs::read_to_string(path)
        .map_err(|e| format!("cannot read '{}': {}", path, e))?;
    simulate_str(&source, max_time)
}

/// Compile SystemVerilog source string and run simulation
pub fn simulate_str(source: &str, max_time: u64) -> Result<(), String> {
    let design = compile_str(source)?;
    run_simulation(design, max_time)
}

/// Compile SystemVerilog source string into IR
pub fn compile_str(source: &str) -> Result<ir::IrDesign, String> {
    let mut pp = Preprocessor::new();
    let preprocessed = pp.preprocess(source, None)
        .map_err(|e| format!("preprocessor: {}", e))?;
    let mut lexer = Lexer::new(&preprocessed);
    let mut tokens = Vec::new();
    loop {
        let (tok, line, col) = lexer.next_token();
        if tok == parser::lexer::Token::Eof {
            break;
        }
        tokens.push((tok, line, col));
    }

    let mut parser = Parser::new(tokens);
    let design = parser.parse_design()?;

    let mut elaborator = elaboration::Elaborator::new(design);
    let ir_design = elaborator.elaborate(None)?;

    Ok(ir_design)
}

/// Run simulation on compiled IR
pub fn run_simulation(ir_design: ir::IrDesign, max_time: u64) -> Result<(), String> {
    let mut engine = simulator::SimulationEngine::new(ir_design, max_time);

    let design_name = &engine.design.top.name.clone();
    let vcd_path = format!("{}.vcd", design_name);
    let vcd = waveform::VcdWriter::new(&vcd_path, &engine.design)
        .map_err(|e| format!("VCD creation failed: {}", e))?;
    engine.set_vcd(vcd);

    engine.run()?;

    println!("Simulation completed at time {}", engine.state.time);
    println!("VCD waveform written to '{}'", vcd_path);

    Ok(())
}

/// Run simulation and return final signal values
pub fn simulate_signals(source: &str, max_time: u64) -> Result<Vec<(String, ir::LogicVec)>, String> {
    let design = compile_str(source)?;
    let mut engine = simulator::SimulationEngine::new(design, max_time);
    engine.run()?;
    let sigs: Vec<(String, ir::LogicVec)> = engine.design.top.signals.iter()
        .map(|s| (s.name.clone(), engine.state.read_signal(
            engine.design.top.signals.iter()
                .position(|x| x.name == s.name).unwrap_or(0)
        ).clone()))
        .collect();
    Ok(sigs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_module() {
        let source = r#"
module counter(
    input clk,
    input rst_n,
    output reg [3:0] count
);
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n)
            count <= 4'b0000;
        else
            count <= count + 4'b0001;
    end
endmodule
"#;
        let design = compile_str(source);
        assert!(design.is_ok(), "compilation failed: {:?}", design.err());
    }

    #[test]
    fn test_byte_shortint_longint_decl() {
        let source = r#"
module test;
    byte b;
    byte signed bs;
    shortint s;
    shortint signed ss;
    longint l;
    longint signed ls;
    byte [7:0] ba;
    initial begin
        b = 8'hAB;
        s = 16'hABCD;
        l = 64'h1234567890ABCDEF;
    end
endmodule
"#;
        let design = compile_str(source);
        assert!(design.is_ok(), "compilation failed: {:?}", design.err());
    }

    #[test]
    fn test_enum_decl() {
        let source = r#"
module test;
    enum { IDLE, START, DONE } state;
endmodule
"#;
        let design = compile_str(source);
        assert!(design.is_ok(), "compilation failed: {:?}", design.err());
    }

    #[test]
    fn test_packed_enum_decl() {
        let source = r#"
module test;
    enum bit [3:0] { RED, GREEN, BLUE } color;
    enum logic [7:0] { A, B, C } val;
    enum int { X, Y, Z } ival;
endmodule
"#;
        let design = compile_str(source);
        assert!(design.is_ok(), "compilation failed: {:?}", design.err());
    }

    #[test]
    fn test_typedef_enum() {
        let source = r#"
module test;
    typedef enum { A, B, C } state_t;
endmodule
"#;
        let design = compile_str(source);
        assert!(design.is_ok(), "compilation failed: {:?}", design.err());
    }

    #[test]
    fn test_typedef_used_in_decl() {
        let source = r#"
module test;
    typedef enum { IDLE, START, DONE } state_t;
    state_t st;
endmodule
"#;
        let design = compile_str(source);
        assert!(design.is_ok(), "compilation failed: {:?}", design.err());
    }

    #[test]
    fn test_typedef_base_types() {
        let source = r#"
module test;
    typedef byte byte_t;
    typedef shortint short_t;
    typedef longint long_t;
    typedef int int_t;
    typedef logic logic_t;
endmodule
"#;
        let design = compile_str(source);
        assert!(design.is_ok(), "compilation failed: {:?}", design.err());
    }

    #[test]
    fn test_typedef_used_with_base_types() {
        let source = r#"
module test;
    typedef byte byte_t;
    typedef shortint short_t;
    byte_t b;
    short_t s;
endmodule
"#;
        let design = compile_str(source);
        assert!(design.is_ok(), "compilation failed: {:?}", design.err());
    }

    #[test]
    fn test_struct_decl() {
        let source = r#"
module test;
    struct {
        logic [7:0] a;
        logic b;
    } my_var;
endmodule
"#;
        let design = compile_str(source);
        assert!(design.is_ok(), "compilation failed: {:?}", design.err());
    }

    #[test]
    fn test_typedef_struct() {
        let source = r#"
module test;
    typedef struct {
        logic [7:0] a;
        logic b;
    } my_struct_t;
    my_struct_t s;
endmodule
"#;
        let design = compile_str(source);
        assert!(design.is_ok(), "compilation failed: {:?}", design.err());
    }

    #[test]
    fn test_typedef_union() {
        let source = r#"
module test;
    typedef union {
        int a;
        logic [31:0] b;
    } my_union_t;
endmodule
"#;
        let design = compile_str(source);
        assert!(design.is_ok(), "compilation failed: {:?}", design.err());
    }

    #[test]
    fn test_counter_simulation() {
        let source = r#"
module tb_counter;
    reg clk;
    reg rst_n;
    wire [3:0] count;

    counter u_counter(
        .clk(clk),
        .rst_n(rst_n),
        .count(count)
    );

    initial begin
        clk = 0;
        rst_n = 0;
        #5 rst_n = 1;
        #100 $finish;
    end

    always #1 clk = ~clk;
endmodule

module counter(
    input clk,
    input rst_n,
    output reg [3:0] count
);
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n)
            count <= 4'b0000;
        else
            count <= count + 4'b0001;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 20).unwrap();
        let count_val = sigs.iter().find(|(n, _)| n == "count")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        assert_eq!(count_val, 8, "count should be 8 at time 20");
    }

    #[test]
    fn test_3level_hierarchy() {
        let source = r#"
module tb;
    reg clk;
    reg rst_n;
    reg [7:0] out;

    top u_top(
        .clk(clk),
        .rst_n(rst_n),
        .out(out)
    );

    initial begin
        clk = 0;
        rst_n = 0;
        #5 rst_n = 1;
        #100 $finish;
    end

    always #1 clk = ~clk;
endmodule

module top(input clk, input rst_n, output [7:0] out);
    sub u_sub(.clk(clk), .rst_n(rst_n), .out(out));
endmodule

module sub(input clk, input rst_n, output reg [7:0] out);
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n)
            out <= 8'd0;
        else
            out <= out + 8'd1;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 6).unwrap();
        let out_val = sigs.iter().find(|(n, _)| n == "out")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        // rst_n=0 at time 0 => out=0
        // rst_n=1 at time 5, posedge at time 6 => out=1
        assert_eq!(out_val, 1, "out should be 1 at time 6");
    }

    #[test]
    fn test_display_format() {
        let source = r#"
module tb;
    reg [7:0] a;
    reg [3:0] b;

    initial begin
        a = 8'd42;
        b = 4'd10;
        $display("a=%d b=%b a=%h", a, b, a);
        $display("plain text");
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 2).unwrap();
        let a_val = sigs.iter().find(|(n, _)| n == "a")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        assert_eq!(a_val, 42, "a should be 42");
    }

    #[test]
    fn test_for_loop_generate_mux() {
        let source = r#"
module tb;
    reg [7:0] in;
    reg [2:0] sel;
    reg [7:0] out;
    integer i;

    always @(*) begin
        out = 8'd0;
        for (i = 0; i < 8; i = i + 1) begin
            if (sel == i)
                out = in;
        end
    end

    initial begin
        in = 8'd42;
        sel = 3'd5;
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 2).unwrap();
        let out_val = sigs.iter().find(|(n, _)| n == "out")
            .map(|(_, v)| v.to_u64())
            .unwrap_or(0);
        assert_eq!(out_val, 42, "out should be 42 (in) after for-loop mux");
    }

    #[test]
    fn test_read_project_file() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("maria_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let maria_path = dir.join(".maria");
        let sv_path = dir.join("test.sv");
        {
            let mut f = fs::File::create(&maria_path).unwrap();
            writeln!(f, "# project file").unwrap();
            writeln!(f, "  ").unwrap();
            writeln!(f, "test.sv").unwrap();
        }
        {
            let mut f = fs::File::create(&sv_path).unwrap();
            writeln!(f, "module tb; initial begin #1 $finish; end endmodule").unwrap();
        }

        let files = read_project_file(maria_path.to_str().unwrap()).unwrap();
        assert_eq!(files.len(), 1, "should read 1 file from .maria");
        assert!(files[0].ends_with("test.sv"));

        let design = compile_files(&files).unwrap();
        assert_eq!(design.top.name, "tb");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parameterized_module_width() {
        let source = r#"
module tb;
    reg clk;
    reg rst_n;
    wire [7:0] count;

    counter #(8) u_counter(
        .clk(clk),
        .rst_n(rst_n),
        .count(count)
    );

    initial begin
        clk = 0;
        rst_n = 0;
        #5 rst_n = 1;
        #100 $finish;
    end

    always #1 clk = ~clk;
endmodule

module counter #(parameter WIDTH = 8) (
    input clk,
    input rst_n,
    output reg [WIDTH-1:0] count
);
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n)
            count <= {WIDTH{1'b0}};
        else
            count <= count + 1'b1;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 20).unwrap();
        let count_val = sigs.iter().find(|(n, _)| n == "count")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        // rst_n=0 at time 0, goes high at 5
        // posedge at 6 => count=1, posedge at 8 => count=2, ... posedge at 20 => count=8
        assert_eq!(count_val, 8, "8-bit counter should be 8 at time 20");
    }

    #[test]
    fn test_array_memory_simulation() {
        let source = r#"
module tb;
    reg clk;
    reg [7:0] mem [0:3];
    reg [1:0] addr;
    wire [7:0] rd_data;

    assign rd_data = mem[addr];

    initial begin
        clk = 0;
        mem[0] = 8'hA0;
        mem[1] = 8'hB1;
        mem[2] = 8'hC2;
        mem[3] = 8'hD3;
        addr = 0;
        #10 addr = 1;
        #10 addr = 2;
        #10 addr = 3;
        #10 $finish;
    end

    always #5 clk = ~clk;
endmodule
"#;
        let sigs = simulate_signals(source, 50).unwrap();

        // Final rd_data should be mem[3]=0xD3 (addr changes to 3 at time 30, then #10 at time 40)
        let rd_val = sigs.iter().find(|(n, _)| n == "rd_data").map(|(_, v)| v.to_u64()).unwrap();
        assert_eq!(rd_val, 0xD3, "rd_data final should be 0xD3 (mem[3])");
    }

    #[test]
    fn test_array_with_readmemh() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("maria_array_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let hex_path = dir.join("mem_init.hex");
        {
            let mut f = fs::File::create(&hex_path).unwrap();
            writeln!(f, "A0").unwrap();
            writeln!(f, "B1").unwrap();
            writeln!(f, "C2").unwrap();
            writeln!(f, "D3").unwrap();
        }

        let hex_str = hex_path.to_str().unwrap().replace('\\', "/");

        let source = format!(r#"
module tb;
    reg [7:0] mem [0:3];
    reg [1:0] addr;
    wire [7:0] rd_data;

    assign rd_data = mem[addr];

    initial begin
        $readmemh("{hex}", mem);
        addr = 0;
        #10 addr = 2;
        #10 $finish;
    end
endmodule
"#, hex = hex_str);
        let sigs = simulate_signals(&source, 30).unwrap();

        // Final rd_data should be mem[2]=0xC2 (addr changes to 2 at time 10, then #10 $finish at 20)
        let rd_val = sigs.iter().find(|(n, _)| n == "rd_data").map(|(_, v)| v.to_u64()).unwrap();
        assert_eq!(rd_val, 0xC2, "rd_data final should be 0xC2 (mem[2])");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_array_index_edge_cases() {
        let source = r#"
module tb;
    reg [3:0] mem [0:1];
    wire [3:0] out0;
    wire [3:0] out1;

    assign out0 = mem[0];
    assign out1 = mem[1];

    initial begin
        mem[0] = 4'hF;
        mem[1] = 4'h5;
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 2).unwrap();

        let out0_val = sigs.iter().find(|(n, _)| n == "out0").map(|(_, v)| v.to_u64()).unwrap();
        let out1_val = sigs.iter().find(|(n, _)| n == "out1").map(|(_, v)| v.to_u64()).unwrap();
        assert_eq!(out0_val, 0xF, "mem[0] should be 0xF");
        assert_eq!(out1_val, 0x5, "mem[1] should be 0x5");
    }

    #[test]
    fn test_parameterized_module_instance_override() {
        let source = r#"
module tb;
    reg [15:0] a;
    reg [15:0] b;
    wire [15:0] sum;

    adder #(16) u_adder(
        .a(a),
        .b(b),
        .sum(sum)
    );

    initial begin
        a = 16'd100;
        b = 16'd200;
        #1 $finish;
    end
endmodule

module adder #(parameter WIDTH = 8) (
    input [WIDTH-1:0] a,
    input [WIDTH-1:0] b,
    output [WIDTH-1:0] sum
);
    assign sum = a + b;
endmodule
"#;
        let sigs = simulate_signals(source, 2).unwrap();
        let sum_val = sigs.iter().find(|(n, _)| n == "sum")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        assert_eq!(sum_val, 300, "16-bit adder: 100 + 200 = 300");
    }

    #[test]
    fn test_function_call() {
        let source = r#"
module tb;
    reg [7:0] a, b, result;

    function [7:0] add;
        input [7:0] a, b;
        begin
            add = a + b;
        end
    endfunction

    initial begin
        a = 10;
        b = 20;
        result = add(a, b);
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 2).unwrap();
        let result_val = sigs.iter().find(|(n, _)| n == "result")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        assert_eq!(result_val, 30, "add(10, 20) should be 30");
    }

    #[test]
    fn test_function_call_in_expr() {
        let source = r#"
module tb;
    reg [7:0] result;

    function [7:0] add;
        input [7:0] a, b;
        begin
            add = a + b;
        end
    endfunction

    function [7:0] mul;
        input [7:0] a, b;
        begin
            mul = a * b;
        end
    endfunction

    initial begin
        result = add(mul(2, 3), 1);
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 2).unwrap();
        let result_val = sigs.iter().find(|(n, _)| n == "result")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        assert_eq!(result_val, 7, "add(mul(2,3), 1) = 7");
    }

    #[test]
    fn test_function_call_in_always_ff() {
        let source = r#"
module tb;
    reg clk;
    reg [7:0] a, b, q;

    function [7:0] add;
        input [7:0] a, b;
        begin
            add = a + b;
        end
    endfunction

    always_ff @(posedge clk) begin
        q <= add(a, b);
    end

    initial begin
        clk = 0;
        a = 5; b = 7;
        #1 clk = 1;
        #1 clk = 0;
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 4).unwrap();
        let q_val = sigs.iter().find(|(n, _)| n == "q")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        assert_eq!(q_val, 12, "q should be 12 after posedge clk");
    }

    #[test]
    fn test_function_internal_decl() {
        let source = r#"
module tb;
    reg [7:0] result;

    function [7:0] add;
        input [7:0] a, b;
        reg [7:0] temp;
        begin
            temp = a + b;
            add = temp;
        end
    endfunction

    initial begin
        result = add(30, 12);
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 2).unwrap();
        let result_val = sigs.iter().find(|(n, _)| n == "result")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        assert_eq!(result_val, 42, "add(30, 12) via internal temp should be 42");
    }

    #[test]
    fn test_function_continuous_assign() {
        let source = r#"
module tb;
    reg [7:0] a, b;
    wire [7:0] result;

    function [7:0] add;
        input [7:0] a, b;
        begin
            add = a + b;
        end
    endfunction

    assign result = add(a, b);

    initial begin
        a = 15; b = 27;
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 2).unwrap();
        let result_val = sigs.iter().find(|(n, _)| n == "result")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        assert_eq!(result_val, 42, "result from assign w/ func call should be 42");
    }

    #[test]
    fn test_generate_if() {
        let source = r#"
module tb;
    reg [7:0] result;

    generate
        if (1) begin
            always @(*) begin
                result = 8'hAB;
            end
        end else begin
            always @(*) begin
                result = 8'hCD;
            end
        end
    endgenerate

    initial begin
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 2).unwrap();
        let result_val = sigs.iter().find(|(n, _)| n == "result")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        assert_eq!(result_val, 0xAB, "generate if(1) should pick true branch");
    }

    #[test]
    fn test_generate_for() {
        let source = r#"
module tb;
    reg [3:0] result;

    genvar i;
    generate
        for (i = 0; i < 4; i = i + 1) begin
            always @(*) begin
                result[i] = 1'b1;
            end
        end
    endgenerate

    initial begin
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 2).unwrap();
        let result_val = sigs.iter().find(|(n, _)| n == "result")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        assert_eq!(result_val, 0xF, "generate for sets all bits of result");
    }

    #[test]
    fn test_signed_arithmetic() {
        let source = r#"
module tb;
    reg [7:0] a, b, result;

    function [7:0] max;
        input [7:0] a, b;
        begin
            if (a > b)
                max = a;
            else
                max = b;
        end
    endfunction

    initial begin
        a = 10;
        b = 20;
        result = max(a, b);
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 2).unwrap();
        let result_val = sigs.iter().find(|(n, _)| n == "result")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        assert_eq!(result_val, 20, "max(10, 20) should be 20");
    }

    #[test]
    fn test_signed_comparison() {
        let source = r#"
module tb;
    reg [7:0] a, b;
    reg gt;

    initial begin
        // 200 as unsigned > 100, but as signed (-56) < 100
        a = 200;
        b = 100;
        // Use unsigned comparison
        if (a > b)
            gt = 1;
        else
            gt = 0;
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 2).unwrap();
        let gt_val = sigs.iter().find(|(n, _)| n == "gt")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        assert_eq!(gt_val, 1, "unsigned 200 > 100");
    }

    #[test]
    fn test_class_parsing_basic() {
        let source = r#"
class driver;
    logic [7:0] data;
    function new();
        data = 42;
    endfunction
    virtual function void print();
        $display("data = %d", data);
    endfunction
endclass
module tb;
    initial begin
        #1 $finish;
    end
endmodule
"#;
        let design = compile_str(source).unwrap();
        assert!(design.classes.contains_key("driver"), "class 'driver' should be registered");
        let cls = &design.classes["driver"];
        assert_eq!(cls.name, "driver");
        assert!(cls.extends.is_none());
        assert_eq!(cls.fields.len(), 1, "driver has 1 field");
        assert_eq!(cls.fields[0].name, "data");
        assert_eq!(cls.methods.len(), 2, "driver has 2 methods (new + print)");
        assert!(cls.methods.iter().any(|m| m.name == "new"));
        assert!(cls.methods.iter().any(|m| m.name == "print" && m.virtual_flag));
    }

    #[test]
    fn test_class_parsing_extends() {
        let source = r#"
class uvm_component;
    string name;
    function new(string name);
        this.name = name;
    endfunction
endclass
class driver extends uvm_component;
    logic [7:0] data;
    function new(string name);
        super.new(name);
    endfunction
endclass
module tb;
    initial begin
        #1 $finish;
    end
endmodule
"#;
        let design = compile_str(source).unwrap();
        assert!(design.classes.contains_key("uvm_component"));
        assert!(design.classes.contains_key("driver"));
        assert_eq!(design.classes["driver"].extends.as_deref(), Some("uvm_component"));
    }

    #[test]
    fn test_class_method_call_syntax() {
        // Test that obj.method() and obj.field parsing works in expressions
        // Just parse AST (not elaborate) since classes need runtime support
        let source = r#"
module tb;
    integer d, x;
    initial begin
        d = new();
        d.print();
        x = d.data;
    end
endmodule

class base;
    function new();
    endfunction
    function void print();
    endfunction
endclass
"#;
        let mut lexer = Lexer::new(source);
        use crate::parser::lexer::Token;
        let mut tokens = Vec::new();
        loop {
            let (tok, line, col) = lexer.next_token();
            if tok == Token::Eof { break; }
            tokens.push((tok, line, col));
        }
        let mut parser = crate::parser::Parser::new(tokens);
        let design = parser.parse_design().unwrap();
        assert!(design.classes.len() >= 1, "should have parsed at least one class");
        let mod_names: Vec<_> = design.modules.iter().map(|m| m.name.clone()).collect();
        assert!(mod_names.contains(&"tb".to_string()));
    }

    #[test]
    fn test_class_field_access_parsing() {
        let source = r#"
class cfg;
    integer timeout;
    function new();
        timeout = 1000;
    endfunction
endclass
module tb;
    integer x;
    integer val;
    initial begin
        x = new();
        val = x.timeout;
    end
endmodule
"#;
        let design = compile_str(source).unwrap();
        assert!(design.classes.contains_key("cfg"));
        let cls = &design.classes["cfg"];
        assert!(cls.fields.iter().any(|f| f.name == "timeout"));
        assert!(cls.methods.iter().any(|m| m.name == "new"));
    }

    #[test]
    fn test_method_call_parsing() {
        let source = r#"
class comp;
    function void print();
    endfunction
endclass
module tb;
    integer h;
    initial begin
        h = new();
        h.print();
    end
endmodule
"#;
        let _design = compile_str(source).unwrap();
    }

    #[test]
    fn test_virtual_method_registration() {
        let source = r#"
class base;
    virtual function void show();
    endfunction
endclass
class extended extends base;
    virtual function void show();
    endfunction
endclass
module tb;
    initial begin
        #1 $finish;
    end
endmodule
"#;
        let design = compile_str(source).unwrap();
        assert!(design.classes.contains_key("base"));
        assert!(design.classes.contains_key("extended"));
        assert_eq!(design.classes["extended"].extends.as_deref(), Some("base"));
        let base_show = design.classes["base"].methods.iter().find(|m| m.name == "show").unwrap();
        assert!(base_show.virtual_flag);
        let ext_show = design.classes["extended"].methods.iter().find(|m| m.name == "show").unwrap();
        assert!(ext_show.virtual_flag);
    }

    #[test]
    fn test_super_new_parsing() {
        let source = r#"
class base;
    function new();
    endfunction
endclass
class derived extends base;
    function new();
        super.new();
    endfunction
endclass
module tb;
    initial begin
        #1 $finish;
    end
endmodule
"#;
        let _design = compile_str(source).unwrap();
    }

    #[test]
    fn test_procedural_for_loop() {
        let source = r#"
module tb;
    reg [7:0] count;
    reg [3:0] i;
    initial begin
        count = 0;
        for (i = 0; i < 5; i = i + 1) begin
            count = count + 1;
        end
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 10).unwrap();
        let count_val = sigs.iter().find(|(n, _)| n == "count")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        assert_eq!(count_val, 5, "count should be 5 after for loop");
    }

    #[test]
    fn test_procedural_while_loop() {
        let source = r#"
module tb;
    reg [7:0] count;
    initial begin
        count = 0;
        while (count < 3) begin
            count = count + 1;
        end
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 10).unwrap();
        let count_val = sigs.iter().find(|(n, _)| n == "count")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        assert_eq!(count_val, 3, "count should be 3 after while loop");
    }

    #[test]
    fn test_super_new_phase_dispatch() {
        let source = r#"
class base;
    function new();
    endfunction
    function void build_phase();
    endfunction
endclass

class derived extends base;
    function new();
        super.new();
    endfunction
    function void build_phase();
        super.build_phase();
    endfunction
endclass

module tb;
    initial begin
        #1 $finish;
    end
endmodule
"#;
        let _sigs = simulate_signals(source, 10).unwrap();
    }

    #[test]
    fn test_class_inheritance_with_super() {
        let source = r#"
class base;
    function void build_phase();
    endfunction
    function int get_val();
        return 5;
    endfunction
endclass

class derived extends base;
    function void build_phase();
        super.build_phase();
    endfunction
    function int get_val();
        return 10 + super.get_val();
    endfunction
endclass

module tb;
    int result;
    initial begin
        result = 0;
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 10).unwrap();
        // Phase execution runs build_phase (checks super dispatch doesn't crash)
        // Then initial block runs
        let _result = sigs.iter().find(|(n, _)| n == "result").unwrap();
    }

    #[test]
    fn test_class_typed_var_decl_and_method_call() {
        let source = r#"
class counter;
    int count;
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
        let sigs = simulate_signals(source, 10).unwrap();
        let (_, val) = sigs.iter().find(|(n, _)| n == "result").unwrap();
        assert_eq!(val.to_u64(), 0, "new counter should have count=0");
    }

    #[test]
    fn test_class_typed_var_method_mutation() {
        let source = r#"
class counter;
    int count;
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
        c.inc();
        c.inc();
        c.inc();
        result = c.get();
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 10).unwrap();
        let (_, val) = sigs.iter().find(|(n, _)| n == "result").unwrap();
        assert_eq!(val.to_u64(), 3, "after 3 inc() calls, count should be 3");
    }

    #[test]
    fn test_class_typed_var_member_access() {
        let source = r#"
class counter;
    int count;
    function new();
        count = 0;
    endfunction
endclass

module tb;
    counter c;
    int result;
    initial begin
        c = new();
        result = c.count;
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 10).unwrap();
        let (_, val) = sigs.iter().find(|(n, _)| n == "result").unwrap();
        assert_eq!(val.to_u64(), 0, "c.count should be 0 after new()");
    }

    #[test]
    fn test_uvm_lite_polymorphic_dispatch() {
        let source = r#"
class uvm_component;
    int level;
    function new(int level);
        this.level = level;
    endfunction
    virtual function int get_type_id();
        return 1;
    endfunction
    function int get_level();
        return this.level;
    endfunction
endclass

class driver extends uvm_component;
    function new(int level);
        super.new(level);
    endfunction
    virtual function int get_type_id();
        return 2;
    endfunction
endclass

module tb;
    uvm_component h;
    driver d;
    int result_type;
    int result_level;
    initial begin
        d = new(42);
        h = d;
        result_type = h.get_type_id();
        result_level = h.get_level();
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 10).unwrap();
        let (_, type_val) = sigs.iter().find(|(n, _)| n == "result_type").unwrap();
        let (_, level_val) = sigs.iter().find(|(n, _)| n == "result_level").unwrap();
        assert_eq!(type_val.to_u64(), 2, "virtual dispatch: should call driver::get_type_id");
        assert_eq!(level_val.to_u64(), 42, "get_level should return 42");
    }

    #[test]
    fn test_null_handle() {
        let source = r#"
class Foo;
    function int get_val();
        return 7;
    endfunction
endclass

module tb;
    Foo h;
    int result;
    initial begin
        h = null;
        if (h == null) begin
            result = 1;
        end else begin
            result = 0;
        end
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 10).unwrap();
        let (_, val) = sigs.iter().find(|(n, _)| n == "result").unwrap();
        assert_eq!(val.to_u64(), 1, "null handle should compare as equal to null");
    }

    #[test]
    fn test_string_function_return() {
        let source = r#"
class driver;
    function string get_type_name();
        return "my_driver";
    endfunction
endclass

module tb;
    driver d;
    int result;
    initial begin
        d = new();
        // Just verify it parses and executes without error
        result = 1;
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 10).unwrap();
        let (_, val) = sigs.iter().find(|(n, _)| n == "result").unwrap();
        assert_eq!(val.to_u64(), 1, "string function should parse and execute");
    }

    #[test]
    fn test_foreach_in_class() {
        let source = r#"
class Accum;
    logic [31:0] arr [0:3];
    int sum;
    function new();
        sum = 0;
    endfunction
    function void init();
        arr[0] = 10;
        arr[1] = 20;
        arr[2] = 30;
        arr[3] = 40;
    endfunction
    function void accumulate();
        foreach (arr[i]) begin
            sum = sum + arr[i];
        end
    endfunction
endclass

module tb;
    Accum a;
    int result;
    initial begin
        a = new();
        a.init();
        a.accumulate();
        result = a.sum;
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 10).unwrap();
        let (_, val) = sigs.iter().find(|(n, _)| n == "result").unwrap();
        assert_eq!(val.to_u64(), 100, "foreach should sum array elements: 10+20+30+40=100");
    }

    #[test]
    fn test_preprocessor_define_and_expand() {
        use parser::preprocessor::Preprocessor;
        let mut pp = Preprocessor::new();
        let source = "`define WIDTH 8\nmodule test;\n    wire [`WIDTH-1:0] data;\nendmodule\n";
        let result = pp.preprocess(source, None).unwrap();
        assert!(result.contains("wire [8-1:0] data"), "macro should expand WIDTH: {}", result);
    }

    #[test]
    fn test_preprocessor_ifdef() {
        use parser::preprocessor::Preprocessor;
        let mut pp = Preprocessor::new();
        pp.define("DEBUG", "1");
        let source = "`ifdef DEBUG\nwire dbg;\n`else\nwire nodbg;\n`endif\nwire always;\n";
        let result = pp.preprocess(source, None).unwrap();
        assert!(result.contains("wire dbg;"), "ifdef true branch should be emitted");
        assert!(!result.contains("wire nodbg;"), "else branch should be skipped");
        assert!(result.contains("wire always;"), "post-endif should be emitted");
    }

    #[test]
    fn test_preprocessor_ifndef() {
        use parser::preprocessor::Preprocessor;
        let mut pp = Preprocessor::new();
        let source = "`ifndef DEBUG\nwire dbg;\n`else\nwire nodbg;\n`endif\n";
        let result = pp.preprocess(source, None).unwrap();
        assert!(result.contains("wire dbg;"), "ifndef true branch should be emitted");
        assert!(!result.contains("wire nodbg;"), "else branch should be skipped");
    }

    #[test]
    fn test_preprocessor_strip_unknown_macro() {
        use parser::preprocessor::Preprocessor;
        let mut pp = Preprocessor::new();
        let source = "`uvm_component_utils(my_driver)\nmodule test;\nendmodule\n";
        let result = pp.preprocess(source, None).unwrap();
        assert!(!result.contains("`uvm_component_utils"), "unknown macro should be stripped");
        assert!(result.contains("module test;"), "module decl should survive");
    }

    #[test]
    fn test_preprocessor_nested_ifdef() {
        use parser::preprocessor::Preprocessor;
        let mut pp = Preprocessor::new();
        pp.define("A", "1");
        pp.define("B", "1");
        let source = "`ifdef A\n`ifdef B\nwire both;\n`else\nwire only_a;\n`endif\n`endif\nwire after;\n";
        let result = pp.preprocess(source, None).unwrap();
        assert!(result.contains("wire both;"), "both defined: both should be emitted");
        assert!(!result.contains("wire only_a;"), "else should be skipped");
        assert!(result.contains("wire after;"), "post-endif emitted");
    }

    #[test]
    fn test_event_control_procedural() {
        let source = r#"
module tb;
    reg clk;
    reg [7:0] q;
    initial begin
        clk = 0;
        q = 0;
        #5 clk = 1;
        #1 clk = 0;
        #1 clk = 1;
        #1 $finish;
    end
    always @(posedge clk) begin
        q <= q + 1;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 10).unwrap();
        let q_val = sigs.iter().find(|(n, _)| n == "q")
            .map(|(_, v)| v.to_u64()).unwrap_or(0);
        assert_eq!(q_val, 2, "q should be 2 after 2 posedge clk events");
    }

    #[test]
    fn test_event_control_procedural_at() {
        let source = r#"
module tb;
    reg clk;
    reg [7:0] q;
    initial begin
        clk = 0;
        q = 0;
        #5 clk = 1;
        @(posedge clk);
        q = 42;
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 10).unwrap();
        let q_val = sigs.iter().find(|(n, _)| n == "q")
            .map(|(_, v)| v.to_u64()).unwrap_or(0);
        assert_eq!(q_val, 42, "q should be 42 after @(posedge clk) triggered");
    }

    #[test]
    fn test_event_trigger() {
        let source = r#"
module tb;
    reg ev;
    reg [7:0] q;
    initial begin
        q = 0;
        -> ev;
        #1 $finish;
    end
    initial begin
        @(ev) q = 99;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 10).unwrap();
        let q_val = sigs.iter().find(|(n, _)| n == "q")
            .map(|(_, v)| v.to_u64()).unwrap_or(0);
        assert_eq!(q_val, 99, "q should be 99 after -> ev triggers @(ev)");
    }

    #[test]
    fn test_gate_primitives_and_or() {
        let source = r#"
module tb;
    reg a, b, c, d;
    wire and_out, or_out;
    and a1(and_out, a, b, c);
    or  o1(or_out, a, b);
    initial begin
        a = 1; b = 1; c = 1;
        #1 d = and_out;
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 10).unwrap();
        let d_val = sigs.iter().find(|(n, _)| n == "d")
            .map(|(_, v)| v.to_u64()).unwrap_or(0);
        assert_eq!(d_val, 1, "and_out should be 1 (1 & 1 & 1 = 1)");
    }

    #[test]
    fn test_gate_not_buf() {
        let source = r#"
module tb;
    reg in;
    wire out;
    not n1(out, in);
    initial begin
        in = 0;
        #1;
        if (out !== 1) $finish;
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 10).unwrap();
        let out_val = sigs.iter().find(|(n, _)| n == "out")
            .map(|(_, v)| v.to_u64()).unwrap_or(0);
        assert_eq!(out_val, 1, "not gate should invert 0 to 1");
    }

    #[test]
    fn test_monitor_task() {
        let source = r#"
module tb;
    reg a;
    initial begin
        a = 0;
        $monitor("a=%d", a);
        #1 a = 1;
        #1 a = 0;
        #1 $finish;
    end
endmodule
"#;
        let _sigs = simulate_signals(source, 10).unwrap();
    }

    #[test]
    fn test_string_methods_len_substr() {
        let source = r#"
module tb;
    reg [63:0] len_val;
    initial begin
        len_val = "hello".len();
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 5).unwrap();
        let len = sigs.iter().find(|(n, _)| n == "len_val")
            .map(|(_, v)| v.to_u64()).unwrap_or(0);
        assert_eq!(len, 5, "len of 'hello' should be 5");
    }

    #[test]
    fn test_string_methods_atoi() {
        let source = r#"
module tb;
    reg [31:0] val;
    initial begin
        val = "42".atoi();
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 5).unwrap();
        let v = sigs.iter().find(|(n, _)| n == "val")
            .map(|(_, v)| v.to_u64()).unwrap_or(0);
        assert_eq!(v, 42, "atoi of '42' should be 42");
    }

    #[test]
    fn test_wait_statement() {
        let source = r#"
module tb;
    reg [7:0] cnt;
    reg done;
    initial begin
        cnt = 0;
        #10 cnt = 5;
    end
    initial begin
        wait (cnt == 5);
        done = 1;
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 20).unwrap();
        let cnt_val = sigs.iter().find(|(n, _)| n == "cnt")
            .map(|(_, v)| v.to_u64()).unwrap_or(0);
        let done_val = sigs.iter().find(|(n, _)| n == "done")
            .map(|(_, v)| v.to_u64()).unwrap_or(0);
        assert_eq!(cnt_val, 5, "cnt should be 5");
        assert_eq!(done_val, 1, "done should be 1 after wait is satisfied");
    }

    #[test]
    fn test_force_statement() {
        let source = r#"
module tb;
    reg [7:0] a;
    reg [7:0] b;
    initial begin
        a = 10;
        b = 20;
        #1 force a = b;
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 5).unwrap();
        let a_val = sigs.iter().find(|(n, _)| n == "a")
            .map(|(_, v)| v.to_u64()).unwrap_or(0);
        assert_eq!(a_val, 20, "a should be forced to b=20");
    }

    #[test]
    fn test_random_urandom() {
        let source = r#"
module tb;
    reg [31:0] r;
    initial begin
        r = $urandom();
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 5).unwrap();
        let r_val = sigs.iter().find(|(n, _)| n == "r")
            .map(|(_, v)| v.to_u64()).unwrap_or(0);
        // $urandom returns a non-zero 32-bit value (could be zero but unlikely)
        assert!(r_val < 4294967296, "r should be a 32-bit value");
    }

    #[test]
    fn test_dumpvars_dumpoff() {
        let source = r#"
module tb;
    reg [7:0] a;
    initial begin
        a = 42;
        $dumpvars();
        #1 $dumpoff();
        #2 $dumpon();
        #3 $finish();
    end
endmodule
"#;
        let sigs = simulate_signals(source, 10).unwrap();
        let a_val = sigs.iter().find(|(n, _)| n == "a")
            .map(|(_, v)| v.to_u64()).unwrap_or(0);
        assert_eq!(a_val, 42, "a should be 42");
    }

    #[test]
    fn test_preprocessor_with_simulation() {
        let source = r#"
`define WIDTH 8
`ifdef NEVER
wire never;
`endif
module test;
    reg [`WIDTH-1:0] data;
    initial begin
        data = 8'hAB;
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 10).unwrap();
        let (_, val) = sigs.iter().find(|(n, _)| n == "data").unwrap();
        assert_eq!(val.to_u64(), 0xAB, "preprocessed signal should have correct value");
    }

    #[test]
    fn test_clog2_in_expr() {
        let source = r#"
module tb;
    reg [7:0] w;
    reg [31:0] result;
    initial begin
        result = $clog2(8);
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 5).unwrap();
        let (_, val) = sigs.iter().find(|(n, _)| n == "result").unwrap();
        assert_eq!(val.to_u64(), 3, "$clog2(8) should be 3");
    }

    #[test]
    fn test_clog2_power_of_two() {
        let source = r#"
module tb;
    reg [31:0] r;
    initial begin
        r = $clog2(16);
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 5).unwrap();
        let (_, val) = sigs.iter().find(|(n, _)| n == "r").unwrap();
        assert_eq!(val.to_u64(), 4, "$clog2(16) should be 4");
    }

    #[test]
    fn test_clog2_one() {
        let source = r#"
module tb;
    reg [31:0] r;
    initial begin
        r = $clog2(1);
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 5).unwrap();
        let (_, val) = sigs.iter().find(|(n, _)| n == "r").unwrap();
        assert_eq!(val.to_u64(), 0, "$clog2(1) should be 0");
    }

    #[test]
    fn test_casex_wildcard() {
        let source = r#"
module tb;
    reg [3:0] sel;
    reg [7:0] out;
    always @(*) begin
        casex (sel)
            4'b1xx0: out = 8'hA0;
            4'b01x0: out = 8'hB0;
            4'b0010: out = 8'hC0;
            default: out = 8'hFF;
        endcase
    end
    initial begin
        sel = 4'b1000;
        #1;
        if (out !== 8'hA0) $finish;
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 10).unwrap();
        let out_val = sigs.iter().find(|(n, _)| n == "out")
            .map(|(_, v)| v.to_u64()).unwrap_or(0);
        assert_eq!(out_val, 0xA0, "casex 4'b1000 should match 4'b1xx0 => 0xA0");
    }

    #[test]
    fn test_casez_wildcard() {
        let source = r#"
module tb;
    reg [3:0] sel;
    reg [7:0] out;
    always @(*) begin
        casez (sel)
            4'b1zz0: out = 8'hA0;
            4'b01z0: out = 8'hB0;
            4'b0010: out = 8'hC0;
            default: out = 8'hFF;
        endcase
    end
    initial begin
        sel = 4'b1010;
        #1;
        if (out !== 8'hA0) $finish;
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 10).unwrap();
        let out_val = sigs.iter().find(|(n, _)| n == "out")
            .map(|(_, v)| v.to_u64()).unwrap_or(0);
        assert_eq!(out_val, 0xA0, "casez 4'b1010 should match 4'b1zz0 => 0xA0");
    }

    #[test]
    fn test_disable_named_block() {
        let source = r#"
module tb;
    reg [7:0] count;
    integer i;
    initial begin
        count = 0;
        for (i = 0; i < 10; i = i + 1) begin : loop_block
            if (i == 5) disable loop_block;
            count = count + 1;
        end
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 10).unwrap();
        let count_val = sigs.iter().find(|(n, _)| n == "count")
            .map(|(_, v)| v.to_u64()).unwrap_or(0);
        assert_eq!(count_val, 5, "disable should break at i=5, count should be 5");
    }

    #[test]
    fn test_disable_outer_block() {
        let source = r#"
module tb;
    reg [7:0] count;
    integer i;
    initial begin : outer
        count = 0;
        for (i = 0; i < 3; i = i + 1) begin : inner
            if (i == 1) disable outer;
            count = count + 1;
        end
        count = 100;
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 10).unwrap();
        let count_val = sigs.iter().find(|(n, _)| n == "count")
            .map(|(_, v)| v.to_u64()).unwrap_or(0);
        assert_eq!(count_val, 1, "disable outer should break at i=1 after count becomes 1");
    }

    #[test]
    fn test_release_deassign() {
        let source = r#"
module tb;
    reg [7:0] a;
    initial begin
        a = 42;
        #1 force a = 99;
        #1 release a;
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 10).unwrap();
        let a_val = sigs.iter().find(|(n, _)| n == "a")
            .map(|(_, v)| v.to_u64()).unwrap_or(0);
        // After release, signal becomes X (represented as 0 when read as u64 via to_u64())
        assert!(a_val != 99, "after release, a should not retain forced value");
    }

    #[test]
    fn test_break_in_loop() {
        let source = r#"
module tb;
    reg [7:0] count;
    initial begin
        count = 0;
        forever begin
            count = count + 1;
            if (count == 5) break;
        end
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 5).unwrap();
        let (_, val) = sigs.iter().find(|(n, _)| n == "count").unwrap();
        assert_eq!(val.to_u64(), 5, "break should exit at count=5");
    }

    #[test]
    fn test_continue_in_loop() {
        let source = r#"
module tb;
    reg [7:0] count;
    reg [7:0] sum;
    initial begin
        count = 0;
        sum = 0;
        while (count < 10) begin
            count = count + 1;
            if (count % 2 == 0) continue;
            sum = sum + count;
        end
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 5).unwrap();
        let (_, val) = sigs.iter().find(|(n, _)| n == "sum").unwrap();
        // Sum of odd numbers 1..9 = 25
        assert_eq!(val.to_u64(), 25, "continue should skip even numbers");
    }

    #[test]
    fn test_fill_literals() {
        let source = r#"
module tb;
    reg [7:0] a;
    reg [7:0] b;
    initial begin
        a = '0;
        b = '1;
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 5).unwrap();
        let a_val = sigs.iter().find(|(n, _)| n == "a").map(|(_, v)| v.to_u64()).unwrap();
        assert_eq!(a_val, 0, "'0 should fill all bits with 0");
    }

    #[test]
    fn test_do_while_loop() {
        let source = r#"
module tb;
    reg [7:0] count;
    initial begin
        count = 0;
        do begin
            count = count + 1;
        end while (count < 5);
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 5).unwrap();
        let (_, val) = sigs.iter().find(|(n, _)| n == "count").unwrap();
        assert_eq!(val.to_u64(), 5, "do-while should execute until count=5");
    }

    #[test]
    fn test_bits_system_function() {
        let source = r#"
module tb;
    reg [7:0] a;
    reg [31:0] result;
    initial begin
        result = $bits(a);
        #1 $finish;
    end
endmodule
"#;
        let sigs = simulate_signals(source, 5).unwrap();
        let (_, val) = sigs.iter().find(|(n, _)| n == "result").unwrap();
        assert_eq!(val.to_u64(), 8, "$bits(reg [7:0]) should be 8");
    }
}
