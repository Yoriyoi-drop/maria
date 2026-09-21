// TB 11_simple_ram — memory read/write cross-sim differential reference.
`timescale 1ns/1ps
module tb_simple_ram;
  logic clk = 0;
  logic we = 0;
  logic [3:0] addr = 0;
  logic [7:0] din = 0;
  logic [7:0] dout;

  simple_ram #(.ADDR_W(4), .DATA_W(8)) dut (
    .clk(clk), .we(we), .addr(addr), .din(din), .dout(dout)
  );

  always #5 clk = ~clk;

  initial begin
    $display("ASRT_START tb_simple_ram");
    // write 0xA5 to addr 3
    addr = 4'h3; din = 8'hA5; we = 1;
    @(posedge clk); @(negedge clk);
    we = 0;
    // read addr 3 → 0xA5
    addr = 4'h3;
    @(posedge clk); @(negedge clk);
    assert (dout == 8'hA5) else $error("ASRT_RD3_BAD=<%02h>", dout);
    $display("ASRT_RD3=ok");

    // write 0x5A addr B, read back
    addr = 4'hB; din = 8'h5A; we = 1;
    @(posedge clk); @(negedge clk);
    we = 0;
    addr = 4'hB;
    @(posedge clk); @(negedge clk);
    assert (dout == 8'h5A) else $error("ASRT_RDB_BAD=<%02h>", dout);
    // address 3 still intact
    addr = 4'h3;
    @(posedge clk); @(negedge clk);
    assert (dout == 8'hA5) else $error("ASRT_RD3_KEEP_BAD=<%02h>", dout);
    $display("ASRT_KEEP=ok");

    $display("ASRT_END tb_simple_ram");
    $finish;
  end
endmodule