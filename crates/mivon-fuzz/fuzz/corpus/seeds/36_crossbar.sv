// Seed 36: crossbar 2x2 (mux+demux)
module crossbar #(parameter W = 8) (
  input  logic [W-1:0] in0, in1,
  input  logic sel,
  output logic [W-1:0] out0, out1
);
  always_comb begin
    if (sel) begin
      out0 = in1;
      out1 = in0;
    end else begin
      out0 = in0;
      out1 = in1;
    end
  end
endmodule

module demux1x4 #(parameter W = 8) (
  input  logic [W-1:0] in,
  input  logic [1:0] sel,
  output logic [W-1:0] y0, y1, y2, y3
);
  always_comb begin
    y0 = '0; y1 = '0; y2 = '0; y3 = '0;
    unique case (sel)
      2'd0: y0 = in;
      2'd1: y1 = in;
      2'd2: y2 = in;
      2'd3: y3 = in;
    endcase
  end
endmodule

// Top: crossbar + demux
module top_crossbar #(parameter W = 8) (
  input  logic [W-1:0] in0, in1,
  input  logic sel_cb,
  input  logic [1:0] sel_dm,
  output logic [W-1:0] cb0, cb1,
  output logic [W-1:0] y0, y1, y2, y3
);
  crossbar #(.W(W)) u_cb (.in0(in0), .in1(in1), .sel(sel_cb), .out0(cb0), .out1(cb1));
  demux1x4 #(.W(W)) u_dm (.in(in0), .sel(sel_dm), .y0(y0), .y1(y1), .y2(y2), .y3(y3));
endmodule