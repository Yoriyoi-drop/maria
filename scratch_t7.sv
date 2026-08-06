module tb;
  logic [31:0] idx = 7;
  initial begin
    $display("T7a plain=%0d", 7);
    $display("T7b sig=%0d", idx);
    $display("T7c hex=%h", 8'(7));
    $display("T7d castbin=%b", 8'(7));
    $finish;
  end
endmodule
