// tb_04_fifo — push 3, pop 2; verifikasi empty/full/dout.
module tb_fifo;
  logic clk = 0;
  logic rst_n = 0;
  logic push, pop;
  logic [7:0] din;
  logic [7:0] dout;
  logic full, empty;

  fifo #(.DEPTH(4), .WIDTH(8)) dut (
    .clk(clk), .rst_n(rst_n), .push(push), .pop(pop),
    .din(din), .dout(dout), .full(full), .empty(empty)
  );

  always #5 clk = ~clk;

  initial begin
    push = 0; pop = 0; din = 0;
    rst_n = 0;
    repeat (2) @(posedge clk);
    rst_n = 1;
    $display("ASRT_EMPTY0=<%0d>", empty);
    // push 3
    din = 8'h11; push = 1; @(posedge clk); push = 0;
    din = 8'h22; push = 1; @(posedge clk); push = 0;
    din = 8'h33; push = 1; @(posedge clk); push = 0;
    $display("ASRT_FULL0=<%0d>", full);
    $display("ASRT_DOUT0=<%0d>", dout);
    // pop 2 — baca SETELAH settle (#1) agar nilai comb (dout=mem[head])
    // sudah terpropagasi; isolasi bug engine vs race read-saya.
    pop = 1; @(posedge clk); pop = 0; #1;
    $display("ASRT_DOUT1=<%0d>", dout);
    pop = 1; @(posedge clk); pop = 0; #1;
    $display("ASRT_DOUT2=<%0d>", dout);
    $display("ASRT_EMPTY1=<%0d>", empty);
    $finish;
  end
endmodule