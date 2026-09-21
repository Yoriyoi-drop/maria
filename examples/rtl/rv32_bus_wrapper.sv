// rv32_bus_wrapper.sv — wrapper RTL CPU untuk `mivon emu --rtl-cpu`.
//
// Menginstansiasi picorv32 (RV32IM) dengan parameter override via `#(...)`
// (reset vector 0x8000_0000, M-extension aktif, register file di-init 0).
// Antarmuka bus sederhana (picorv32-style):
//   mem_valid / mem_instr / mem_addr / mem_wdata / mem_wstrb  (output, CPU → host)
//   mem_ready / mem_rdata                                     (input,  host → CPU)
//   trap (output) — tinggi saat ebreak/ecall/instruksi ilegal.
//
// Semua device/peripheral lain (UART, timer, ...) juga wajib dari RTL
// (Direct RTL Device) — lihat EMULATOR.md §10.
`timescale 1ns / 1ps

module rv32_bus_wrapper (
    input  logic        clk,
    input  logic        resetn,
    // Memory port (host = mivon emu)
    output logic        mem_valid,
    output logic        mem_instr,
    output logic [31:0] mem_addr,
    output logic [31:0] mem_wdata,
    output logic [ 3:0] mem_wstrb,
    input  logic        mem_ready,
    input  logic [31:0] mem_rdata,
    // Status
    output logic        trap
);

    picorv32 #(
        .PROGADDR_RESET (32'h8000_0000),
        .PROGADDR_IRQ   (32'h8000_0000),
        .ENABLE_MUL     (1),
        .ENABLE_DIV     (1),
        .REGS_INIT_ZERO (1)
    ) u_cpu (
        .clk        (clk),
        .resetn     (resetn),
        .trap       (trap),
        .mem_valid  (mem_valid),
        .mem_instr  (mem_instr),
        .mem_addr   (mem_addr),
        .mem_wdata  (mem_wdata),
        .mem_wstrb  (mem_wstrb),
        .mem_ready  (mem_ready),
        .mem_rdata  (mem_rdata),
        .mem_la_read (),
        .mem_la_write(),
        .mem_la_addr (),
        .mem_la_wdata(),
        .mem_la_wstrb(),
        .pcpi_valid (),
        .pcpi_insn  (),
        .pcpi_rs1   (),
        .pcpi_rs2   (),
        .pcpi_wr    (1'b0),
        .pcpi_rd    (32'b0),
        .pcpi_wait  (1'b0),
        .pcpi_ready (1'b0),
        .irq        (32'b0),
        .eoi        (),
        .trace_valid(),
        .trace_data ()
    );

endmodule
