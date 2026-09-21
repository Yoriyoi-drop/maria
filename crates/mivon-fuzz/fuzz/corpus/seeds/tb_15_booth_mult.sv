// TB 15_booth_mult — Booth multiplier cross-sim differential reference.
`timescale 1ns/1ps
module tb_booth_mult;
  logic [7:0] a, b;
  logic [15:0] p;

  booth_mult #(.W(8)) dut (.a(a), .b(b), .p(p));

  task check(input [7:0] ea, eb);
    a = ea; b = eb;
    #1;
    $display("ASRT_P=<%0d>", p);
    assert (p == ($signed(ea) * $signed(eb))) else
      $error("ASRT_MULT_BAD=<%0d>>, expect %0d", p, $signed(ea)*$signed(eb));
  endtask

  initial begin
    $display("ASRT_START tb_booth_mult");
    check(8'd55, 8'd3);       // 165
    check(8'd100, 8'd100);    // 10000
    check(8'd15, 8'd15);      // 225
    check(8'd0, 8'd255);      // 0
    #5;
    $display("ASRT_END tb_booth_mult");
    $finish;
  end
endmodule