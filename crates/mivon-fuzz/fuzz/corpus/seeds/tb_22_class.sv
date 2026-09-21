// TB 22_class — class randomize cross-sim differential reference.
// randomize() dengan constraint in-line — hasil nondeterministik antar
// simulator (PRNG berbeda). Oracle Wajib SKIP jika reference tak sama.
`timescale 1ns/1ps
module tb_class;
  logic clk = 0;
  logic [7:0] out;

  class_top dut (.clk(clk), .out(out));

  always #5 clk = ~clk;

  initial begin
    $display("ASRT_START tb_class");
    repeat (2) @(posedge clk);
    @(negedge clk);
    // randomize → addr di-paksa 0xAA (constraint in-line), data random
    $display("ASRT_OUT=<%02h>", out);
    $display("ASRT_END tb_class");
    $finish;
  end
endmodule