// TB 14_regfile_struct — reg_file + packed struct cross-sim differential reference.
`timescale 1ns/1ps
module tb_regfile_struct;
  logic clk = 0;
  logic we = 0;
  logic [2:0] waddr = 0, raddr = 0;
  logic [31:0] wdata = 0;
  logic [31:0] rdata;

  packed_line_t c_in, c_out;
  logic c_we;

  reg_file #(.DEPTH(8), .WIDTH(32)) u_reg (
    .clk(clk), .we(we), .waddr(waddr), .raddr(raddr),
    .wdata(wdata), .rdata(rdata)
  );
  cache_line u_cache (.we(c_we), .din(c_in), .dout(c_out));

  always #5 clk = ~clk;

  initial begin
    $display("ASRT_START tb_regfile_struct");
    // write 0xDEADBEEF to reg 5, read back
    waddr = 3'd5; wdata = 32'hDEADBEEF; we = 1;
    @(posedge clk); @(negedge clk);
    we = 0;
    raddr = 3'd5;
    @(posedge clk); @(negedge clk);
    assert (rdata == 32'hDEADBEEF) else $error("ASRT_RF5_BAD=<%08h>", rdata);
    $display("ASRT_RF5=ok");

    // reg 0 untouched, reg 2 untouched (write other addr earlier)
    raddr = 3'd0;
    @(posedge clk); @(negedge clk);
    assert (rdata == 32'h00000000) else $error("ASRT_RF0_BAD=<%08h>", rdata);
    $display("ASRT_RF0=ok");

    // cache_line: din.data+1 when we
    c_we = 0;
    c_in = '{valid: 1'b1, tag: 8'hAB, data: 32'h00000005};
    #1;
    assert (c_out.data == 32'h00000005) else $error("ASRT_CACHE_NOWE_BAD=<%08h>", c_out.data);
    c_we = 1;
    #1;
    assert (c_out.data == 32'h00000006) else $error("ASRT_CACHE_WE_BAD=<%08h>", c_out.data);
    $display("ASRT_CACHE=ok");

    $display("ASRT_END tb_regfile_struct");
    $finish;
  end
endmodule