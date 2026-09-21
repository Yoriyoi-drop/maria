// TB 17_prio_gray — priority encoder + gray-to-binary cross-sim differential reference.
`timescale 1ns/1ps
module tb_prio_gray;
  logic [7:0] in;
  logic [2:0] out;
  logic valid;
  logic [3:0] gray;
  logic [3:0] bin;

  prio_enc u_prio (.in(in), .out(out), .valid(valid));
  gray_bin #(.W(4)) u_gray (.gray(gray), .bin(bin));

  task check_prio(input [7:0] e, input [2:0] exp, input exp_valid);
    in = e;
    #1;
    assert (out == exp) else $error("ASRT_PRIO_OUT_BAD=<%0b>, expect %0b", out, exp);
    assert (valid == exp_valid) else $error("ASRT_PRIO_VALID_BAD=<%0b>", valid);
  endtask

  initial begin
    $display("ASRT_START tb_prio_gray");
    check_prio(8'b1000_0000, 3'd7, 1'b1);
    check_prio(8'b0001_0000, 3'd4, 1'b1);
    check_prio(8'b0000_0011, 3'd1, 1'b1);
    check_prio(8'b0000_0000, 3'd0, 1'b0);
    $display("ASRT_PRIO=ok");

    // gray 0000 → bin 0000
    gray = 4'b0000; #1;
    assert (bin == 4'b0000) else $error("ASRT_GRAY0_BAD=<%0b>", bin);
    // gray 0011 (=2) → bin 0010
    gray = 4'b0011; #1;
    assert (bin == 4'b0010) else $error("ASRT_GRAY2_BAD=<%0b>", bin);
    // gray 1011 (=7) → bin 0101
    gray = 4'b1011; #1;
    assert (bin == 4'b0101) else $error("ASRT_GRAY7_BAD=<%0b>", bin);
    $display("ASRT_GRAY=ok");

    $display("ASRT_END tb_prio_gray");
    $finish;
  end
endmodule