// TB 08_interface — interface + modport cross-sim differential reference.
// Reference tools (iverilog/verilator) punya dukungan interface terbatas;
// jika compile gagal di reference → oracle skip (bukan bug).
`timescale 1ns/1ps
module tb_interface;
  bus_if #(.W(8)) b ();

  master u_master (b.master);
  slave  u_slave  (b.slave);

  initial begin
    b.clk = 0;
    #1;
    $display("ASRT_REQ=<%0d>", b.req);
    $display("ASRT_ACK=<%0d>", b.ack);
    $display("ASRT_ADDR=<%02h>", b.addr);
    $display("ASRT_DATA=<%02h>", b.data);
    assert (b.req == 1) else $error("ASRT_REQ_BAD=<%0d>", b.req);
    assert (b.ack == 1) else $error("ASRT_ACK_BAD=<%0d>", b.ack);
    assert (b.addr == 8'h10) else $error("ASRT_ADDR_BAD=<%02h>", b.addr);
    assert (b.data == 8'h11) else $error("ASRT_DATA_BAD=<%02h>", b.data);
    $display("ASRT_END tb_interface");
    $finish;
  end
endmodule