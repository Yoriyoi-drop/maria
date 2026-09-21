// TB 23_string — string ops + $sformatf + hash cross-sim differential reference.
`timescale 1ns/1ps
module tb_string;
  logic clk = 0;
  logic [31:0] hash;

  str_demo dut (.clk(clk), .hash(hash));

  always #5 clk = ~clk;

  initial begin
    $display("ASRT_START tb_string");
    repeat (5) @(posedge clk);
    @(negedge clk);
    // hash = 31-hash of "hello mivon" — konsisten antar simulator
    $display("ASRT_HASH=<%08h>", hash);
    $display("ASRT_END tb_string");
    $finish;
  end
endmodule