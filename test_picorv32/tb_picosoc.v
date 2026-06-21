module tb_picosoc;
    reg clk = 0;
    reg resetn = 0;

    always #5 clk = ~clk;

    initial begin
        #15 resetn <= 1;
    end

    wire ser_tx;
    wire ser_rx = 1;

    wire flash_csb;
    wire flash_clk;
    wire flash_io0_oe;
    wire flash_io1_oe;
    wire flash_io2_oe;
    wire flash_io3_oe;
    wire flash_io0_do;
    wire flash_io1_do;
    wire flash_io2_do;
    wire flash_io3_do;
    wire flash_io0_di;
    wire flash_io1_di;
    wire flash_io2_di;
    wire flash_io3_di;

    assign flash_io0_di = flash_io0_oe ? flash_io0_do : 1'bz;
    assign flash_io1_di = flash_io1_oe ? flash_io1_do : 1'bz;
    assign flash_io2_di = flash_io2_oe ? flash_io2_do : 1'bz;
    assign flash_io3_di = flash_io3_oe ? flash_io3_do : 1'bz;

    wire iomem_valid;
    wire iomem_ready;
    wire [3:0] iomem_wstrb;
    wire [31:0] iomem_addr;
    wire [31:0] iomem_wdata;
    wire [31:0] iomem_rdata;

    assign iomem_ready = 0;
    assign iomem_rdata = 0;

    wire irq_5 = 0;
    wire irq_6 = 0;
    wire irq_7 = 0;

    picosoc #(
        .MEM_WORDS(256),
        .ENABLE_MUL(1),
        .ENABLE_DIV(1),
        .ENABLE_COMPRESSED(0),
        .STACKADDR(32'h00000400),
        .PROGADDR_RESET(32'h00000000),
        .PROGADDR_IRQ(32'h00000010)
    ) soc (
        .clk          (clk),
        .resetn       (resetn),
        .ser_tx       (ser_tx),
        .ser_rx       (ser_rx),
        .flash_csb    (flash_csb),
        .flash_clk    (flash_clk),
        .flash_io0_oe (flash_io0_oe),
        .flash_io1_oe (flash_io1_oe),
        .flash_io2_oe (flash_io2_oe),
        .flash_io3_oe (flash_io3_oe),
        .flash_io0_do (flash_io0_do),
        .flash_io1_do (flash_io1_do),
        .flash_io2_do (flash_io2_do),
        .flash_io3_do (flash_io3_do),
        .flash_io0_di (flash_io0_di),
        .flash_io1_di (flash_io1_di),
        .flash_io2_di (flash_io2_di),
        .flash_io3_di (flash_io3_di),
        .irq_5        (irq_5),
        .irq_6        (irq_6),
        .irq_7        (irq_7),
        .iomem_valid  (iomem_valid),
        .iomem_ready  (iomem_ready),
        .iomem_wstrb  (iomem_wstrb),
        .iomem_addr   (iomem_addr),
        .iomem_wdata  (iomem_wdata),
        .iomem_rdata  (iomem_rdata)
    );

    initial begin
        #200 $finish;
    end
endmodule
