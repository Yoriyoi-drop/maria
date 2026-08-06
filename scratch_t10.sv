module scratch_t10;
  logic clk, rst_n, sha_en_i, hash_go;
  logic [31:0] round_d, round_q;
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) round_q <= '0;
    else round_q <= round_d;
  end
  always_comb begin : round_counter
    round_d = round_q;
    if (!sha_en_i || hash_go) begin
      round_d = '0;
    end
  end
endmodule
