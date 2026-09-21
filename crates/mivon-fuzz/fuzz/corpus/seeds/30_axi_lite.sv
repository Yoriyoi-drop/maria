// Seed 30: AXI-lite style handshake — valid/ready with counter
module axi_lite_slave #(parameter W = 32) (
  input  logic clk, rst_n,
  input  logic awvalid,
  output logic awready,
  input  logic [W-1:0] awaddr,
  input  logic wvalid,
  output logic wready,
  input  logic [W-1:0] wdata,
  output logic bvalid,
  input  logic bready
);
  logic [W-1:0] mem [256];
  logic [W-1:0] waddr_q;
  logic write_pending;

  assign awready = !write_pending;
  assign wready  = !write_pending;

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      write_pending <= 1'b0;
      bvalid <= 1'b0;
    end else begin
      if (awvalid && !write_pending) begin
        waddr_q <= awaddr;
        write_pending <= 1'b1;
      end
      if (wvalid && write_pending) begin
        mem[waddr_q[7:0]] <= wdata;
        write_pending <= 1'b0;
        bvalid <= 1'b1;
      end
      if (bvalid && bready) bvalid <= 1'b0;
    end
  end
endmodule