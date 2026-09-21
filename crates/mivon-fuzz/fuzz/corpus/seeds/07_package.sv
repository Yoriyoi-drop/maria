// Seed 07: package + typedef + import (self-contained)
package define_pkg;
  typedef logic [7:0] byte_t;
  typedef logic [15:0] halfword_t;
  typedef enum logic [1:0] { IDLE, RUN, DONE } state_e;
  parameter int WIDTH = 16;
  localparam logic [3:0] VERSION = 4'h2;
endpackage

module pkg_user (
  input  logic clk,
  input  logic rst_n,
  output define_pkg::byte_t out
);
  import define_pkg::*;
  define_pkg::state_e st;

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      st <= IDLE;
      out <= '0;
    end else begin
      case (st)
        IDLE: st <= RUN;
        RUN:  begin st <= DONE; out <= out + 1'b1; end
        DONE: st <= IDLE;
      endcase
    end
  end
endmodule