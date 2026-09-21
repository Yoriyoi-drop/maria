// Seed 22: class-based verification pattern (self-contained)
class packet;
  rand bit [7:0] addr;
  rand bit [7:0] data;
  constraint c_addr { addr != 8'h00; }
  constraint c_data { data inside {[1:100]}; }
endclass

module class_top (
  input logic clk,
  output logic [7:0] out
);
  packet p;
  integer seed = 42;

  initial begin
    p = new();
    assert (p.randomize() with { addr == 8'hAA; });
    out = p.addr ^ p.data;
    $finish;
  end
endmodule