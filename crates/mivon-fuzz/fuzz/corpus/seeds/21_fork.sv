// Seed 21: fork/join + fork/join_any di testbench
module fork_demo (
  input logic clk,
  output logic [3:0] result
);
  initial begin
    fork
      begin
        @(posedge clk);
        result = 4'h1;
      end
      begin
        @(posedge clk);
        @(posedge clk);
        result = 4'h2;
      end
    join_any
    #5;
    result = 4'hF;
    $finish;
  end
endmodule