// TB 09_func_task — function/task + recursive gcd cross-sim differential reference.
`timescale 1ns/1ps
module tb_calc;
  logic [7:0] a, b;
  logic [7:0] sum, prod, gcd_out;

  calc #(.W(8)) dut (.a(a), .b(b), .sum(sum), .prod(prod), .gcd_out(gcd_out));

  task check(input [7:0] ea, eb);
    logic [7:0] exp_sum;
    logic [7:0] exp_prod;
    a = ea; b = eb;
    exp_sum = ea + eb;
    exp_prod = ea * eb; // narrowed to 8 bit, sama dgn DUT
    #1;
    $display("ASRT_SUM=<%0d>", sum);
    $display("ASRT_PROD=<%0d>", prod);
    $display("ASRT_GCD=<%0d>", gcd_out);
    assert (sum == exp_sum) else $error("ASRT_SUM_BAD=<%0d>", sum);
    assert (prod == exp_prod) else $error("ASRT_PROD_BAD=<%0d>", prod);
    assert (gcd_out == gcd_ref(ea, eb)) else $error("ASRT_GCD_BAD=<%0d>", gcd_out);
  endtask

  function automatic [7:0] gcd_ref(input logic [7:0] x, y);
    if (y == 0) gcd_ref = x;
    else gcd_ref = gcd_ref(y, x % y);
  endfunction

  initial begin
    $display("ASRT_START tb_calc");
    check(8'd12, 8'd8);     // gcd = 4
    check(8'd35, 8'd10);    // gcd = 5
    check(8'd100, 8'd25);   // gcd = 25
    check(8'd7, 8'd13);     // gcd = 1 (coprime)
    #5;
    $display("ASRT_END tb_calc");
    $finish;
  end
endmodule