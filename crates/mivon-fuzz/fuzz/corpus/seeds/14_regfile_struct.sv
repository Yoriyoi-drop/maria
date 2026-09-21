// Seed 14: parameterized array + packed struct
module reg_file #(
  parameter DEPTH = 8,
  parameter WIDTH = 32
)(
  input  logic clk,
  input  logic we,
  input  logic [$clog2(DEPTH)-1:0] waddr, raddr,
  input  logic [WIDTH-1:0] wdata,
  output logic [WIDTH-1:0] rdata
);
  logic [WIDTH-1:0] regs [DEPTH];

  always_ff @(posedge clk) begin
    if (we) regs[waddr] <= wdata;
  end

  assign rdata = regs[raddr];
endmodule

typedef struct packed {
  logic valid;
  logic [7:0] tag;
  logic [31:0] data;
} packed_line_t;

module cache_line (
  input  logic we,
  input  packed_line_t din,
  output packed_line_t dout
);
  always_comb begin
    dout = din;
    if (we) dout.data = din.data + 32'h1;
  end
endmodule

// Top: reg_file + cache_line
module top14 #(parameter DEPTH = 8, parameter WIDTH = 32) (
  input  logic clk, we_rf, we_cl,
  input  logic [$clog2(DEPTH)-1:0] waddr, raddr,
  input  logic [WIDTH-1:0] wdata,
  output logic [WIDTH-1:0] rdata,
  input  packed_line_t din_cl,
  output packed_line_t dout_cl
);
  reg_file #(.DEPTH(DEPTH), .WIDTH(WIDTH)) u_rf (
    .clk(clk), .we(we_rf), .waddr(waddr), .raddr(raddr), .wdata(wdata), .rdata(rdata)
  );
  cache_line u_cl (.we(we_cl), .din(din_cl), .dout(dout_cl));
endmodule