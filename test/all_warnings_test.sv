// Test file for all post-simulation warnings
module top;
    // WR0014: Uninitialized register — logic with all-X, never assigned
    logic [7:0] uninit_reg;
    
    // WR0104: Unused signal — has defined init but never changes
    logic [7:0] tied_low = 8'h00;
    
    // Clock that never toggles — should trigger WR0202
    logic dead_clk;
    
    // Reset that stays asserted — should trigger WR0203
    logic [7:0] counter;
    logic rst_n;
    
    always_ff @(posedge dead_clk or negedge rst_n) begin
        if (!rst_n)
            counter <= 8'h00;
        else
            counter <= counter + 1;
    end
    
    initial begin
        rst_n = 0;  // reset asserted, never released
        dead_clk = 0;  // clock stays low, never toggles
        // uninit_reg and tied_low are never assigned
        #100 $finish;
    end
endmodule
