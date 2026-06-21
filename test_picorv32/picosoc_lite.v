module picosoc_lite (
    input clk,
    input resetn,

    output ser_tx,
    input  ser_rx
);
    parameter MEM_WORDS = 256;
    parameter [31:0] STACKADDR = (4*MEM_WORDS);
    parameter [31:0] PROGADDR_RESET = 32'h 0000_0000;

    wire        mem_valid;
    wire        mem_instr;
    reg         mem_ready;
    wire [31:0] mem_addr;
    wire [31:0] mem_wdata;
    wire [3:0]  mem_wstrb;
    reg  [31:0] mem_rdata;

    reg [31:0] ram [0:MEM_WORDS-1];

    always @(posedge clk) begin
        mem_ready <= 0;
        if (mem_valid && !mem_ready) begin
            mem_ready <= 1;
            if (mem_wstrb) begin
                if (mem_wstrb[0]) ram[mem_addr[31:2]][7:0] <= mem_wdata[7:0];
                if (mem_wstrb[1]) ram[mem_addr[31:2]][15:8] <= mem_wdata[15:8];
                if (mem_wstrb[2]) ram[mem_addr[31:2]][23:16] <= mem_wdata[23:16];
                if (mem_wstrb[3]) ram[mem_addr[31:2]][31:24] <= mem_wdata[31:24];
            end else begin
                mem_rdata <= ram[mem_addr[31:2]];
            end
        end
    end

    wire        uart_div_sel = mem_valid && (mem_addr == 32'h 0200_0004);
    wire        uart_dat_sel = mem_valid && (mem_addr == 32'h 0200_0008);

    wire [31:0] uart_div_do;
    wire [31:0] uart_dat_do;
    wire        uart_dat_wait;

    wire [3:0]  uart_div_we;
    assign uart_div_we = uart_div_sel ? mem_wstrb : 4'b0;

    wire        uart_dat_we;
    assign uart_dat_we = uart_dat_sel ? mem_wstrb[0] : 1'b0;

    wire        uart_dat_re;
    assign uart_dat_re = uart_dat_sel && !mem_wstrb;

    assign mem_ready = uart_div_sel;
    assign mem_rdata = uart_div_sel ? uart_div_do : uart_dat_sel ? uart_dat_do : mem_rdata;

    simpleuart uart (
        .clk         (clk),
        .resetn      (resetn),
        .ser_tx      (ser_tx),
        .ser_rx      (ser_rx),
        .reg_div_we  (uart_div_we),
        .reg_div_di  (mem_wdata),
        .reg_div_do  (uart_div_do),
        .reg_dat_we  (uart_dat_we),
        .reg_dat_re  (uart_dat_re),
        .reg_dat_di  (mem_wdata),
        .reg_dat_do  (uart_dat_do),
        .reg_dat_wait(uart_dat_wait)
    );

    wire        trap;
    wire [31:0] irq = 0;

    picorv32 #(
        .STACKADDR(STACKADDR),
        .PROGADDR_RESET(PROGADDR_RESET),
        .PROGADDR_IRQ(32'h 0000_0010),
        .ENABLE_MUL(1),
        .ENABLE_DIV(1),
        .ENABLE_IRQ(0),
        .COMPRESSED_ISA(0)
    ) cpu (
        .clk        (clk),
        .resetn     (resetn),
        .trap       (trap),
        .mem_valid  (mem_valid),
        .mem_instr  (mem_instr),
        .mem_ready  (mem_ready),
        .mem_addr   (mem_addr),
        .mem_wdata  (mem_wdata),
        .mem_wstrb  (mem_wstrb),
        .mem_rdata  (mem_rdata),
        .irq        (irq)
    );
endmodule
