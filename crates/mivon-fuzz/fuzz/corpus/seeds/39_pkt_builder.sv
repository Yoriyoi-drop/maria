// Seed 39: param struct + bit-packed counter driver
module pkt_builder #(parameter W = 8) (
  input  logic clk, rst_n,
  input  logic en,
  input  logic [7:0] src,
  output logic [W-1:0] payload,
  output logic valid
);
  logic [W-1:0] cnt;

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      cnt <= '0;
      valid <= 1'b0;
    end else begin
      valid <= en;
      if (en) cnt <= cnt + 1;
    end
  end

  assign payload = cnt ^ src;
endmodule