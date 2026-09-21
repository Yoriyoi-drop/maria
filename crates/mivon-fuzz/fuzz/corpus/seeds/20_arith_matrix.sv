// Seed 20: unsigned/signed arithmetic + multi-dim array
module arith #(parameter W = 8) (
  input  logic [W-1:0] a, b,
  input  logic sign_sel,
  output logic [2*W-1:0] mul_out,
  output logic [W-1:0] add_out,
  output logic overflow
);
  wire signed [W-1:0] sa = a;
  wire signed [W-1:0] sb = b;

  always_comb begin
    if (sign_sel) begin
      mul_out = sa * sb;
      add_out = sa + sb;
      overflow = (sa[W-1] == sb[W-1]) && (add_out[W-1] != sa[W-1]);
    end else begin
      mul_out = a * b;
      add_out = a + b;
      overflow = (a + b) < a;
    end
  end
endmodule

module matrix3x3 (
  input  logic [7:0] m [3][3],
  input  logic [7:0] v [3],
  output logic [7:0] r [3]
);
  integer i, j;
  always_comb begin
    for (i = 0; i < 3; i++) begin
      r[i] = 8'h0;
      for (j = 0; j < 3; j++) begin
        r[i] = r[i] + m[i][j] * v[j];
      end
    end
  end
endmodule

// Top: instansiasi arith + matrix3x3 (konteks sim multi-modul)
module top20 #(parameter W = 8) (
  input  logic [W-1:0] a, b,
  input  logic sign_sel,
  output logic [2*W-1:0] mul_out,
  output logic [W-1:0] add_out,
  output logic overflow,
  input  logic [7:0] m [3][3],
  input  logic [7:0] v [3],
  output logic [7:0] r [3]
);
  arith #(.W(W)) u_a (.a(a), .b(b), .sign_sel(sign_sel), .mul_out(mul_out), .add_out(add_out), .overflow(overflow));
  matrix3x3 u_m (.m(m), .v(v), .r(r));
endmodule