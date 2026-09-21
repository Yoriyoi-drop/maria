// Seed 17: priority encoder + gray code converter
module prio_enc (
  input  logic [7:0] in,
  output logic [2:0] out,
  output logic valid
);
  always_comb begin
    valid = |in;
    casez (in)
      8'b1???????: out = 3'd7;
      8'b01??????: out = 3'd6;
      8'b001?????: out = 3'd5;
      8'b0001????: out = 3'd4;
      8'b00001???: out = 3'd3;
      8'b000001??: out = 3'd2;
      8'b0000001?: out = 3'd1;
      8'b00000001: out = 3'd0;
      default: out = 3'd0;
    endcase
  end
endmodule

module gray_bin #(parameter W = 4) (
  input  logic [W-1:0] gray,
  output logic [W-1:0] bin
);
  integer i;
  always_comb begin
    bin[W-1] = gray[W-1];
    for (i = W-2; i >= 0; i--) begin
      bin[i] = bin[i+1] ^ gray[i];
    end
  end
endmodule

// Top: prio_enc + gray_bin
module top17 #(parameter W = 4) (
  input  logic [7:0] pe_in,
  output logic [2:0] pe_out,
  output logic pe_valid,
  input  logic [W-1:0] gray,
  output logic [W-1:0] bin
);
  prio_enc u_p (.in(pe_in), .out(pe_out), .valid(pe_valid));
  gray_bin #(.W(W)) u_g (.gray(gray), .bin(bin));
endmodule