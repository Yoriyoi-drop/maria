// TB 12_divider — restoring divider cross-sim differential reference.
// Menghindari loop tak-terminasi: start sekali, pastikan selesai.
`timescale 1ns/1ps
module tb_divider;
  logic clk = 0;
  logic rst_n = 0;
  logic start = 0;
  logic [7:0] dividend = 0;
  logic [7:0] divisor = 0;
  logic [7:0] quotient;
  logic done;

  divider #(.W(8)) dut (
    .clk(clk), .rst_n(rst_n), .start(start),
    .dividend(dividend), .divisor(divisor),
    .quotient(quotient), .done(done)
  );

  always #5 clk = ~clk;

  task run_div(input [7:0] a, b);
    $display("ASRT_DIV_START=<%0d>", a);
    dividend = a; divisor = b;
    @(negedge clk);
    start = 1;
    @(posedge clk); @(negedge clk);
    start = 0;
    // W=8 → max 8 cycle + 1 done
    repeat (9) @(posedge clk);
    @(negedge clk);
    assert (done === 1) else $error("ASRT_DIV_NOTDONE=<%0d>", done);
    assert (quotient == (a / b)) else $error("ASRT_DIV_Q_BAD=<%0d, expect %0d>", quotient, a/b);
    $display("ASRT_DIV_Q=<%0d>", quotient);
  endtask

  initial begin
    $display("ASRT_START tb_divider");
    rst_n = 0;
    #20;
    rst_n = 1;
    #10;
    run_div(8'd100, 8'd4);   // 25
    run_div(8'd255, 8'd16);  // 15
    run_div(8'd121, 8'd11);  // 11
    #5;
    $display("ASRT_END tb_divider");
    $finish;
  end
endmodule