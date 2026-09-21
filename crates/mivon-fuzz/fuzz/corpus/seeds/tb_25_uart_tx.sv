// TB 25_uart_tx — UART TX FSM cross-sim differential reference.
// Pattern: reset → start one TX (data 0xA5) → run enough posedges → check
// tx_done + final tx_line. Deterministic across mivon/iverilog/verilator.
`timescale 1ns/1ps
module tb_uart_tx;
  logic clk = 0;
  logic rst_n = 0;
  logic tx_start = 0;
  logic [7:0] tx_data = 0;
  logic tx_line;
  logic tx_done;

  uart_tx #(.CLK_PER_BIT(4)) dut (
    .clk(clk), .rst_n(rst_n), .tx_start(tx_start), .tx_data(tx_data),
    .tx_line(tx_line), .tx_done(tx_done)
  );

  always #5 clk = ~clk;

  initial begin
    $display("ASRT_START tb_uart_tx");
    rst_n = 0;
    #20;
    rst_n = 1;
    #10;
    @(negedge clk);

    // start TX of 0xA5 = 1010_0101 (LSB-first: 1,0,1,0,0,1,0,1)
    tx_start = 1;
    @(posedge clk); @(negedge clk);
    tx_start = 0;

    // IDLE(1) + START(4) + 8*DATA(4) + STOP(4) clocks = run ~44 posedges.
    // tx_done pulih saat STOP completes → sample after generous bound.
    repeat (56) begin @(posedge clk); @(negedge clk); end
    $display("ASRT_DONE=<%0d>", tx_done);
    $display("ASRT_LINE0=<%0d>", tx_line);

    @(posedge clk); @(negedge clk);
    // back to IDLE → tx_line==1 (mark idle)
    $display("ASRT_LINE1=<%0d>", tx_line);
    assert (tx_done === 1) else $error("ASRT_TXDONE_BAD=<%0d>", tx_done);
    $display("ASRT_END tb_uart_tx");
    $finish;
  end
endmodule