// Seed 03: mux 4-to-1 + decoder, always_comb + case/casez
module mux4 (
  input  logic [1:0] sel,
  input  logic [3:0] d0, d1, d2, d3,
  output logic [3:0] y
);
  always_comb begin
    unique case (sel)
      2'd0: y = d0;
      2'd1: y = d1;
      2'd2: y = d2;
      2'd3: y = d3;
    endcase
  end
endmodule

module decoder3x8 (
  input  logic [2:0] in,
  output logic [7:0] out
);
  always_comb begin
    out = 8'b0;
    casez (in)
      3'b??1: out[0] = 1'b1;
      3'b?1?: out[1] = 1'b1;
      3'b1??: out[2] = 1'b1;
      default: out[3] = 1'b1;
    endcase
  end
endmodule

// Top: instansiasi mux + decoder
module top03 (
  input  logic [1:0] sel,
  input  logic [3:0] d0, d1, d2, d3,
  output logic [3:0] mux_y,
  input  logic [2:0] dec_in,
  output logic [7:0] dec_out
);
  mux4 u_m (.sel(sel), .d0(d0), .d1(d1), .d2(d2), .d3(d3), .y(mux_y));
  decoder3x8 u_d (.in(dec_in), .out(dec_out));
endmodule