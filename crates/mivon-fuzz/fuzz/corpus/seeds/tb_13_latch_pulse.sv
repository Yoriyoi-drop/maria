// TB 13_latch_pulse — always_latch + comb cross-sim differential reference.
`timescale 1ns/1ps
module tb_pulse_gen;
  logic clk = 0;
  logic rst_n = 0;
  logic in = 0;
  logic out;

  pulse_gen dut (.clk(clk), .rst_n(rst_n), .in(in), .out(out));

  always #5 clk = ~clk;

  initial begin
    $display("ASRT_START tb_pulse_gen");
    rst_n = 0;
    #20;
    // during reset, out latches delay_q[2]=0
    assert (out === 0) else $error("ASRT_OUT_RST_BAD=<%0d>", out);
    #5;
    rst_n = 1;
    // feed in=1 for 3 cycles → delay_q=111 → out=1
    @(negedge clk);
    in = 1;
    repeat (3) begin @(posedge clk); @(negedge clk); end
    assert (out === 1) else $error("ASRT_OUT_111_BAD=<%0d>", out);
    $display("ASRT_OUT_111=ok");

    // in=0 → delay_q=110 → out=1 (delay_q[2] still 1)
    in = 0;
    @(posedge clk); @(negedge clk);
    assert (out === 1) else $error("ASRT_OUT_110_BAD=<%0d>", out);
    // next → delay_q=100 → out=0
    @(posedge clk); @(negedge clk);
    assert (out === 0) else $error("ASRT_OUT_100_BAD=<%0d>", out);
    $display("ASRT_DECAY=ok");

    $display("ASRT_END tb_pulse_gen");
    $finish;
  end
endmodule