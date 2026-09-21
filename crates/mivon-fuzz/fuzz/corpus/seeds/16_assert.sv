// Seed 16: assertion binding + cover property
module fifo_assert (
  input logic clk, rst_n,
  input logic push, pop,
  input logic full, empty
);
  // tidak boleh push saat full
  assert property (@(posedge clk) disable iff (!rst_n)
    !(push && full));
  // tidak boleh pop saat empty
  assert property (@(posedge clk) disable iff (!rst_n)
    !(pop && empty));
  cover property (@(posedge clk) push && !full);
  cover property (@(posedge clk) pop && !empty);
endmodule