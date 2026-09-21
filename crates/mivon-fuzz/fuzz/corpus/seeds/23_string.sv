// Seed 23: string ops + $sformatf + $display formatting
module str_demo (
  input logic clk,
  output logic [31:0] hash
);
  string name = "mivon";
  string greeting;
  integer i;
  logic [31:0] h = 0;

  always_ff @(posedge clk) begin
    greeting = $sformatf("hello %s", name);
    h = 0;
    for (i = 0; i < greeting.len(); i++) begin
      h = h * 31 + greeting[i];
    end
    hash <= h;
  end
endmodule