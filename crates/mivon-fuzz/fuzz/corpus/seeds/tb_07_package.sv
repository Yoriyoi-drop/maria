// TB 07_package — package/typedef/enum + import cross-sim differential reference.
`timescale 1ns/1ps
module tb_package;
  logic clk = 0;
  logic rst_n = 0;
  logic [7:0] out;

  pkg_user #() dut (.clk(clk), .rst_n(rst_n), .out(out));

  always #5 clk = ~clk;

  initial begin
    $display("ASRT_START tb_package");
    rst_n = 0;
    #20;
    rst_n = 1;
    #10;
    @(negedge clk);
    // reset → out=0, st=IDLE → next cycle st=RUN
    assert (out == 8'h00) else $error("ASRT_OUT0_BAD=<%02h>", out);

    // IDLE→RUN (out unchanged)
    @(posedge clk); @(negedge clk);
    assert (out == 8'h00) else $error("ASRT_OUT_IDLE_RUN_BAD=<%02h>", out);

    // RUN: st→DONE, out <= out+1
    @(posedge clk); @(negedge clk);
    assert (out == 8'h01) else $error("ASRT_OUT_RUN_BAD=<%02h>", out);

    // DONE→IDLE
    @(posedge clk); @(negedge clk);
    assert (out == 8'h01) else $error("ASRT_OUT_DONE_BAD=<%02h>", out);

    $display("ASRT_END tb_package");
    $finish;
  end
endmodule