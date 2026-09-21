// Seed 32: ring counter / Johnson counter
module johnson #(parameter N = 4) (
  input  logic clk, rst_n,
  output logic [N-1:0] out
);
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) out <= '0;
    else out <= {~out[0], out[N-1:1]};
  end
endmodule

module ring #(parameter N = 4) (
  input  logic clk, rst_n,
  output logic [N-1:0] out
);
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) out <= {{N-1{1'b0}}, 1'b1};
    else out <= {out[N-2:0], out[N-1]};
  end
endmodule

// Top: instansiasi kedua (target sim deterministik)
module top_jr #(parameter N = 4) (
  input  logic clk, rst_n,
  output logic [N-1:0] j_out,
  output logic [N-1:0] r_out
);
  johnson #(.N(N)) u_j (.clk(clk), .rst_n(rst_n), .out(j_out));
  ring    #(.N(N)) u_r (.clk(clk), .rst_n(rst_n), .out(r_out));
endmodule