// Seed 26: streaming operator + bit slicing + part select
module bit_ops #(parameter W = 16) (
  input  logic [W-1:0] a,
  output logic [W-1:0] rev,
  output logic [W-1:0] swapped,
  output logic [3:0] nibble,
  output logic [W-1:0] shr,
  output logic [W-1:0] shl
);
  assign rev     = {<<{a}};           // bit reversal via streaming
  assign swapped = {a[0+:8], a[15:8]}; // part select + swap byte
  assign nibble  = a[8+:4];            // upward part select
  assign shr     = a >> 3;
  assign shl     = a << 2;
endmodule