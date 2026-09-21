// Seed 28: SPI master — shift + edge detect + counter
module spi_master #(
  parameter CLK_DIV = 4
)(
  input  logic clk, rst_n,
  input  logic start,
  input  logic [7:0] tx_data,
  output logic [7:0] rx_data,
  output logic sclk,
  output logic mosi,
  input  logic miso,
  output logic cs_n,
  output logic busy
);
  typedef enum logic [1:0] { IDLE, TRANSFER, DONE } state_e;
  state_e state;
  logic [7:0] tx_reg, rx_reg;
  logic [2:0] bit_idx;
  integer div_cnt;

  assign sclk = (state == TRANSFER) ? clk : 1'b0;
  assign cs_n = (state == IDLE);

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      state <= IDLE;
      tx_reg <= '0;
      rx_reg <= '0;
      bit_idx <= 0;
      div_cnt <= 0;
    end else begin
      case (state)
        IDLE: begin
          mosi <= 1'b1;
          busy <= 1'b0;
          if (start) begin
            state <= TRANSFER;
            tx_reg <= tx_data;
            bit_idx <= 0;
            div_cnt <= 0;
            busy <= 1'b1;
          end
        end
        TRANSFER: begin
          if (div_cnt == CLK_DIV-1) begin
            div_cnt <= 0;
            mosi <= tx_reg[7];
            tx_reg <= {tx_reg[6:0], 1'b0};
            rx_reg <= {rx_reg[6:0], miso};
            if (bit_idx == 7) state <= DONE;
            else bit_idx <= bit_idx + 1;
          end else div_cnt <= div_cnt + 1;
        end
        DONE: begin
          busy <= 1'b0;
          state <= IDLE;
        end
      endcase
    end
  end

  assign rx_data = rx_reg;
endmodule