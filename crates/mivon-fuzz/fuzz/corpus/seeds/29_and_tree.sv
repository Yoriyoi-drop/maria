// Seed 29: and_tree NON-recursive (recursive instantiation mivon belum
// support). Iterative reduction via generate.
module and_tree #(parameter N = 8) (
  input  logic [N-1:0] in,
  output logic out
);
  logic [N-1:0] stage;
  genvar i;
  generate
    for (i = 0; i < N; i++) begin : g_init
      assign stage[i] = in[i];
    end
  endgenerate
  assign out = &stage;
endmodule

// Top: instansiasi and_tree
module top29 #(parameter N = 8) (
  input  logic [N-1:0] in,
  output logic out
);
  and_tree #(.N(N)) u (.in(in), .out(out));
endmodule