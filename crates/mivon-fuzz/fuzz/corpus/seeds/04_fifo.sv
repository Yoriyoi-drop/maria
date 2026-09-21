// Seed 04: synchronous FIFO dengan counter
module fifo #(
  parameter DEPTH = 4,
  parameter WIDTH = 8
)(
  input  logic clk, rst_n,
  input  logic push, pop,
  input  logic [WIDTH-1:0] din,
  output logic [WIDTH-1:0] dout,
  output logic full, empty
);
  logic [WIDTH-1:0] mem [DEPTH];
  integer head = 0, tail = 0, cnt = 0;

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      head <= 0; tail <= 0; cnt <= 0;
    end else begin
      if (push && !full) begin
        mem[tail % DEPTH] <= din;
        tail <= tail + 1;
        cnt <= cnt + 1;
      end
      if (pop && !empty) begin
        head <= head + 1;
        cnt <= cnt - 1;
      end
    end
  end

  assign dout  = mem[head % DEPTH];
  assign full  = (cnt == DEPTH);
  assign empty = (cnt == 0);
endmodule