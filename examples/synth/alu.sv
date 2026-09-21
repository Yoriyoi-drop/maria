// Contoh RTL untuk technology mapping (SYNTHESIS.md §12 — phase 4).
//
// ALU 8-bit, op 2-bit. Ekspektasi mapping (generic LUT6/CARRY4):
//   - y[i] = mux(op, a&b, a|b, a+b, a^b) — cone per bit: 4 leaf
//     (a[i], b[i], op[0], op[1]) ≤ K=6 → SATU LUT6 per bit → 8 LUT.
//   - a+b diimplementasi via carry chain CARRY4 (8-bit → ceil(8/4)=2).
//   - Total: LUT=8, CARRY4=2, FF=0.
module alu (
    input  logic [7:0] a, b,
    input  logic [1:0] op,
    output logic [7:0] y
);
    always_comb begin
        case (op)
            2'd0: y = a & b;
            2'd1: y = a | b;
            2'd2: y = a + b;
            default: y = a ^ b;
        endcase
    end
endmodule
