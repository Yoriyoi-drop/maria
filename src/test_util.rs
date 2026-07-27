//! Test utilities — AST comparison, regression helpers.

use crate::compare_asts;

/// Assert that two designs are structurally equivalent.
#[macro_export]
macro_rules! assert_designs_eq {
    ($a:expr, $b:expr) => {
        let diffs = $crate::compare_asts(&$a, &$b);
        assert!(
            diffs.is_empty(),
            "Designs differ:\n  {}",
            diffs.join("\n  ")
        );
    };
}

/// Assert that compiling the same source twice produces identical designs.
#[macro_export]
macro_rules! assert_compile_stable {
    ($source:expr) => {
        let design_a = $crate::compile_str($source).unwrap();
        let design_b = $crate::compile_str($source).unwrap();
        let diffs = $crate::compare_asts(&design_a, &design_b);
        assert!(
            diffs.is_empty(),
            "Compilation not deterministic:\n  {}",
            diffs.join("\n  ")
        );
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_compile_deterministic() {
        let source = r#"
module counter(
    input clk,
    input rst_n,
    output reg [7:0] count
);
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n)
            count <= 8'h00;
        else
            count <= count + 8'h01;
    end
endmodule
"#;
        assert_compile_stable!(source);
    }

    #[test]
    fn test_compile_deterministic_multi_module() {
        let source = r#"
module top(input clk, input rst_n, output [7:0] out);
    wire [7:0] w1, w2;
    sub u1 (.clk(clk), .in(8'h01), .out(w1));
    sub u2 (.clk(clk), .in(8'h02), .out(w2));
    assign out = w1 + w2;
endmodule

module sub(input clk, input [7:0] in, output reg [7:0] out);
    always_ff @(posedge clk) out <= in;
endmodule
"#;
        assert_compile_stable!(source);
    }

    #[test]
    fn test_ast_compare_equal() {
        let source = "module m(input a, output b); assign b = a; endmodule";
        let a = crate::compile_str(source).unwrap();
        let b = crate::compile_str(source).unwrap();
        let diffs = crate::compare_asts(&a, &b);
        assert!(diffs.is_empty(), "Expected no diffs, got: {:?}", diffs);
    }
}
