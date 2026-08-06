package dv_utils_pkg;
  typedef enum bit [1:0] {
    Host,
    Device,
    Monitor
  } if_mode_e;
endpackage

module scratch_t8;
  import dv_utils_pkg::*;
  logic [1:0] mode;
  always_comb begin
    mode = dv_utils_pkg::Device;
  end
endmodule
