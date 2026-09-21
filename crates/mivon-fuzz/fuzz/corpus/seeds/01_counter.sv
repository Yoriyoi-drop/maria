// Seed 01: counter parameterized + reset + enable
module counter #(parameter W = 8, parameter MAX = 255) (
  input  logic clk,
  input  logic rst_n,
  input  logic en,
  output logic [W-1:0] count,
  output logic done
);
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) count <= '0;
    else if (en) count <= count + 1'b1;
  end
  assign done = (count == MAX);
endmodule