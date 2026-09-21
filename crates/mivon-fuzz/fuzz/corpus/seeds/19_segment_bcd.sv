// Seed 19: 7-segment decoder + counter combo
module segment_dec (
  input  logic [3:0] bcd,
  output logic [6:0] seg
);
  always_comb begin
    unique case (bcd)
      4'h0: seg = 7'b1000000;
      4'h1: seg = 7'b1111001;
      4'h2: seg = 7'b0100100;
      4'h3: seg = 7'b0110000;
      4'h4: seg = 7'b0011001;
      4'h5: seg = 7'b0010010;
      4'h6: seg = 7'b0000010;
      4'h7: seg = 7'b1111000;
      4'h8: seg = 7'b0000000;
      4'h9: seg = 7'b0010000;
      default: seg = 7'b1111111;
    endcase
  end
endmodule

module bcd_counter (
  input  logic clk, rst_n,
  output logic [3:0] bcd,
  output logic carry
);
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) bcd <= 4'h0;
    else if (bcd == 4'h9) bcd <= 4'h0;
    else bcd <= bcd + 4'h1;
  end
  assign carry = (bcd == 4'h9);
endmodule

// Top: segment_dec + bcd_counter
module top19 (
  input  logic clk, rst_n,
  input  logic [3:0] bcd,
  output logic [6:0] seg,
  output logic [3:0] cnt,
  output logic carry
);
  segment_dec u_d (.bcd(bcd), .seg(seg));
  bcd_counter u_c (.clk(clk), .rst_n(rst_n), .bcd(cnt), .carry(carry));
endmodule