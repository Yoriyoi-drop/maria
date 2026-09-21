// Seed 10: shift register + LFSR
module lfsr #(parameter W = 8) (
  input  logic clk, rst_n,
  input  logic en,
  output logic [W-1:0] data_out
);
  logic [W-1:0] lfsr_reg;

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) lfsr_reg <= '1;
    else if (en) begin
      lfsr_reg <= {lfsr_reg[W-2:0], ^lfsr_reg};
    end
  end

  assign data_out = lfsr_reg;
endmodule

module shifter #(parameter W = 16) (
  input  logic clk, rst_n,
  input  logic load,
  input  logic [W-1:0] din,
  output logic [W-1:0] dout
);
  logic [W-1:0] reg_q;

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) reg_q <= '0;
    else if (load) reg_q <= din;
    else reg_q <= {reg_q[W-2:0], 1'b0};
  end

  assign dout = reg_q;
endmodule

// Top: lfsr + shifter
module top10 #(parameter W = 8, parameter S = 16) (
  input  logic clk, rst_n, en, load,
  input  logic [S-1:0] din,
  output logic [W-1:0] l_out,
  output logic [S-1:0] s_out
);
  lfsr #(.W(W)) u_l (.clk(clk), .rst_n(rst_n), .en(en), .data_out(l_out));
  shifter #(.W(S)) u_s (.clk(clk), .rst_n(rst_n), .load(load), .din(din), .dout(s_out));
endmodule