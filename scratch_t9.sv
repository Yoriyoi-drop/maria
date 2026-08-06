package scratch_pkg;
  typedef enum bit [1:0] {
    Host,
    Device,
    Monitor
  } if_mode_e;
endpackage

interface scratch_if;
  import scratch_pkg::*;
  logic [1:0] if_mode;
  wire alert_tx;
  assign alert_tx = (if_mode == scratch_pkg::Host) ? 1'b1 : 'z;
endinterface

module scratch_t9;
  scratch_if u_if();
endmodule
