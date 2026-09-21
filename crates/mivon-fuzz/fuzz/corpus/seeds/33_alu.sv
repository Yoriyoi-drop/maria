// Seed 33: parameterized ALU (add/sub/and/or/xor/compare)
module alu #(parameter W = 8) (
  input  logic [W-1:0] a, b,
  input  logic [2:0] op,
  output logic [W-1:0] result,
  output logic zero, neg
);
  logic [W-1:0] r;
  always_comb begin
    unique case (op)
      3'd0: r = a + b;
      3'd1: r = a - b;
      3'd2: r = a & b;
      3'd3: r = a | b;
      3'd4: r = a ^ b;
      3'd5: r = (a < b) ? ((1 << (W-1))) | 1'b1 : 1'b0;
      3'd6: r = a << 1;
      default: r = a >> 1;
    endcase
  end
  assign result = r;
  assign zero = (r == '0);
  assign neg  = r[W-1];
endmodule