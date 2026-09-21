// tb_06a_generate — drive in bus, verifikasi inverter.
module tb_inverter_bus;
  logic [7:0] a;
  logic [7:0] b;

  inverter_bus #(.N(8)) dut (.a(a), .b(b));

  initial begin
    a = 8'hF0; #1;
    $display("ASRT_INV_F0=<%0d>", b);
    a = 8'h0F; #1;
    $display("ASRT_INV_0F=<%0d>", b);
    a = 8'h55; #1;
    $display("ASRT_INV_55=<%0d>", b);
    $finish;
  end
endmodule