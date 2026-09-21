// TB 26_bit_ops — streaming / part-select / shift reference.
// NOTE: iverilog tidak didukung streaming `{<<{a}}` (compile-gagal), jadi seed
// ini adalah mivon-only differential via internal jalur (O5). Marker tetap
// dibroadcast agar konsisten; oracle icarus akan skip seed ini (ref N/A).
`timescale 1ns/1ps
module tb_bit_ops;
  logic clk = 0;
  logic [15:0] a = 16'b1100_1010_0011_0101;
  logic [15:0] rev, swapped, shr, shl;
  logic [3:0] nibble;

  bit_ops #(.W(16)) dut (.a(a), .rev(rev), .swapped(swapped),
                         .nibble(nibble), .shr(shr), .shl(shl));

  always #5 clk = ~clk;

  initial begin
    $display("ASRT_START tb_bit_ops");
    repeat (3) @(posedge clk);
    @(negedge clk);
    $display("ASRT_REV=<%04h>", rev);
    $display("ASRT_SWAPPED=<%04h>", swapped);
    $display("ASRT_NIBBLE=<%h>", nibble);
    $display("ASRT_SHR=<%h>", shr);
    $display("ASRT_SHL=<%h>", shl);
    $display("ASRT_END tb_bit_ops");
    $finish;
  end
endmodule