// Seed 40: parametrized generic shift/multiply unit
module mul_shift #(parameter W = 8) (
  input  logic [W-1:0] a, shift_amt,
  input  logic sel_mul, sel_shl, sel_shr,
  output logic [2*W-1:0] result
);
  logic [2*W-1:0] r;
  always_comb begin
    r = '0;
    if (sel_mul) r = a * a;
    else if (sel_shl) r = a << shift_amt;
    else if (sel_shr) r = a >> shift_amt;
  end
  assign result = r;
endmodule

module modulo #(parameter W = 8) (
  input  logic [W-1:0] a, b,
  output logic [W-1:0] rem
);
  always_comb begin
    if (b == '0) rem = '0;
    else rem = a % b;
  end
endmodule

// Top: mul_shift + modulo
module top_ms #(parameter W = 8) (
  input  logic [W-1:0] a, sh, b,
  input  logic m, s, r,
  output logic [2*W-1:0] result,
  output logic [W-1:0] rem
);
  mul_shift #(.W(W)) u_ms (.a(a), .shift_amt(sh), .sel_mul(m), .sel_shl(s), .sel_shr(r), .result(result));
  modulo #(.W(W)) u_mod (.a(a), .b(b), .rem(rem));
endmodule