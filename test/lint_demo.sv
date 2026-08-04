module lint_demo (
    input  logic clk,
    input  logic rst_n,
    input  logic [3:0] din,
    output logic [3:0] dout
);
    logic unused_sig;
    logic [3:0] state;
    logic [7:0] wide;

    assign wide = din;

    always_comb begin
        if (din == 4'h0) begin
            dout = 4'h1;
        end
    end

    always_comb begin
        state = din + state;
        dout = state;
    end

    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state <= 4'h0;
        end else begin
            case (state)
                4'h0: state <= 4'h1;
                4'h1: state <= 4'h2;
                4'h2: state <= 4'h0;
            endcase
        end
    end
endmodule
