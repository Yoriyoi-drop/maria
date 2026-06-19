module tb_counter;
    reg clk;
    reg rst_n;
    wire [3:0] count;

    counter u_counter(
        .clk(clk),
        .rst_n(rst_n),
        .count(count)
    );

    initial begin
        clk = 0;
        rst_n = 0;
        #5 rst_n = 1;
        #100 $finish;
    end

    always #1 clk = ~clk;
endmodule

module counter(
    input clk,
    input rst_n,
    output reg [3:0] count
);
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n)
            count <= 4'b0000;
        else
            count <= count + 4'b0001;
    end
endmodule
