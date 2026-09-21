// TB 18_clock_gen — self-driving clock gen cross-sim differential reference.
// DUT (clock_gen) self-drives clk + count and self-$finish setelah count==16.
// TB hanya sampel count sebelum DUT finish (~t=165) dan mem-broadcast marker.
`timescale 1ns/1ps
module tb_clock_gen;
  logic clk;
  logic [3:0] count;

  clock_gen dut (.clk(clk), .count(count));

  initial begin
    $display("ASRT_START tb_clock_gen");
    // clk toggles setiap 5ns → posedge di 5,15,25,35,... → count++ per posedge.
    #40;                       // ≈ 4 posedges → count==4
    $display("ASRT_COUNT40=<%0d>", count);
    #60;                       // ≈ t=100 → ~10 posedges
    $display("ASRT_COUNT100=<%0d>", count);
    // DUT $finish berhenti simulation saat count==16 (~t=165); tb pasrah.
  end
endmodule