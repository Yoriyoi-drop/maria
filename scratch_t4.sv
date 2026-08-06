module tb;
  logic [4:0] r;
  logic [31:0] idx = 7;
  initial begin
    r = $clog2(32)'(idx);
    $display("T4 r=%0d", r);
    $finish;
  end
endmodule
