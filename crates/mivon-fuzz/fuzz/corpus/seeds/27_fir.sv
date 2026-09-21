// Seed 27: systolic FIR filter — generate + delayed pipeline
module fir_systolic #(
  parameter TAPS = 4,
  parameter W = 8
)(
  input  logic clk, rst_n,
  input  logic [W-1:0] x,
  output logic [2*W+3-1:0] y
);
  logic [W-1:0] taps [TAPS];
  logic [2*W+3-1:0] acc [TAPS];

  genvar i;
  generate
    for (i = 0; i < TAPS; i++) begin : gen_tap
      if (i == 0) begin : gen_first
        always_ff @(posedge clk or negedge rst_n) begin
          if (!rst_n) begin taps[i] <= '0; acc[i] <= '0; end
          else begin
            taps[i] <= x;
            acc[i] <= x * (i+1);
          end
        end
      end else begin : gen_rest
        always_ff @(posedge clk or negedge rst_n) begin
          if (!rst_n) begin taps[i] <= '0; acc[i] <= '0; end
          else begin
            taps[i] <= taps[i-1];
            acc[i] <= acc[i-1] + taps[i-1] * (i+1);
          end
        end
      end
    end
  endgenerate

  assign y = acc[TAPS-1];
endmodule