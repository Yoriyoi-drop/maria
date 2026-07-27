// Test file for post-simulation warnings
module top;
    logic clk;
    logic [7:0] counter;  // uninitialized register
    logic [7:0] unused;   // unused signal - should get WR0104
    
    always_ff @(posedge clk) begin
        counter <= counter + 1;
    end
    
    initial begin
        clk = 0;
        forever #5 clk = ~clk;
    end
    
    initial begin
        #100 $finish;
    end
endmodule
