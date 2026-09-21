// TB 21_fork — fork/join_any cross-sim differential reference.
`timescale 1ns/1ps
module tb_fork;
  logic clk = 0;
  logic [3:0] result;

  fork_demo dut (.clk(clk), .result(result));

  always #5 clk = ~clk;

  initial begin
    $display("ASRT_START tb_fork");
    repeat (3) @(posedge clk);
    @(negedge clk);
    // join_any fires after first branch (@(posedge clk) → 4'h1) then #5 → 4'hF
    assert (result == 4'hF) else $error("ASRT_FORK_BAD=<%0h>", result);
    $display("ASRT_FORK=<%0h>", result);
    $display("ASRT_END tb_fork");
    $finish;
  end
endmodule