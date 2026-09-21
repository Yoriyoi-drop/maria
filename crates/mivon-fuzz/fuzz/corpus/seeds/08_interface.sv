// Seed 08: interface + modport + clocking block
interface bus_if #(parameter W = 8);
  logic clk;
  logic req;
  logic ack;
  logic [W-1:0] addr;
  logic [W-1:0] data;

  clocking cb @(posedge clk);
    output req, addr;
    input  ack, data;
  endclocking

  modport master (clocking cb, output req, addr, input ack, data, clk);
  modport slave  (input req, addr, clk, output ack, data);
endinterface

module master (bus_if.master bus);
  assign bus.req = 1'b1;
  assign bus.addr = 8'h10;
endmodule

module slave (bus_if.slave bus);
  assign bus.ack = bus.req;
  assign bus.data = bus.addr + 8'h01;
endmodule

// Top: instansiasi interface + master/slave
module top08 #(parameter W = 8) (
  output logic [W-1:0] ack_out,
  output logic [W-1:0] data_out
);
  bus_if #(.W(W)) b ();
  master u_m (b.master);
  slave  u_s (b.slave);
  assign ack_out = b.ack;
  assign data_out = b.data;
endmodule