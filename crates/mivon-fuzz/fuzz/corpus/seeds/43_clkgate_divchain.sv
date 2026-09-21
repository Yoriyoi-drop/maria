// Seed 43: clock gate + glitch-free divider chain
module clk_gate (
  input  logic clk, en,
  output logic gclk
);
  logic en_q;
  always_latch begin
    if (!clk) en_q <= en;
  end
  assign gclk = clk & en_q;
endmodule

module div_chain #(parameter N = 4) (
  input  logic clk, rst_n,
  output logic [N-1:0] out
);
  logic [N-1:0] div;
  genvar i;
  generate
    for (i = 0; i < N; i++) begin : g
      if (i == 0) begin
        always_ff @(posedge clk or negedge rst_n) begin
          if (!rst_n) div[0] <= 1'b0;
          else div[0] <= ~div[0];
        end
      end else begin
        always_ff @(posedge div[i-1] or negedge rst_n) begin
          if (!rst_n) div[i] <= 1'b0;
          else div[i] <= ~div[i];
        end
      end
    end
  endgenerate
  assign out = div;
endmodule

// Top: clock gate + divider chain
module top_cg #(parameter N = 4) (
  input  logic clk, rst_n, en,
  output logic gclk,
  output logic [N-1:0] div_out
);
  clk_gate u_g (.clk(clk), .en(en), .gclk(gclk));
  div_chain #(.N(N)) u_d (.clk(clk), .rst_n(rst_n), .out(div_out));
endmodule