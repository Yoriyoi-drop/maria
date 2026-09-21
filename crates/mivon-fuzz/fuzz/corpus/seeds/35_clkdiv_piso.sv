// Seed 35: parallel-in/serial-out + clock divider
module clock_div #(parameter DIV = 4) (
  input  logic clk, rst_n,
  output logic out
);
  logic [$clog2(DIV)-1:0] cnt;

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      cnt <= '0;
      out <= 1'b0;
    end else begin
      if (cnt == DIV-1) begin
        cnt <= '0;
        out <= ~out;
      end else cnt <= cnt + 1;
    end
  end
endmodule

module piso #(parameter W = 8) (
  input  logic clk, rst_n,
  input  logic load,
  input  logic [W-1:0] din,
  output logic out
);
  logic [W-1:0] sh;
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      sh <= '0;
      out <= 1'b0;
    end else if (load) begin
      sh <= din;
    end else begin
      out <= sh[W-1];
      sh <= {sh[W-2:0], 1'b0};
    end
  end
endmodule

// Top: clock divider + PISO
module top_clkdiv #(parameter DIV = 4, parameter W = 8) (
  input  logic clk, rst_n,
  input  logic load,
  input  logic [W-1:0] din,
  output logic div_out,
  output logic s_out
);
  clock_div #(.DIV(DIV)) u_d (.clk(clk), .rst_n(rst_n), .out(div_out));
  piso #(.W(W)) u_p (.clk(clk), .rst_n(rst_n), .load(load), .din(din), .out(s_out));
endmodule