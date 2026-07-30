// Profile test: large valid SystemVerilog design
// 30+ modules, 80+ instances, 500+ lines

module reg8 (
    input clk, rst_n, en,
    input [7:0] d,
    output reg [7:0] q
);
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) q <= 8'h00;
        else if (en) q <= d;
    end
endmodule

module reg16 (
    input clk, rst_n, en,
    input [15:0] d,
    output reg [15:0] q
);
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) q <= 16'h0000;
        else if (en) q <= d;
    end
endmodule

module reg32 (
    input clk, rst_n, en,
    input [31:0] d,
    output reg [31:0] q
);
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) q <= 32'h00000000;
        else if (en) q <= d;
    end
endmodule

module adder8 (
    input [7:0] a, b,
    input ci,
    output [7:0] sum,
    output co
);
    assign {co, sum} = a + b + ci;
endmodule

module adder16 (
    input [15:0] a, b,
    input ci,
    output [15:0] sum,
    output co
);
    assign {co, sum} = a + b + ci;
endmodule

module mux2 (
    input [7:0] a, b,
    input sel,
    output [7:0] y
);
    assign y = sel ? b : a;
endmodule

module counter8 (
    input clk, rst_n, en,
    output reg [7:0] count
);
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) count <= 8'h00;
        else if (en) count <= count + 8'h01;
    end
endmodule

module counter16 (
    input clk, rst_n, en,
    output reg [15:0] count
);
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) count <= 16'h0000;
        else if (en) count <= count + 16'h0001;
    end
endmodule

module shifter8 (
    input [7:0] val,
    input [2:0] amt,
    input dir,
    output [7:0] res
);
    assign res = dir ? (val >> amt) : (val << amt);
endmodule

module priority_enc (
    input [7:0] req,
    output reg [2:0] grant,
    output any_grant
);
    assign any_grant = |req;
    always_comb begin
        grant = 3'h0;
        if (req[0]) grant = 3'h0;
        else if (req[1]) grant = 3'h1;
        else if (req[2]) grant = 3'h2;
        else if (req[3]) grant = 3'h3;
        else if (req[4]) grant = 3'h4;
        else if (req[5]) grant = 3'h5;
        else if (req[6]) grant = 3'h6;
        else if (req[7]) grant = 3'h7;
    end
endmodule

module dec3to8 (
    input [2:0] sel,
    output reg [7:0] y
);
    always_comb begin
        case (sel)
            3'h0: y = 8'h01;
            3'h1: y = 8'h02;
            3'h2: y = 8'h04;
            3'h3: y = 8'h08;
            3'h4: y = 8'h10;
            3'h5: y = 8'h20;
            3'h6: y = 8'h40;
            3'h7: y = 8'h80;
        endcase
    end
endmodule

module comparator8 (
    input [7:0] a, b,
    output eq, lt, gt
);
    assign eq = (a == b);
    assign lt = (a < b);
    assign gt = (a > b);
endmodule

module edge_det (
    input clk, rst_n, sig,
    output reg rising, falling
);
    reg sig_d;
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            sig_d <= 1'h0;
            rising <= 1'h0;
            falling <= 1'h0;
        end else begin
            sig_d <= sig;
            rising <= sig & ~sig_d;
            falling <= ~sig & sig_d;
        end
    end
endmodule

