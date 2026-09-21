// TB 20_arith_matrix — signed/unsigned arith + matrix cross-sim differential reference.
`timescale 1ns/1ps
module tb_arith_matrix;
  logic [7:0] a, b;
  logic sign_sel;
  logic [15:0] mul_out;
  logic [7:0] add_out;
  logic overflow;

  logic [7:0] m[3][3];
  logic [7:0] v[3];
  logic [7:0] r[3];

  arith #(.W(8)) u_arith (.a(a), .b(b), .sign_sel(sign_sel),
                          .mul_out(mul_out), .add_out(add_out), .overflow(overflow));
  matrix3x3 u_mat (.m(m), .v(v), .r(r));

  integer i, j;

  initial begin
    $display("ASRT_START tb_arith_matrix");
    // unsigned: 3*4=12, 3+4=7, overflow 0
    sign_sel = 0; a = 8'd3; b = 8'd4; #1;
    assert (mul_out == 16'd12) else $error("ASRT_UMUL_BAD=<%0d>", mul_out);
    assert (add_out == 8'd7) else $error("ASRT_UADD_BAD=<%0d>", add_out);
    assert (overflow == 0) else $error("ASRT_UOVF_BAD=<%0d>", overflow);

    // unsigned overflow: 200+200=144 wrap, overflow=1
    sign_sel = 0; a = 8'd200; b = 8'd200; #1;
    assert (add_out == 8'd144) else $error("ASRT_UOVFADD_BAD=<%0d>", add_out);
    assert (overflow == 1) else $error("ASRT_UOVF_BAD2=<%0d>", overflow);
    $display("ASRT_UNSIGNED=ok");

    // signed: -5 * 6 = -30
    sign_sel = 1; a = 8'hFB; b = 8'd6; #1;  // -5 * 6
    assert ($signed(mul_out) == -30) else $error("ASRT_SMUL_BAD=<%0d>", $signed(mul_out));
    assert ($signed(add_out) == 1) else $error("ASRT_SADD_BAD=<%0d>", $signed(add_out));
    $display("ASRT_SIGNED=ok");

    // matrix: m = [[1,2,3],[4,5,6],[7,8,9]], v = [1,1,1] → r = [6,15,24]
    for (i = 0; i < 3; i++) begin
      for (j = 0; j < 3; j++) begin
        m[i][j] = 8'(1 + i*3 + j);
      end
      v[i] = 8'd1;
    end
    #1;
    assert (r[0] == 8'd6) else $error("ASRT_MAT0_BAD=<%0d>", r[0]);
    assert (r[1] == 8'd15) else $error("ASRT_MAT1_BAD=<%0d>", r[1]);
    assert (r[2] == 8'd24) else $error("ASRT_MAT2_BAD=<%0d>", r[2]);
    $display("ASRT_MATRIX=ok");

    $display("ASRT_END tb_arith_matrix");
    $finish;
  end
endmodule