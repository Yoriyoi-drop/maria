//! Fuzz tests for reduction operators applied to ternary expressions with
//! signal branches. Regression for the const_fold_width bug where
//! `&(1 ? 1'b1 : b)` was incorrectly folded to `Const(1, w=1)` causing
//! `&(1'b1)` = 1 instead of the correct `&(2'b01)` = 0.
//!
//! Root cause: `const_fold_width` for TernaryOp returned the true-branch
//! width when the false branch was a signal (width unknown), instead of
//! returning None to force fallback width computation.

#[cfg(test)]
mod reduction_ternary_fuzz {
    /// &(ternary) with signal false branch — the original crash bug.
    /// Maria gave y=1, Icarus gives y=0.
    #[test]
    fn red_ternary_and_signal_false() {
        let src = r#"
module red_ternary_and;
    reg [1:0] b;
    wire [1:0] y;
    assign y = &(1 ? 1'b1 : b);
    initial begin
        b = 2'b10;
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
        assert_eq!(y, 0, "y should be 0 ( &(1'b01) = 0 ), got {}", y);
    }

    /// |(ternary) with signal false branch — same pattern, different reduction.
    #[test]
    fn red_ternary_or_signal_false() {
        let src = r#"
module red_ternary_or;
    reg [1:0] b;
    wire [1:0] y;
    assign y = |(1 ? 1'b0 : b);
    initial begin
        b = 2'b10;
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
        // |(1'b0) = 0, but with 2-bit context: 2'b10 has bit 1 set
        // Actually ternary true branch is taken: 1'b0, |(0) = 0
        assert_eq!(y, 0, "y should be 0 ( |(1'b0) = 0 ), got {}", y);
    }

    /// ^(ternary) with signal false branch — XOR reduction.
    #[test]
    fn red_ternary_xor_signal_false() {
        let src = r#"
module red_ternary_xor;
    reg [1:0] b;
    wire [1:0] y;
    assign y = ^(1 ? 2'b11 : b);
    initial begin
        b = 2'b10;
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
        // ^(2'b11) = 1^1 = 0, then 2'b00
        assert_eq!(y, 0, "y should be 0 ( ^(2'b11) = 0 ), got {}", y);
    }

    /// &(ternary) with wider signal false branch — different widths.
    #[test]
    fn red_ternary_and_wide_signal() {
        let src = r#"
module red_ternary_and_wide;
    reg [3:0] b;
    wire [3:0] y;
    assign y = &(1 ? 1'b1 : b);
    initial begin
        b = 4'b1111;
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
        // &(1'b1) = 1, but ternary width = max(1,4) = 4
        // true branch 1-bit zero-extended to 4-bit = 4'b0001
        // &(4'b0001) = 0
        assert_eq!(y, 0, "y should be 0, got {}", y);
    }

    /// !(ternary) with signal false branch — logical NOT.
    #[test]
    fn red_ternary_not_signal_false() {
        let src = r#"
module red_ternary_not;
    reg [1:0] b;
    wire [1:0] y;
    assign y = !(1 ? 1'b1 : b);
    initial begin
        b = 2'b10;
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
        // !(1'b1) = 0, !(2'b01) = 0 (bitwise ! of non-zero = 0 for 1-bit)
        assert_eq!(y, 0, "y should be 0 ( !(1'b1) = 0 ), got {}", y);
    }

    /// &(ternary) inside always_comb — should trigger sensitivity list correctly.
    #[test]
    fn red_ternary_always_comb() {
        let src = r#"
module red_ternary_ac;
    reg [1:0] b;
    reg [1:0] y;
    always_comb begin
        y = &(1 ? 1'b1 : b);
    end
    initial begin
        b = 2'b10;
        #10;
        b = 2'b01;
        #10;
    end
endmodule
"#;
        let sigs = crate::simulate_signals(src, 30).unwrap();
        let y = sigs
            .iter()
            .find(|(n, _)| n == "y")
            .map(|(_, v)| v.to_u64())
            .unwrap();
        // Always const-folded to 0 regardless of b
        assert_eq!(y, 0, "y should always be 0, got {}", y);
    }

    /// &(ternary) with both-constant branches — should still work.
    #[test]
    fn red_ternary_both_constant() {
        let src = r#"
module red_ternary_const;
    wire [1:0] y;
    assign y = &(1 ? 2'b11 : 2'b00);
    initial begin
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
        // &(2'b11) = 1
        assert_eq!(y, 1, "y should be 1 ( &(2'b11) = 1 ), got {}", y);
    }

    /// Nested reduction in ternary with signal — complex pattern.
    #[test]
    fn red_ternary_nested() {
        let src = r#"
module red_ternary_nested;
    reg [2:0] a;
    reg [2:0] b;
    wire [2:0] y;
    assign y = {(&(a)), |(b), ^(1 ? 1'b0 : a)};
    initial begin
        a = 3'b111;
        b = 3'b101;
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
        // &(3'b111) = 1, |(3'b101) = 1, ^(1'b0) = 0
        // y = {1,1,0} = 3'b110 = 6
        assert_eq!(y, 6, "y should be 6 (110b), got {}", y);
    }
}
