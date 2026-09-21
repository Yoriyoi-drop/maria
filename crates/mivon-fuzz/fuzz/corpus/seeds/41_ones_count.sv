// Seed 41: thermometer code + ones counter
module ones_count #(parameter W = 8) (
  input  logic [W-1:0] in,
  output logic [3:0] cnt
);
  integer i;
  always_comb begin
    cnt = 4'd0;
    for (i = 0; i < W; i++) begin
      cnt = cnt + in[i];
    end
  end
endmodule

module thermo #(parameter W = 8) (
  input  logic [W-1:0] in,
  output logic [W-1:0] out
);
  integer j;
  always_comb begin
    out = '0;
    for (j = 0; j < W; j++) begin
      if (in[j]) out[j] = 1'b1;
    end
  end
endmodule

// Top: ones counter + thermometer
module top_ones #(parameter W = 8) (
  input  logic [W-1:0] in,
  output logic [3:0] cnt,
  output logic [W-1:0] thermo_out
);
  ones_count #(.W(W)) u_cnt (.in(in), .cnt(cnt));
  thermo #(.W(W)) u_th (.in(in), .out(thermo_out));
endmodule