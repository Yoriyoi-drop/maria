// Q2 AUDIT — STRESS: hierarchical (HierRefIndex) runtime lookups per access.
//
// Candidate: engine `find_signal(name)` = linear scan atas SEMUA flattened
// signal (design.top.signals) PER evaluation of IrLValue::HierRefIndex /
// IrExpr::HierRef — tidak ada cache. Setiap akses `sif.arr[k]` membayar O(S).
//
// Workload: S ~ N*P signal (N leaf di generate, P regs/leaf), dan M ~ N akses
// hierarkis `sif.arr[k]` per cycle dari top-level initial. Per cycle biaya
// hier = O(M * S) = O(N * (N*P)) = O(N^2). Leaf sendiri melakukan eval O(N*P)
// per cycle (linear) — dipakai sebagai pekerjaan dasar yang realistis.
//
// Jalankan: maria benches_quad/q2_find_signal_stress.sv -T <Ts>
// Parameter: N leaf, M elemen interface array, P reg/leaf, T cycle.

interface bus_if #(
    parameter int K = 8,
    parameter int DN = 64
);
    logic [K-1:0] arr [0:DN-1];
    logic [K-1:0] probe;
endinterface

`ifndef NVAL
`define NVAL 100
`endif

// Leaf: P register → flat signal count naik P per leaf.
module leaf #(
    parameter int P = 16
)(
    input logic clk
);
    logic [7:0] r0;
    logic [7:0] r1;
    logic [7:0] r2;
    logic [7:0] r3;
    logic [7:0] r4;
    logic [7:0] r5;
    logic [7:0] r6;
    logic [7:0] r7;
    logic [7:0] r8;
    logic [7:0] r9;
    logic [7:0] ra;
    logic [7:0] rb;
    logic [7:0] rc;
    logic [7:0] rd;
    logic [7:0] re;
    logic [7:0] rf;

    always_ff @(posedge clk) begin
        r0 <= r0 + 8'd1;
        r1 <= r1 + 8'd2;
        r2 <= r2 + 8'd3;
        r3 <= r3 + 8'd4;
        r4 <= r4 + 8'd5;
        r5 <= r5 + 8'd6;
        r6 <= r6 + 8'd7;
        r7 <= r7 + 8'd8;
        r8 <= r8 + 8'd9;
        r9 <= r9 + 8'd10;
        ra <= ra + 8'd11;
        rb <= rb + 8'd12;
        rc <= rc + 8'd13;
        rd <= rd + 8'd14;
        re <= re + 8'd15;
        rf <= rf + 8'd16;
    end
endmodule

module top #(
    parameter int N = `NVAL,  // jumlah leaf (sumber sinyal S) & akses hier per cycle
    parameter int T = 10     // jumlah cycle
)();
    logic clk;
    logic [7:0] acc;

    bus_if #(.DN(N)) sif_early ();

    genvar g;
    generate
        for (g = 0; g < N; g++) begin : gen_leaf
            leaf #(16) u_leaf (.clk(clk));
        end
    endgenerate

    // Interface kedua DIDEKLARASIKAN SETELAH generate → flat-list position
    // di akhir vector → find_signal() membayar scan O(S) penuh per akses
    // (realistis: TB mengakses bus kedua yang dipakai scoreboard/checker).
    bus_if #(.DN(N)) sif_late ();

    initial begin
        clk = 0;
        forever #5 clk = ~clk;
    end

    initial begin
        for (int c = 0; c < T; c++) begin
            @(posedge clk);
            for (int k = 0; k < N; k++) begin
                sif_late.arr[k] = sif_late.arr[k] + 8'd1;   // 2x find_signal per akses (write + width), posisi akhir → O(S) per akses
            end
            acc <= acc + 8'd1;
        end
        $display("done T=%0d N=%0d arr31=%0d", T, N, sif_late.arr[31]);
        $finish;
    end
endmodule