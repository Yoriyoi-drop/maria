// uart_console.sv — Direct RTL Device: UART console TX+RX + status + IRQ.
//
// Kontrak MMIO slave (di-decode oleh top SoC, UART_BASE = 0x10000000):
//   tulis 1 byte ke UART_BASE+0 (wstrb != 0) → byte di-latch ke `tx_byte`,
//   `tx_done` pulse 1 cycle, `tx_count` naik. Host (mivon emu) membaca
//   `tx_byte`/`tx_done` dan meneruskan byte ke terminal host.
//   baca UART_BASE+0 → byte TX terakhir; UART_BASE+4 → `tx_count`; UART_BASE+8
//   → byte RX terakhir (baca juga menghapus `rx_pending`); UART_BASE+12 →
//   status `rx_pending`.
//
// RX (input host → device, R4 bidirectional): host menaikkan `rx_wr` (level,
// dengan `rx_din` = byte). Rising-edge `rx_wr` meng-latch byte ke `rx_byte`,
// men-set `rx_pending`, menaikkan `irq_rx` (level, bit 5). `rx_wr` boleh
// tetap tinggi (edge-detect mencegah re-latch). `irq_rx` turun saat CPU
// membaea UART_BASE+8 (read-clear) atau ack `eoi[5]`.
//
// IRQ TX: tiap byte TX menaikkan `irq_tx` (level, bit 3) setelah `IRQ_DELAY`
// (=16) cycle dari `tx_done` — tunda memberi CPU waktu mencapai `waitirq`
// sebelum IRQ tiba (deterministik). `irq_tx` turun saat ack `eoi[3]`.
//
// Semua logika UART (latch, counter, handshake, IRQ) murni RTL — Rust hanya
// membaca sinyal output untuk console / menulis `rx_wr`+`rx_din` untuk input,
// tidak memodelkan UART.
`timescale 1ns / 1ps

module uart_console (
    input  logic        clk,
    input  logic        resetn,
    // MMIO write strobe TX (dari decoder SoC)
    input  logic        wr,
    input  logic [ 7:0] din,
    // MMIO read strobe RX (baca UART_BASE+8 → rx_byte, read-clear pending)
    input  logic        rd,
    // Input host → device (RX): strobe level + byte
    input  logic        rx_wr,
    input  logic [ 7:0] rx_din,
    // Status ke host (console capture)
    output logic        tx_done,
    output logic [ 7:0] tx_byte,
    // Status register (baca dari bus)
    output logic [31:0] tx_count,
    output logic [ 7:0] rx_byte,
    output logic        rx_pending,
    // IRQ: ack end-of-interrupt dari CPU (bit 3 = TX, bit 5 = RX)
    input  logic [31:0] eoi,
    output logic        irq_tx,
    output logic        irq_rx
);

    reg [ 7:0] tx_byte;
    reg        tx_done;
    reg [31:0] tx_count;

    reg [ 7:0] rx_byte;
    reg        rx_pending;
    reg        rx_wr_d;
    reg        irq_rx;

    // IRQ TX: shift-register penunda `tx_done` → `irq_tx` (level), clear oleh
    // `eoi[3]`. IRQ_DELAY harus lebih besar dari cycle CPU dari store 'A'
    // sampai `waitirq` ter-decode (±10) — 16 aman.
    localparam integer IRQ_DELAY = 16;
    reg [IRQ_DELAY-1:0] irq_delay_shift;
    reg                irq_tx;

    always @(posedge clk) begin
        if (!resetn) begin
            tx_done  <= 1'b0;
            tx_byte  <= 8'h00;
            tx_count <= 32'h0;
            rx_byte      <= 8'h00;
            rx_pending   <= 1'b0;
            rx_wr_d      <= 1'b0;
            irq_rx       <= 1'b0;
            irq_delay_shift <= {IRQ_DELAY{1'b0}};
            irq_tx   <= 1'b0;
        end else begin
            // ── TX ──
            tx_done <= 1'b0; // pulse 1 cycle
            if (wr) begin
                tx_byte  <= din;
                tx_done  <= 1'b1;
                tx_count <= tx_count + 32'h1;
            end
            irq_delay_shift <= {irq_delay_shift[IRQ_DELAY-2:0], tx_done};
            if (eoi[3])
                irq_tx <= 1'b0;
            else if (irq_delay_shift[IRQ_DELAY-1])
                irq_tx <= 1'b1;

            // ── RX (rising-edge rx_wr → latch; read-clear) ──
            rx_wr_d <= rx_wr;
            if (rx_wr && !rx_wr_d) begin
                rx_byte    <= rx_din;
                rx_pending <= 1'b1;
                irq_rx     <= 1'b1;
            end
            if (rd) begin
                rx_pending <= 1'b0;
                irq_rx     <= 1'b0;
            end
            if (eoi[5])
                irq_rx <= 1'b0;
        end
    end

endmodule
