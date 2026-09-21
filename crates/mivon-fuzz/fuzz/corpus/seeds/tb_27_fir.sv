// TB 27_fir — systolic FIR generate cross-sim differential reference.
// TAPS=4, W=8. Reset → feed x=0x2A (@taps[0]) → pipeline shifts; sample y.
`timescale 1ns/1ps
module tb_fir;
  logic clk = 0;
  logic rst_n = 0;
  logic [7:0] x = 0;
  logic [20:0] y;

  fir_systolic #(.TAPS(4), .W(8)) dut (
    .clk(clk), .rst_n(rst_n), .x(x), .y(y)
  );

  always #5 clk = ~clk;

  initial begin
    $display("ASRT_START tb_fir");
    rst_n = 0;
    #20;
    rst_n = 1;
    #10;
    @(negedge clk);

    // feed x constant 0x2A for 4 cycles → acc[TAPS-1] = sum of taps*(i+1)
    x = 8'h2A;
    repeat (6) begin @(posedge clk); @(negedge clk); end
    $display("ASRT_Y=<%0d>", y);

    x = 8'h00;
    repeat (4) begin @(posedge clk); @(negedge clk); end
    $display("ASRT_Y_ZERO=<%0d>", y);

    $display("ASRT_END tb_fir");
    $finish;
  end
endmodule