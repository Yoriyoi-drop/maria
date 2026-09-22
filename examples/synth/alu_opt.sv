// Contoh demo pass optimizer SIR (SYNTHESIS.md §6) — `mivon synth --dump-sir-opt`.
//
// Ekspresi sengaja ditulis "boros" agar pass optimizer terlihat:
//   y = (a & 0) | (b & FF) | ~~a        → y = a | b          (identity + double-inv)
//   z = (a + 0) + (b * 4)               → z = a + (b << 2)   (a+0, strength-reduction)
module alu_opt (
    input  logic [7:0] a, b,
    output logic [7:0] y,
    output logic [7:0] z
);
    assign y = (a & 8'h00) | (b & 8'hFF) | ~~a;
    assign z = (a + 8'h00) + (b * 8'h04);
    
endmodule

