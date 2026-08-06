module tb;
  localparam int LP = $clog2(32);
  logic [31:0] idx = 7;
  initial begin
    $display("T6a LP=%0d", LP);
    $display("T6b numcast=%0d", 8'(7));
    $display("T6c clogcast=%0d", $clog2(32)'(7));
    $display("T6d sigcast=%0d", 8'(idx));
    $display("T6e sigclog=%0d", $clog2(32)'(idx));
    $finish;
  end
endmodule