module pulse_gen (
    input clk, rst_n,
    input [15:0] period,
    output reg pulse
);
    reg [15:0] cnt;
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            cnt <= 16'h0000;
            pulse <= 1'h0;
        end else begin
            if (cnt >= period - 16'h0001) begin
                cnt <= 16'h0000;
                pulse <= 1'h1;
            end else begin
                cnt <= cnt + 16'h0001;
                pulse <= 1'h0;
            end
        end
    end
endmodule

module sync2 (
    input clk, rst_n,
    input async,
    output reg sync
);
    reg meta;
    always_ff @(posedge clk) begin
        meta <= async;
        sync <= meta;
    end
endmodule

module sr_latch (
    input s, r,
    output reg q, qn
);
    always_comb begin
        if (s & ~r) begin q = 1'h1; qn = 1'h0; end
        else if (~s & r) begin q = 1'h0; qn = 1'h1; end
        else if (s & r) begin q = 1'h0; qn = 1'h0; end
    end
endmodule

// Medium modules with hierarchy

module alu8 (
    input clk, rst_n,
    input [7:0] a, b,
    input [2:0] op,
    input en,
    output reg [7:0] result,
    output zero
);
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) result <= 8'h00;
        else if (en) begin
            case (op)
                3'h0: result <= a + b;
                3'h1: result <= a - b;
                3'h2: result <= a & b;
                3'h3: result <= a | b;
                3'h4: result <= a ^ b;
                3'h5: result <= a << b[2:0];
                3'h6: result <= a >> b[2:0];
                3'h7: result <= a < b ? 8'h01 : 8'h00;
            endcase
        end
    end
    assign zero = (result == 8'h00);
endmodule

module fifo8 (
    input clk, rst_n,
    input wr, rd,
    input [7:0] din,
    output reg [7:0] dout,
    output full, empty
);
    reg [7:0] mem [0:7];
    reg [2:0] wr_ptr, rd_ptr;
    reg [3:0] count;

    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            wr_ptr <= 3'h0;
            rd_ptr <= 3'h0;
            count <= 4'h0;
        end else begin
            if (wr && !full) begin
                mem[wr_ptr] <= din;
                wr_ptr <= wr_ptr + 3'h1;
            end
            if (rd && !empty) begin
                rd_ptr <= rd_ptr + 3'h1;
            end
            if (wr && !full && !(rd && !empty)) count <= count + 4'h1;
            else if (rd && !empty && !(wr && !full)) count <= count - 4'h1;
        end
    end
    assign dout = mem[rd_ptr];
    assign full = (count == 4'h8);
    assign empty = (count == 4'h0);
endmodule

module pipeline_stage (
    input clk, rst_n, en,
    input [7:0] a, b,
    output [7:0] sum, diff, and_op, or_op
);
    reg [7:0] a_reg, b_reg;
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            a_reg <= 8'h00;
            b_reg <= 8'h00;
        end else if (en) begin
            a_reg <= a;
            b_reg <= b;
        end
    end
    assign sum   = a_reg + b_reg;
    assign diff  = a_reg - b_reg;
    assign and_op = a_reg & b_reg;
    assign or_op  = a_reg | b_reg;
endmodule

module sync_module (
    input clk, rst_n,
    input [7:0] data_in,
    output [7:0] data_out,
    output ready
);
    reg [7:0] sync_reg;
    reg sync_ready;

    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            sync_reg <= 8'h00;
            sync_ready <= 1'h0;
        end else begin
            sync_reg <= data_in;
            sync_ready <= 1'h1;
        end
    end
    assign data_out = sync_reg;
    assign ready = sync_ready;
endmodule

// Complex module with internal sub-module instances

module filter_stage (
    input clk, rst_n, en,
    input [7:0] din,
    output [7:0] dout
);
    wire [7:0] reg1_out, reg2_out, reg3_out, sum_out;

    reg8 r1 (.clk(clk), .rst_n(rst_n), .en(en), .d(din), .q(reg1_out));
    reg8 r2 (.clk(clk), .rst_n(rst_n), .en(en), .d(reg1_out), .q(reg2_out));
    reg8 r3 (.clk(clk), .rst_n(rst_n), .en(en), .d(reg2_out), .q(reg3_out));
    adder8 add (.a(reg1_out), .b(reg3_out), .ci(1'h0), .sum(sum_out), .co());
    mux2 mux (.a(sum_out), .b(reg2_out), .sel(en), .y(dout));
endmodule

module complex_sub_system (
    input clk, rst_n,
    input [7:0] data_a, data_b,
    input start,
    output [7:0] result,
    output done
);
    wire en, alu_en;
    wire [7:0] alu_out;
    wire [7:0] cnt_out;
    wire zero;
    wire [2:0] alu_op;

    reg8 ctrl_reg (.clk(clk), .rst_n(rst_n), .en(1'h1), .d(data_a), .q(alu_op));
    alu8 alu (.clk(clk), .rst_n(rst_n), .a(data_a), .b(data_b), .op(alu_op), .en(start), .result(alu_out), .zero(zero));
    counter8 cnt (.clk(clk), .rst_n(rst_n), .en(start), .count(cnt_out));
    comparator8 cmp (.a(cnt_out), .b(8'h10), .eq(done), .lt(), .gt());
    mux2 res_mux (.a(alu_out), .b(cnt_out), .sel(done), .y(result));
endmodule

// Top module: instantiates everything

module profile_top (
    input clk, rst_n,
    input [7:0] sw, btn,
    output [7:0] led,
    output [7:0] debug
);
    wire [7:0] reg_a, reg_b, reg_c;
    wire [15:0] reg16_a;
    wire [31:0] reg32_a;
    wire [7:0] sum8, mux_out, cnt_val, shift_res, alu_res;
    wire co8;
    wire [2:0] pri_grant;
    wire pri_any, zero_flag;
    wire fifo_full, fifo_empty;
    wire filt_out;

    // 30 instances total
    reg8   u0 (.clk(clk), .rst_n(rst_n), .en(btn[0]), .d(sw), .q(reg_a));
    reg8   u1 (.clk(clk), .rst_n(rst_n), .en(btn[1]), .d(reg_a), .q(reg_b));
    reg8   u2 (.clk(clk), .rst_n(rst_n), .en(btn[2]), .d(reg_b), .q(reg_c));
    reg16  u3 (.clk(clk), .rst_n(rst_n), .en(btn[3]), .d({reg_a, reg_b}), .q(reg16_a));
    reg32  u4 (.clk(clk), .rst_n(rst_n), .en(btn[4]), .d({reg16_a, reg_a, 8'h00}), .q(reg32_a));
    adder8 u5 (.a(reg_a), .b(reg_b), .ci(1'h0), .sum(sum8), .co(co8));
    mux2   u6 (.a(reg_a), .b(reg_b), .sel(btn[5]), .y(mux_out));
    counter8 u7 (.clk(clk), .rst_n(rst_n), .en(btn[6]), .count(cnt_val));
    counter16 u8 (.clk(clk), .rst_n(rst_n), .en(btn[7]), .count());
    shifter8 u9 (.val(reg_a), .amt(cnt_val[2:0]), .dir(btn[0]), .res(shift_res));
    priority_enc u10 (.req(reg_a), .grant(pri_grant), .any_grant(pri_any));
    dec3to8 u11 (.sel(pri_grant), .y());
    comparator8 u12 (.a(reg_a), .b(reg_b), .eq(), .lt(), .gt());
    edge_det u13 (.clk(clk), .rst_n(rst_n), .sig(btn[7]), .rising(), .falling());
    pulse_gen u14 (.clk(clk), .rst_n(rst_n), .period(16'h0100), .pulse());
    sync2 u15 (.clk(clk), .rst_n(rst_n), .async(btn[6]), .sync());
    sr_latch u16 (.s(btn[0]), .r(btn[1]), .q(), .qn());
    alu8 u17 (.clk(clk), .rst_n(rst_n), .a(reg_a), .b(reg_b), .op(cnt_val[2:0]), .en(btn[5]), .result(alu_res), .zero(zero_flag));
    fifo8 u18 (.clk(clk), .rst_n(rst_n), .wr(btn[0]), .rd(btn[1]), .din(sw), .dout(), .full(fifo_full), .empty(fifo_empty));
    pipeline_stage u19 (.clk(clk), .rst_n(rst_n), .en(btn[3]), .a(reg_a), .b(reg_b), .sum(), .diff(), .and_op(), .or_op());
    sync_module u20 (.clk(clk), .rst_n(rst_n), .data_in(reg_a), .data_out(), .ready());
    filter_stage u21 (.clk(clk), .rst_n(rst_n), .en(btn[4]), .din(reg_a), .dout(filt_out));
    complex_sub_system u22 (.clk(clk), .rst_n(rst_n), .data_a(reg_a), .data_b(reg_b), .start(btn[7]), .result(led), .done());
    reg8 u23 (.clk(clk), .rst_n(rst_n), .en(1'h1), .d(cnt_val), .q());
    reg8 u24 (.clk(clk), .rst_n(rst_n), .en(1'h1), .d(shift_res), .q());
    reg8 u25 (.clk(clk), .rst_n(rst_n), .en(1'h1), .d(alu_res), .q());
    reg8 u26 (.clk(clk), .rst_n(rst_n), .en(1'h1), .d(filt_out), .q());
    reg8 u27 (.clk(clk), .rst_n(rst_n), .en(1'h1), .d(mux_out), .q());
    reg8 u28 (.clk(clk), .rst_n(rst_n), .en(1'h1), .d(sw), .q());
    reg8 u29 (.clk(clk), .rst_n(rst_n), .en(1'h1), .d(sum8), .q());

    assign debug = cnt_val;

    // Simulation control
    initial begin
        $display("Profile test started");
        #100;
        $display("Profile test complete");
        $finish;
    end
endmodule

// Additional modules in separate files (simulated in same file)

module wrapper_a (
    input clk, rst_n,
    input [7:0] a, b, c,
    output [7:0] out
);
    wire [7:0] sum1, sum2;
    adder8 add1 (.a(a), .b(b), .ci(1'h0), .sum(sum1), .co());
    adder8 add2 (.a(sum1), .b(c), .ci(1'h0), .sum(sum2), .co());
    reg8 reg_out (.clk(clk), .rst_n(rst_n), .en(1'h1), .d(sum2), .q(out));
endmodule

module wrapper_b (
    input clk, rst_n,
    input [7:0] d,
    output [7:0] q1, q2, q3
);
    reg8 r1 (.clk(clk), .rst_n(rst_n), .en(1'h1), .d(d), .q(q1));
    reg8 r2 (.clk(clk), .rst_n(rst_n), .en(1'h1), .d(q1), .q(q2));
    reg8 r3 (.clk(clk), .rst_n(rst_n), .en(1'h1), .d(q2), .q(q3));
endmodule

module wrapper_c (
    input clk, rst_n,
    input [7:0] x, y,
    output [7:0] z
);
    wire [7:0] diff, neg;
    wire lt;
    comparator8 cmp (.a(x), .b(y), .eq(), .lt(lt), .gt());
    adder8 add (.a(x), .b(y), .ci(1'h0), .sum(z), .co());
endmodule

module wrapper_d (
    input clk, rst_n,
    input [7:0] sel,
    output [7:0] out
);
    wire [7:0] cnt;
    counter8 cnt_inst (.clk(clk), .rst_n(rst_n), .en(1'h1), .count(cnt));
    assign out = cnt + sel;
endmodule

module wrapper_e (
    input clk, rst_n,
    input [7:0] din,
    output [7:0] dout
);
    wire [7:0] s1, s2;
    shifter8 sh1 (.val(din), .amt(3'h1), .dir(1'h0), .res(s1));
    shifter8 sh2 (.val(s1), .amt(3'h2), .dir(1'h1), .res(s2));
    assign dout = s2;
endmodule

// Many small modules for scale testing

module scale_module1 (
    input a, b, c,
    output y
);
    assign y = a & b | c;
endmodule

module scale_module2 (
    input a, b, c,
    output y
);
    assign y = a ^ b ^ c;
endmodule

module scale_module3 (
    input a, b, c,
    output y
);
    assign y = (a & b) | (~a & c);
endmodule

module scale_module4 (
    input a, b,
    output y
);
    assign y = ~(a & b);
endmodule

module scale_module5 (
    input a, b,
    output y
);
    assign y = ~(a | b);
endmodule

module scale_module6 (
    input a, b,
    output y
);
    assign y = a & ~b;
endmodule

module scale_module7 (
    input a, b,
    output y
);
    assign y = ~a & b;
endmodule

module scale_module8 (
    input a, b,
    output y
);
    assign y = a | ~b;
endmodule
