// TB 28_spi — SPI master cross-sim differential reference.
// Reset → start transfer (tx 0x5A, miso 0x33) → run enough cycles → sample.
`timescale 1ns/1ps
module tb_spi;
  logic clk = 0;
  logic rst_n = 0;
  logic start = 0;
  logic [7:0] tx_data = 0;
  logic [7:0] rx_data;
  logic sclk, mosi, miso = 0, cs_n, busy;

  spi_master #(.CLK_DIV(4)) dut (
    .clk(clk), .rst_n(rst_n), .start(start), .tx_data(tx_data),
    .rx_data(rx_data), .sclk(sclk), .mosi(mosi), .miso(miso),
    .cs_n(cs_n), .busy(busy)
  );

  always #5 clk = ~clk;

  initial begin
    $display("ASRT_START tb_spi");
    rst_n = 0;
    #20;
    rst_n = 1;
    #10;
    @(negedge clk);
    $display("ASRT_IDLE_CS=<%0d>", cs_n);

    // start transfer of 0x5A, miso idle 0x33
    tx_data = 8'h5A;
    miso = 8'h33;
    start = 1;
    @(posedge clk); @(negedge clk);
    start = 0;
    $display("ASRT_BUSY_START=<%0d>", busy);

    // 1 IDLE + 8*(4) bits + DONE ≈ run generous
    repeat (48) begin @(posedge clk); @(negedge clk); end
    $display("ASRT_DONE_BUSY=<%0d>", busy);
    $display("ASRT_RXDATA=<%02h>", rx_data);
    assert (busy === 0) else $error("ASRT_SPI_BUSY_BAD=<%0d>", busy);
    $display("ASRT_END tb_spi");
    $finish;
  end
endmodule