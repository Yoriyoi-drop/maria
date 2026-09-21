// TB 16_assert — assertion properties cross-sim differential reference.
// Murni property: tidak boleh push saat full / pop saat empty.
`timescale 1ns/1ps
module tb_fifo_assert;
  logic clk = 0;
  logic rst_n = 0;
  logic push = 0, pop = 0, full = 0, empty = 0;

  fifo_assert dut (.clk(clk), .rst_n(rst_n), .push(push), .pop(pop),
                   .full(full), .empty(empty));

  always #5 clk = ~clk;

  initial begin
    $display("ASRT_START tb_fifo_assert");
    rst_n = 0;
    #20;
    rst_n = 1;
    #10;
    // legal: push saat tidak full
    @(negedge clk);
    push = 1; full = 0; pop = 0; empty = 1;
    @(posedge clk); @(negedge clk);
    // legal: pop saat tidak empty
    push = 0; full = 0; pop = 1; empty = 0;
    @(posedge clk); @(negedge clk);
    // violation: push saat full → harus memicu assert
    push = 1; full = 1; pop = 0; empty = 0;
    @(posedge clk); @(negedge clk);
    $display("ASRT_VIOL_TRY=1");
    // legal lagi setelah itu
    push = 0; full = 0; pop = 0; empty = 1;
    @(posedge clk); @(negedge clk);
    $display("ASRT_END tb_fifo_assert");
    $finish;
  end
endmodule