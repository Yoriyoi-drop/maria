module tb;
  logic [31:0] idx = 7;
  logic [4:0] r;
  initial begin
    r = $clog2(32)'(idx);
    $display("T5a display=%0d", $clog2(32)'(idx));
    $display("T5b r=%0d", r);
    $display("T5c lit=%0d", $clog2(32)'(7));
    $finish;
  end
endmodule
