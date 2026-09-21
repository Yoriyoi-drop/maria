// tb_03_mux_decoder — drive sel + d*, verifikasi mux4.
module tb_mux4;
  logic [1:0] sel;
  logic [3:0] d0, d1, d2, d3;
  logic [3:0] y;

  mux4 dut (.sel(sel), .d0(d0), .d1(d1), .d2(d2), .d3(d3), .y(y));

  initial begin
    d0 = 4'hA; d1 = 4'hB; d2 = 4'hC; d3 = 4'hD;
    sel = 2'd0; #1;
    $display("ASRT_Y0=<%0d>", y);
    sel = 2'd1; #1;
    $display("ASRT_Y1=<%0d>", y);
    sel = 2'd2; #1;
    $display("ASRT_Y2=<%0d>", y);
    sel = 2'd3; #1;
    $display("ASRT_Y3=<%0d>", y);
    $finish;
  end
endmodule