// TB 24_gen_if — generate if/else sync FIFO cross-sim differential reference.
`timescale 1ns/1ps
module tb_sync_fifo_if;
  logic clk = 0;
  logic rst_n = 0;
  logic push = 0, pop = 0;
  logic full, empty;

  sync_fifo_if #(.DEPTH(16)) dut (
    .clk(clk), .rst_n(rst_n), .push(push), .pop(pop), .full(full), .empty(empty)
  );

  always #5 clk = ~clk;

  initial begin
    $display("ASRT_START tb_sync_fifo_if");
    rst_n = 0;
    #20;
    rst_n = 1;
    #10;
    @(negedge clk);
    assert (empty === 1) else $error("ASRT_EMPTY_INIT_BAD=<%0d>", empty);
    assert (full === 0) else $error("ASRT_FULL_INIT_BAD=<%0d>", full);

    // push 5 → cnt=5, empty=0
    push = 1;
    repeat (5) begin @(posedge clk); @(negedge clk); end
    push = 0;
    assert (empty === 0) else $error("ASRT_NOT_EMPTY_BAD=<%0d>", empty);
    $display("ASRT_PUSH5=ok");

    // both push & pop → cnt unchanged; full/empty unchanged
    push = 1; pop = 1;
    @(posedge clk); @(negedge clk);
    push = 0; pop = 0;
    assert (full === 0 && empty === 0) else $error("ASRT_BOTH_BAD full=%0d empty=%0d", full, empty);
    $display("ASRT_BOTH=ok");

    // pop 5 → empty=1
    pop = 1;
    repeat (5) begin @(posedge clk); @(negedge clk); end
    pop = 0;
    assert (empty === 1) else $error("ASRT_EMPTY_END_BAD=<%0d>", empty);
    $display("ASRT_EMPTY_END=ok");

    $display("ASRT_END tb_sync_fifo_if");
    $finish;
  end
endmodule