// Test file for elaboration error diagnostics
module top;
    // This will trigger an elaboration error
    nonexistent_module u_inst ();
    
    logic clk;
    logic [7:0] counter;
    
    always_ff @(posedge clk) begin
        counter <= counter + 1;
    end
    
    initial begin
        clk = 0;
        forever #5 clk = ~clk;
    end
    
    initial begin
        $monitor("counter = %d", counter);
        #100 $finish;
    end
endmodule
