// rv32_soc.sv — RTL SoC untuk `mivon emu --rtl-cpu`: CPU (picorv32) +
// Direct RTL Devices (uart_console, timer_console) + address decoder MMIO —
// SEMUA di RTL.
//
// Bus:
//   - Alamat non-MMIO (termasuk fetch): diteruskan ke host (`mem_valid`/
//     `mem_ready`/`mem_rdata` port top) → dilayani host sebagai RAM.
//   - Store/load ke 0x1000_0000..0x1000_1fff: decode RTL → `uart_sel`/
//     `timer_sel` → device RTL di-latch, ack via OR internal (`cpu_mem_ready`).
//     Host TIDAK menjawab txn MMIO (serve() mengecek `mmio_sel`), hanya
//     membaca `uart_tx_done`/`uart_tx_byte` untuk console host.
//   - `mmio_sel`/`uart_tx_done`/`uart_tx_byte` adalah sinyal observasi host —
//     semua logika tetap di RTL.
//   - IRQ device (R4): UART `irq_tx` (level, bit 3) + timer `irq_timer`
//     (level, bit 4, device-initiated) + UART RX `irq_rx` (level, bit 5) →
//     `u_cpu.irq`; picorv32 mengakui via `eoi` (pulse di irq_state[1]) →
//     device. PROGADDR_IRQ = 0x80000100 (handler IRQ di RAM).
//   - UART RX (input host → device): host menulis `uart_rx_wr`+`uart_rx_din`
//     (port top) → UART RTL meng-latch (edge), CPU baca UART_BASE+8.
`timescale 1ns / 1ps

module rv32_soc (
    input  logic        clk,
    input  logic        resetn,
    // Host RAM port (CPU ↔ host; host melayani txn non-MMIO)
    output logic        mem_valid,
    output logic        mem_instr,
    output logic [31:0] mem_addr,
    output logic [31:0] mem_wdata,
    output logic [ 3:0] mem_wstrb,
    input  logic        mem_ready,
    input  logic [31:0] mem_rdata,
    // Status CPU
    output logic        trap,
    // Observasi host (Direct RTL Device / console)
    output logic        mmio_sel,
    output logic        uart_tx_done,
    output logic [ 7:0] uart_tx_byte,
    output logic [31:0] uart_tx_count,
    output logic        uart_irq_tx,
    output logic        uart_rx_pending,
    output logic [ 7:0] uart_rx_byte,
    output logic        uart_irq_rx,
    output logic        timer_irq,
    output logic [31:0] timer_count,
    // Input host → device (UART RX): strobe level + byte
    input  logic        uart_rx_wr,
    input  logic [ 7:0] uart_rx_din
);

    localparam [31:0] MMIO_BASE = 32'h1000_0000;
    localparam [31:0] MMIO_END  = 32'h1000_2000;
    localparam [31:0] UART_BASE = 32'h1000_0000;
    localparam [31:0] UART_END  = 32'h1000_1000;
    localparam [31:0] TIMER_BASE = 32'h1000_1000;

    // Decode MMIO: txn data (bukan fetch) ke rentang device.
    assign mmio_sel = mem_valid && !mem_instr
                    && mem_addr >= MMIO_BASE && mem_addr < MMIO_END;
    assign uart_sel  = mmio_sel && mem_addr >= UART_BASE && mem_addr < UART_END;
    assign timer_sel = mmio_sel && mem_addr >= TIMER_BASE;

    // CPU ack = ack host (RAM) ATAU ack MMIO (decoder RTL, combinational).
    wire cpu_mem_ready = mmio_sel || mem_ready;

    // CPU rdata = mux RTL: MMIO → register device, non-MMIO → rdata host
    // (RAM). Host TIDAK men-drive mem_rdata saat MMIO.
    // UART: +0 tx_byte, +4 tx_count, +8 rx_byte, +12 rx_pending (bit [3:2]).
    // TIMER_BASE+0 → count timer.
    wire [31:0] uart_rdata = (mem_addr[3:2] == 2'b00) ? {24'h0, uart_tx_byte}
                           : (mem_addr[3:2] == 2'b01) ? uart_tx_count
                           : (mem_addr[3:2] == 2'b10) ? {24'h0, uart_rx_byte}
                           :                            {31'h0, uart_rx_pending};
    wire [31:0] cpu_mem_rdata = timer_sel ? timer_count
                              : uart_sel  ? uart_rdata
                              :             mem_rdata;

    // Read strobe UART RX (baca UART_BASE+8 → read-clear pending).
    wire uart_rd_rx = uart_sel && (mem_wstrb == 4'b0000) && (mem_addr[3:2] == 2'b10);

    picorv32 #(
        .PROGADDR_RESET (32'h8000_0000),
        .PROGADDR_IRQ   (32'h8000_0100),
        .ENABLE_IRQ     (1),
        .ENABLE_MUL     (1),
        .ENABLE_DIV     (1),
        .REGS_INIT_ZERO (1)
    ) u_cpu (
        .clk         (clk),
        .resetn      (resetn),
        .trap        (trap),
        .mem_valid   (mem_valid),
        .mem_instr   (mem_instr),
        .mem_addr    (mem_addr),
        .mem_wdata   (mem_wdata),
        .mem_wstrb   (mem_wstrb),
        .mem_ready   (cpu_mem_ready),
        .mem_rdata   (cpu_mem_rdata),
        .mem_la_read (),
        .mem_la_write(),
        .mem_la_addr (),
        .mem_la_wdata(),
        .mem_la_wstrb(),
        .pcpi_valid  (),
        .pcpi_insn   (),
        .pcpi_rs1    (),
        .pcpi_rs2    (),
        .pcpi_wr     (1'b0),
        .pcpi_rd     (32'b0),
        .pcpi_wait   (1'b0),
        .pcpi_ready  (1'b0),
        // bit 5 = UART RX, bit 4 = timer, bit 3 = UART TX (konkat: kiri = MSB).
        .irq         ({26'b0, uart_irq_rx, timer_irq, uart_irq_tx, 3'b0}),
        .eoi         (uart_eoi),
        .trace_valid (),
        .trace_data  ()
    );

    wire [31:0] uart_eoi; // eoi CPU → ack IRQ device (bit 3 = UART, bit 4 = timer)

    // Anotasi Mivon (EMULATOR.md §10): region MMIO + IRQ didefinisikan DI
    // source → `mivon emu --dump-memory-map` menampilkannya tanpa --addr/.meu.
    (* mivon_region = "mmio", base = "0x10000000", size = "0x1000" *)
    (* mivon_irq = "3" *)
    uart_console u_uart (
        .clk       (clk),
        .resetn    (resetn),
        .wr        (uart_sel && |mem_wstrb),
        .din       (mem_wdata[7:0]),
        .rd        (uart_rd_rx),
        .rx_wr     (uart_rx_wr),
        .rx_din    (uart_rx_din),
        .tx_done   (uart_tx_done),
        .tx_byte   (uart_tx_byte),
        .tx_count  (uart_tx_count),
        .rx_byte   (uart_rx_byte),
        .rx_pending(uart_rx_pending),
        .eoi       (uart_eoi),
        .irq_tx    (uart_irq_tx),
        .irq_rx    (uart_irq_rx)
    );

    (* mivon_region = "mmio", base = "0x10001000", size = "0x1000" *)
    (* mivon_irq = "4" *)
    timer_console u_timer (
        .clk       (clk),
        .resetn    (resetn),
        .wr        (timer_sel && |mem_wstrb),
        .din       (mem_wdata),
        .eoi       (uart_eoi),
        .irq_timer (timer_irq),
        .count     (timer_count)
    );

endmodule
