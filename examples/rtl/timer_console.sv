// timer_console.sv — Direct RTL Device: countdown timer + IRQ device-initiated.
//
// Kontrak MMIO slave (di-decode oleh top SoC):
//   tulis 32-bit ke TIMER_BASE+0 (wstrb != 0) → `count` di-load ke nilai baru
//   (reload juga membersihkan IRQ), lalu `count` turun 1 per cycle.
//   baca TIMER_BASE+0 → nilai `count` saat ini (verifikasi host/CPU).
//   Saat `count` mencapai 0 (transisi 1→0) → `irq_timer` naik (LEVEL, bit 4)
//   tanpa perlu aksi CPU — interrupt murni dari device.
//   `irq_timer` turun saat CPU memberi ack `eoi[4]` (pulse end-of-interrupt
//   picorv32 di irq_state[1]) atau saat timer di-reload.
//
// Semua logika timer (counter, IRQ, ack) murni RTL — Rust tidak memodelkan
// timer, hanya mengamati sinyal output bila perlu.
`timescale 1ns / 1ps

module timer_console (
    input  logic        clk,
    input  logic        resetn,
    // MMIO write strobe (dari decoder SoC) — load nilai countdown baru.
    input  logic        wr,
    input  logic [31:0] din,
    // IRQ: ack end-of-interrupt dari CPU (bit 4 = timer).
    input  logic [31:0] eoi,
    output logic        irq_timer,
    // Status ke host/bus (baca): nilai countdown tersisa.
    output logic [31:0] count
);

    reg [31:0] count;
    reg        irq_timer;

    always @(posedge clk) begin
        if (!resetn) begin
            count     <= 32'h0;
            irq_timer <= 1'b0;
        end else begin
            if (eoi[4])
                irq_timer <= 1'b0;
            if (wr) begin
                count     <= din;
                irq_timer <= 1'b0; // reload = clear IRQ
            end else if (count != 32'h0) begin
                count <= count - 32'h1;
                if (count == 32'h1)
                    irq_timer <= 1'b1; // transisi 1→0 → IRQ level
            end
        end
    end

endmodule
