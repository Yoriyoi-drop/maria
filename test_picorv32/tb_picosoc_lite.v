module tb_picosoc_lite;
    reg clk = 0;
    reg resetn = 0;

    always #5 clk = ~clk;

    initial begin
        #15 resetn <= 1;
    end

    wire ser_tx;
    reg ser_rx = 1;

    picosoc_lite soc (
        .clk    (clk),
        .resetn (resetn),
        .ser_tx (ser_tx),
        .ser_rx (ser_rx)
    );

    initial begin
        #200 $finish;
    end
endmodule
