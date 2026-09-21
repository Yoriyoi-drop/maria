// Seed 31: edge-detector + debounce (clock-domain)
module edge_detect (
  input  logic clk, rst_n,
  input  logic in,
  output logic rising, falling
);
  logic in_d1, in_d2;

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      in_d1 <= 1'b0;
      in_d2 <= 1'b0;
    end else begin
      in_d1 <= in;
      in_d2 <= in_d1;
    end
  end

  assign rising  = in_d1 & ~in_d2;
  assign falling = ~in_d1 & in_d2;
endmodule