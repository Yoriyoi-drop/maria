// Seed 09: function + task + recursive function
module calc #(parameter W = 8) (
  input  logic [W-1:0] a, b,
  output logic [W-1:0] sum,
  output logic [W-1:0] prod,
  output logic [W-1:0] gcd_out
);

  function automatic [W-1:0] gcd(input logic [W-1:0] x, y);
    if (y == 0) gcd = x;
    else gcd = gcd(y, x % y);
  endfunction

  task automatic swap(input logic [W-1:0] x, y, output logic [W-1:0] xo, yo);
    xo = y;
    yo = x;
  endtask

  always_comb begin
    sum = a + b;
    prod = a * b;
    gcd_out = gcd(a, b);
  end
endmodule