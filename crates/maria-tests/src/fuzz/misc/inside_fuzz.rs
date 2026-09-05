//! Fuzz tests for `inside` operator edge cases.
//! Inside checks if a value is within a set/range of values.

#[cfg(test)]
mod inside_fuzz {
    /// Basic inside with range
    #[test]
    fn inside_range() {
        let src = r#"
module inside_range;
    reg [3:0] a;
    wire y;
    assign y = a inside {[4'd3:4'd7]};
    initial begin
        a = 4'd5;
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
        assert_eq!(y, 1, "5 is inside [3:7]");
    }

    /// Inside with multiple ranges (explicit list syntax may not be supported)
    #[test]
    fn inside_list() {
        let src = r#"
module inside_list;
    reg [3:0] a;
    wire y;
    assign y = a inside {[4'd1:4'd1], [4'd3:4'd3], [4'd5:4'd5], [4'd7:4'd7]};
    initial begin
        a = 4'd3;
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
        assert_eq!(y, 1, "3 is inside {{1}},{{3}},{{5}},{{7}}");
    }

    /// Inside with negated (not inside)
    #[test]
    fn inside_not() {
        let src = r#"
module inside_not;
    reg [3:0] a;
    wire y;
    assign y = !(a inside {[4'd0:4'd3]});
    initial begin
        a = 4'd5;
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
        assert_eq!(y, 1, "5 is NOT inside [0:3]");
    }

    /// Inside with mixed range and values
    #[test]
    fn inside_mixed() {
        let src = r#"
module inside_mixed;
    reg [7:0] a;
    wire y;
    assign y = a inside {[8'd0:8'd10], 8'd20, 8'd30, [8'd100:8'd200]};
    initial begin
        a = 8'd20;
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
        assert_eq!(y, 1, "20 is inside {{0-10, 20, 30, 100-200}}");
    }

    /// Inside with wide value
    #[test]
    fn inside_wide() {
        let src = r#"
module inside_wide;
    reg [7:0] a;
    wire y;
    assign y = a inside {[8'd0:8'd255]};
    initial begin
        a = 8'd128;
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
        assert_eq!(y, 1, "128 is inside [0:255]");
    }

    /// Inside with value at boundary
    #[test]
    fn inside_boundary() {
        let src = r#"
module inside_boundary;
    reg [3:0] a;
    wire y;
    assign y = a inside {[4'd3:4'd7]};
    initial begin
        a = 4'd3;
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
        assert_eq!(y, 1, "3 is at lower boundary");
    }

    /// Inside value outside all ranges
    #[test]
    fn inside_outside() {
        let src = r#"
module inside_outside;
    reg [3:0] a;
    wire y;
    assign y = a inside {[4'd0:4'd2], [4'd5:4'd7]};
    initial begin
        a = 4'd4;
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
        assert_eq!(y, 0, "4 is NOT inside [0:2] or [5:7]");
    }
}
