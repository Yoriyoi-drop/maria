// Seed 13: always_latch + always_comb mixed
module pulse_gen (
  input  logic clk, rst_n,
  input  logic in,
  output logic out
);
  logic [2:0] delay_q;

  always_latch begin
    if (rst_n) out = delay_q[2];
  end

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) delay_q <= '0;
    else delay_q <= {delay_q[1:0], in};
  end
endmodule