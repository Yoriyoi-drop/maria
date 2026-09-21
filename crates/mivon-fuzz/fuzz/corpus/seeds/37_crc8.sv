// Seed 37: CRC-8 (poly 0x07) serial + parallel
module crc8_serial #(parameter W = 8) (
  input  logic clk, rst_n,
  input  logic bit_in, valid,
  output logic [7:0] crc
);
  logic [7:0] crc_q;
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) crc_q <= 8'h00;
    else if (valid) begin
      crc_q[0] <= crc_q[7] ^ bit_in;
      crc_q[1] <= crc_q[0];
      crc_q[2] <= crc_q[1] ^ crc_q[7] ^ bit_in;
      crc_q[3] <= crc_q[2];
      crc_q[4] <= crc_q[3];
      crc_q[5] <= crc_q[4];
      crc_q[6] <= crc_q[5];
      crc_q[7] <= crc_q[6];
    end
  end
  assign crc = crc_q;
endmodule