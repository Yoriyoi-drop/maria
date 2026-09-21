// Seed 38: RAM with two read ports + write (dual-port style)
module dpram #(
  parameter DEPTH = 16,
  parameter W = 8
)(
  input  logic clk,
  input  logic we_a, we_b,
  input  logic [$clog2(DEPTH)-1:0] addr_a, addr_b,
  input  logic [W-1:0] din_a, din_b,
  output logic [W-1:0] dout_a, dout_b
);
  logic [W-1:0] mem [DEPTH];

  always_ff @(posedge clk) begin
    if (we_a) mem[addr_a] <= din_a;
    if (we_b) mem[addr_b] <= din_b;
    dout_a <= mem[addr_a];
    dout_b <= mem[addr_b];
  end
endmodule