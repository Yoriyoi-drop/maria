// tb_29_and_tree — drive in[7:0], verifikasi AND-tree reduce.
module tb_and_tree;
  logic [7:0] in;
  logic out;

  and_tree #(.N(8)) dut (.in(in), .out(out));

  initial begin
    in = 8'hFF; #1;
    $display("ASRT_ALL1=<%0d>", out);
    in = 8'hFE; #1;
    $display("ASRT_ALL0=<%0d>", out);
    in = 8'h80; #1;
    $display("ASRT_ONE=<%0d>", out);
    in = 8'h01; #1;
    $display("ASRT_ONE0=<%0d>", out);
    $finish;
  end
endmodule