// Coverage test: line, toggle, branch, FSM coverage
module coverage_test;
    logic clk = 0;
    logic rst = 1;
    logic [3:0] state = 0;
    logic [3:0] next_state = 0;
    logic [7:0] data = 0;
    logic [7:0] result = 0;

    // Clock generation
    always #5 clk = ~clk;

    // FSM: simple 3-state machine (states: 0, 1, 2)
    always @(posedge clk or posedge rst) begin
        if (rst) begin
            state <= 0;
        end else begin
            state <= next_state;
        end
    end

    // Next state logic (branch coverage test)
    always @(*) begin
        case (state)
            0: begin
                if (data[0])
                    next_state = 1;
                else
                    next_state = 2;
            end
            1: begin
                next_state = 2;
            end
            2: begin
                next_state = 0;
            end
            default: next_state = 0;
        endcase
    end

    // Data generation
    initial begin
        rst = 1;
        #10;
        rst = 0;
        data = 8'hA5;
        #10;
        data = 8'h5A;
        #10;
        data = 8'hFF;
        #10;
        data = 8'h00;
        #10;
        $finish;
    end

    // Result assignment (line coverage)
    always @(state or data) begin
        if (state == 0)
            result = data;
        else if (state == 1)
            result = data << 1;
        else
            result = data >> 1;
    end
endmodule
