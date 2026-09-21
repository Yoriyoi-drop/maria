// tb_02_adder4 — drive a,b,cin; verifikasi sum/cout.
module tb_adder4;
  logic [3:0] a, b;
  logic cin;
  logic [3:0] sum;
  logic cout;

  adder4 dut (.a(a), .b(b), .cin(cin), .sum(sum), .cout(cout));

  initial begin
    a = 4'b0011; b = 4'b0101; cin = 0;
    #1;
    $display("ASRT_SUM=<%0d>", sum);
    $display("ASRT_COUT=<%0d>", cout);
    a = 4'b1111; b = 4'b0001; cin = 0;
    #1;
    $display("ASRT_SUM2=<%0d>", sum);
    $display("ASRT_COUT2=<%0d>", cout);
    a = 4'b1010; b = 4'b0101; cin = 1;
    #1;
    $display("ASRT_SUM3=<%0d>", sum);
    $finish;
  end
endmodule