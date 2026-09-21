// Seed 34: synchronous down counter with load + compare output
module down_counter #(parameter W = 8) (
  input  logic clk, rst_n,
  input  logic load, dec,
  input  logic [W-1:0] d,
  output logic [W-1:0] q,
  output logic underflow
);
  logic [W-1:0] cnt;

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) cnt <= '0;
    else begin
      if (load) cnt <= d;
      else if (dec && cnt > 0) cnt <= cnt - 1;
    end
  end

  assign q = cnt;
  assign underflow = (cnt == 0) & dec;
endmodule