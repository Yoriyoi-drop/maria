package dm;
  parameter int DataCount = 32;
endpackage
module tb;
  import dm::*;
  logic [31:0] idx = 7;
  initial begin
    $display("T3 pkgcast=%0d", $clog2(dm::DataCount)'(idx));
    $display("T3 litcast=%0d", $clog2(32)'(idx));
    $finish;
  end
endmodule
