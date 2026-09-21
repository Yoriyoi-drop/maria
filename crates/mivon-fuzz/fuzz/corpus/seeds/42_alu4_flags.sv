// Seed 42: 4-bit ALU with status flags (carry/overflow)
module alu4 (
  input  logic [3:0] a, b,
  input  logic [2:0] op,
  output logic [3:0] y,
  output logic carry, overflow
);
  logic [4:0] add_r, sub_r;

  always_comb begin
    add_r = {1'b0, a} + {1'b0, b};
    sub_r = {1'b0, a} - {1'b0, b};
    unique case (op)
      3'd0: y = add_r[3:0];
      3'd1: y = sub_r[3:0];
      3'd2: y = a & b;
      3'd3: y = a | b;
      3'd4: y = a ^ b;
      3'd5: y = ~a;
      default: y = a;
    endcase
    carry = (op == 3'd0) ? add_r[4] : (op == 3'd1) ? ~sub_r[4] : 1'b0;
    overflow = (op == 3'd0) ? (a[3] == b[3] && y[3] != a[3])
              : (op == 3'd1) ? (a[3] != b[3] && y[3] != a[3]) : 1'b0;
  end
endmodule