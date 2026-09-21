// Seed 15: booth multiplier — always_comb + loop + shift
module booth_mult #(parameter W = 8) (
  input  logic [W-1:0] a, b,
  output logic [2*W-1:0] p
);
  integer i;
  logic [2*W:0] acc;

  always_comb begin
    acc = {{W+1{1'b0}}, b, 1'b0};
    for (i = 0; i < W; i++) begin
      case (acc[1:0])
        2'b01: acc = acc + ({ {W{a[W-1]}}, a} << 1);
        2'b10: acc = acc - ({ {W{a[W-1]}}, a} << 1);
        default: ;
      endcase
      acc = {acc[2*W], acc[2*W:1]};
    end
    p = acc[2*W:1];
  end
endmodule