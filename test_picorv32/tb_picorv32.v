module tb_picorv32;
    reg clk = 0;
    reg resetn = 0;
    wire trap;

    always #5 clk = ~clk;

    initial begin
        resetn <= 0;
        #15 resetn <= 1;
    end

    wire        mem_valid;
    wire        mem_instr;
    reg         mem_ready;
    wire [31:0] mem_addr;
    wire [31:0] mem_wdata;
    wire [3:0]  mem_wstrb;
    reg  [31:0] mem_rdata;

    reg [31:0] memory [0:127];

    always @(posedge clk) begin
        mem_ready <= 0;
        if (mem_valid && !mem_ready) begin
            mem_ready <= 1;
            if (mem_wstrb) begin
                if (mem_wstrb[0]) memory[mem_addr[31:2]][7:0] <= mem_wdata[7:0];
                if (mem_wstrb[1]) memory[mem_addr[31:2]][15:8] <= mem_wdata[15:8];
                if (mem_wstrb[2]) memory[mem_addr[31:2]][23:16] <= mem_wdata[23:16];
                if (mem_wstrb[3]) memory[mem_addr[31:2]][31:24] <= mem_wdata[31:24];
            end else begin
                mem_rdata <= memory[mem_addr[31:2]];
            end
        end
    end

    wire [31:0] irq = 0;

    picorv32 #(
        .STACKADDR(32'h00000100),
        .PROGADDR_RESET(32'h00000000),
        .PROGADDR_IRQ(32'h00000010),
        .ENABLE_MUL(1),
        .ENABLE_DIV(1),
        .ENABLE_IRQ(0),
        .COMPRESSED_ISA(0)
    ) uut (
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

    initial begin
        #1000 $finish;
    end
endmodule
