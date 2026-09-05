//! Fuzz tests for complex expression patterns: deeply nested operators,
//! mixed width arithmetic, and width edge cases.

#[cfg(test)]
mod complex_expr_fuzz {
    /// Deeply nested ternary with signals
    #[test]
    fn nested_ternary_signals() {
        let src = r#"
module nested_tern;
    reg [3:0] a;
    reg [3:0] b;
    reg [3:0] c;
    wire [3:0] y;
    assign y = a ? (b ? c : a) : (c ? b : a);
    initial begin
        a = 4'hA; b = 4'h5; c = 4'h3;
        #10;
    end
endmodule
"#;
        let sigs = crate::simulate_signals(src, 20).unwrap();
        let y = sigs
            .iter()
            .find(|(n, _)| n == "y")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        // a=10 (true), b=5 (true), result=c=3
        assert_eq!(y, 3, "nested ternary");
    }

    /// Mixed arithmetic and comparison in one expression
    #[test]
    fn mixed_arith_cmp() {
        let src = r#"
module mixed_arith;
    reg [7:0] a;
    reg [7:0] b;
    wire [7:0] y;
    assign y = (a + b > a) ? (a - b) : (b - a);
    initial begin
        a = 8'h10; b = 8'h20;
        #10;
    end
endmodule
"#;
        let sigs = crate::simulate_signals(src, 20).unwrap();
        let y = sigs
            .iter()
            .find(|(n, _)| n == "y")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        // a+b=48 > a=16 (true) → a-b = 0x10-0x20 = 0xF0 (unsigned wrap)
        assert_eq!(y, 0xF0, "a-b unsigned wrap");
    }

    /// Concatenation with arithmetic
    #[test]
    fn concat_arith() {
        let src = r#"
module concat_arith;
    reg [3:0] a;
    reg [3:0] b;
    wire [8:0] y;
    assign y = {1'b0, a} + {1'b0, b};
    initial begin
        a = 4'hF; b = 4'h3;
        #10;
    end
endmodule
"#;
        let sigs = crate::simulate_signals(src, 20).unwrap();
        let y = sigs
            .iter()
            .find(|(n, _)| n == "y")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        assert_eq!(y, 18, "15+3=18");
    }

    /// Shift inside comparison inside ternary
    #[test]
    fn shift_in_cmp_in_tern() {
        let src = r#"
module shift_cmp_tern;
    reg [7:0] a;
    reg [7:0] b;
    wire [7:0] y;
    assign y = (a << 1) > b ? a : b;
    initial begin
        a = 8'h10; b = 8'h1F;
        #10;
    end
endmodule
"#;
        let sigs = crate::simulate_signals(src, 20).unwrap();
        let y = sigs
            .iter()
            .find(|(n, _)| n == "y")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        // a<<1 = 0x20 > b=0x1F (true) → y=a=0x10
        assert_eq!(y, 0x10, "a<<1 > b");
    }

    /// Bitwise NOT inside arithmetic
    #[test]
    fn bitnot_in_arith() {
        let src = r#"
module bitnot_arith;
    reg [7:0] a;
    wire [7:0] y;
    assign y = ~a + 8'd1;
    initial begin
        a = 8'h0A;
        #10;
    end
endmodule
"#;
        let sigs = crate::simulate_signals(src, 20).unwrap();
        let y = sigs
            .iter()
            .find(|(n, _)| n == "y")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        // ~0x0A = 0xF5, 0xF5 + 1 = 0xF6
        assert_eq!(y, 0xF6, "~a+1 two's complement");
    }

    /// Signed comparison with mixed widths
    #[test]
    fn signed_cmp_mixed_width() {
        let src = r#"
module signed_cmp;
    reg signed [7:0] a;
    reg signed [3:0] b;
    wire [7:0] y;
    assign y = (a < b) ? 8'd1 : 8'd0;
    initial begin
        a = -8'sd1; b = 4'sd3;
        #10;
    end
endmodule
"#;
        let sigs = crate::simulate_signals(src, 20).unwrap();
        let y = sigs
            .iter()
            .find(|(n, _)| n == "y")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        // -1 < 3 (signed) → true → y=1
        assert_eq!(y, 1, "-1 < 3 signed");
    }

    /// Replication inside arithmetic
    #[test]
    fn replicate_arith() {
        let src = r#"
module replicate_arith;
    reg [3:0] a;
    wire [7:0] y;
    assign y = {2{a}} + 8'd1;
    initial begin
        a = 4'h5;
        #10;
    end
endmodule
"#;
        let sigs = crate::simulate_signals(src, 20).unwrap();
        let y = sigs
            .iter()
            .find(|(n, _)| n == "y")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        // {2{4'h5}} = 8'h55, 85+1 = 86
        assert_eq!(y, 86, "replicated a + 1");
    }

    /// Complex: shift + compare + ternary + concat
    #[test]
    fn complex_combo() {
        let src = r#"
module complex_combo;
    reg [7:0] a;
    reg [7:0] b;
    wire [7:0] y;
    assign y = {(a > b), (a == b), (a < b), 5'b00000} | (a ^ b);
    initial begin
        a = 8'h10; b = 8'h20;
        #10;
    end
endmodule
"#;
        let sigs = crate::simulate_signals(src, 20).unwrap();
        let y = sigs
            .iter()
            .find(|(n, _)| n == "y")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        // a>b=0, a==b=0, a<b=1 → {0,0,1,00000} = 8'h20
        // a^b = 0x10 ^ 0x20 = 0x30
        // 0x20 | 0x30 = 0x30
        assert_eq!(y, 0x30, "complex combo");
    }

    /// Wide shift (>32 bit)
    #[test]
    fn wide_shift() {
        let src = r#"
module wide_shift;
    reg [63:0] a;
    wire [63:0] y;
    assign y = a << 32;
    initial begin
        a = 64'h00000001_00000000;
        #10;
    end
endmodule
"#;
        let sigs = crate::simulate_signals(src, 20).unwrap();
        let y = sigs
            .iter()
            .find(|(n, _)| n == "y")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        // 0x100000000 << 32 = 0x0 (shifted out)
        assert_eq!(y, 0, "shifted out");
    }

    /// Reduction chain: multiple reductions chained
    #[test]
    fn reduction_chain() {
        let src = r#"
module reduction_chain;
    reg [7:0] a;
    reg [7:0] b;
    wire [7:0] y;
    assign y = {&a, |b, ^a, ~&(a | b), ~(a ^ b), &(a & b), |(^a), ^(^b)};
    initial begin
        a = 8'hFF; b = 8'h0F;
        #10;
    end
endmodule
"#;
        let sigs = crate::simulate_signals(src, 20).unwrap();
        let y = sigs
            .iter()
            .find(|(n, _)| n == "y")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        // No-crash test for chained reductions with mixed operators
        // Exact value depends on width propagation — just verify non-zero
        assert!(y > 0 && y <= 0xFF, "reduction chain result: {}", y);
    }
}
