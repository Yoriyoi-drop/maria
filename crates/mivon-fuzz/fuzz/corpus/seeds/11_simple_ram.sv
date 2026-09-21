// Seed 11: memory + read/write + initial testbench pattern
module simple_ram #(
  parameter ADDR_W = 4,
  parameter DATA_W = 8
)(
  input  logic clk,
  input  logic we,
  input  logic [ADDR_W-1:0] addr,
  input  logic [DATA_W-1:0] din,
  output logic [DATA_W-1:0] dout
);
  logic [DATA_W-1:0] mem [2**ADDR_W];

  always_ff @(posedge clk) begin
    if (we) mem[addr] <= din;
    dout <= mem[addr];
  end
endmodule