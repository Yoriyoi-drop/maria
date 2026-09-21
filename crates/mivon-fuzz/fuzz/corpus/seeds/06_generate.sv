// Seed 06: generate for + parameterized bus inverter
module inverter_bus #(parameter N = 8) (
  input  logic [N-1:0] a,
  output logic [N-1:0] b
);
  genvar i;
  generate
    for (i = 0; i < N; i++) begin : gen_inv
      assign b[i] = ~a[i];
    end
  endgenerate
endmodule

module tree_and #(parameter N = 4) (
  input  logic [N-1:0] in,
  output logic out
);
  genvar j;
  logic [N-1:0] partial;
  generate
    for (j = 0; j < N; j++) begin : gen_init
      assign partial[j] = in[j];
    end
  endgenerate
  assign out = &partial;
endmodule

// Top: instansiasi inverter + tree_and
module top06 #(parameter N = 8, parameter M = 4) (
  input  logic [N-1:0] a,
  output logic [N-1:0] inv_out,
  input  logic [M-1:0] tin,
  output logic t_out
);
  inverter_bus #(.N(N)) u_i (.a(a), .b(inv_out));
  tree_and #(.N(M)) u_t (.in(tin), .out(t_out));
endmodule