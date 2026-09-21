// TB 30_axi_lite — AXI-lite handshake cross-sim differential reference.
// Reset → AWVALID addr handshake → WVALID write data → check bvalid + mem.
`timescale 1ns/1ps
module tb_axi_lite;
  logic clk = 0;
  logic rst_n = 0;
  logic awvalid = 0, wvalid = 0, bready = 0;
  logic [31:0] awaddr = 0, wdata = 0;
  logic awready, wready, bvalid;

  axi_lite_slave #(.W(32)) dut (
    .clk(clk), .rst_n(rst_n), .awvalid(awvalid), .awready(awready),
    .awaddr(awaddr), .wvalid(wvalid), .wready(wready),
    .wdata(wdata), .bvalid(bvalid), .bready(bready)
  );

  always #5 clk = ~clk;

  initial begin
    $display("ASRT_START tb_axi_lite");
    rst_n = 0;
    #20;
    rst_n = 1;
    #10;
    @(negedge clk);

    // AW handshake
    awaddr = 32'h0000_00AB;
    awvalid = 1; @(posedge clk); @(negedge clk); awvalid = 0;
    @(posedge clk); @(negedge clk);
    $display("ASRT_AWREADY0=<%0d>", awready);
    assert (awready === 0) else $error("ASRT_AWREADY_BAD=<%0d>", awready);

    // W write
    wdata = 32'hC0DE_BEEF;
    wvalid = 1; @(posedge clk); @(negedge clk); wvalid = 0;
    @(posedge clk); @(negedge clk);
    $display("ASRT_BVALID=<%0d>", bvalid);
    assert (bvalid === 1) else $error("ASRT_BVALID_BAD=<%0d>", bvalid);

    // bready acks
    bready = 1; @(posedge clk); @(negedge clk); bready = 0;
    @(posedge clk); @(negedge clk);
    $display("ASRT_BVALID_CLR=<%0d>", bvalid);
    assert (bvalid === 0) else $error("ASRT_BVALID_RESET_BAD=<%0d>", bvalid);

    $display("ASRT_END tb_axi_lite");
    $finish;
  end
endmodule