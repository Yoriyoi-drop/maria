// Seed 25: UART TX — shift out serial, bit bang, baud counter
module uart_tx #(
  parameter CLK_PER_BIT = 10
)(
  input  logic clk, rst_n,
  input  logic tx_start,
  input  logic [7:0] tx_data,
  output logic tx_line,
  output logic tx_done
);
  typedef enum logic [1:0] { IDLE, START, DATA, STOP } state_e;
  state_e state;
  logic [7:0] shift;
  logic [3:0] bit_cnt;
  integer baud_cnt;

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      state <= IDLE;
      tx_line <= 1'b1;
      tx_done <= 1'b0;
      baud_cnt <= 0;
      bit_cnt <= 0;
    end else begin
      tx_done <= 1'b0;
      case (state)
        IDLE: begin
          tx_line <= 1'b1;
          if (tx_start) begin
            state <= START;
            shift <= tx_data;
            baud_cnt <= 0;
          end
        end
        START: begin
          tx_line <= 1'b0;
          if (baud_cnt == CLK_PER_BIT-1) begin
            baud_cnt <= 0;
            state <= DATA;
            bit_cnt <= 0;
          end else baud_cnt <= baud_cnt + 1;
        end
        DATA: begin
          tx_line <= shift[0];
          if (baud_cnt == CLK_PER_BIT-1) begin
            baud_cnt <= 0;
            shift <= {1'b0, shift[7:1]};
            if (bit_cnt == 7) state <= STOP;
            else bit_cnt <= bit_cnt + 1;
          end else baud_cnt <= baud_cnt + 1;
        end
        STOP: begin
          tx_line <= 1'b1;
          if (baud_cnt == CLK_PER_BIT-1) begin
            baud_cnt <= 0;
            state <= IDLE;
            tx_done <= 1'b1;
          end else baud_cnt <= baud_cnt + 1;
        end
      endcase
    end
  end
endmodule