// Seed 24: generate if/else + localparam width expressions
module sync_fifo_if #(parameter DEPTH = 16) (
  input  logic clk, rst_n,
  input  logic push, pop,
  output logic full, empty
);
  logic [$clog2(DEPTH)-1:0] cnt;
  localparam logic [$clog2(DEPTH):0] DEPTH_V = DEPTH;

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) cnt <= '0;
    else begin
      unique case ({push && !full, pop && !empty})
        2'b10: cnt <= cnt + 1'b1;
        2'b01: cnt <= cnt - 1'b1;
        default: cnt <= cnt;
      endcase
    end
  end

  assign full  = (cnt == DEPTH_V[$clog2(DEPTH):0]);
  assign empty = (cnt == '0);

  generate
    if (DEPTH > 4) begin : gen_deep
      localparam int LOG_DEPTH = $clog2(DEPTH);
    end else begin : gen_shallow
      localparam int LOG_DEPTH = 1;
    end
  endgenerate
endmodule