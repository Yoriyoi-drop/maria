// TB 05_traffic_light — Moore FSM cross-sim differential reference.
// State machine: H_GREEN→H_YELLOW(10)→F_GREEN→F_YELLOW(3)→H_GREEN.
`timescale 1ns/1ps
module tb_traffic_light;
  logic clk = 0;
  logic rst_n = 0;
  logic car_waiting = 0;
  logic [1:0] highway_light;
  logic [1:0] farm_light;

  traffic_light dut (
    .clk(clk), .rst_n(rst_n), .car_waiting(car_waiting),
    .highway_light(highway_light), .farm_light(farm_light)
  );

  always #5 clk = ~clk;

  task tick(input int n);
    repeat (n) @(posedge clk);
    @(negedge clk);
  endtask

  initial begin
    $display("ASRT_START tb_traffic_light");
    rst_n = 0;
    #20;
    rst_n = 1;
    #10;
    @(negedge clk);
    // initial state H_GREEN: highway=01 (green), farm=10 (red)
    assert (highway_light == 2'b01) else $error("ASRT_HG_INIT_BAD=<%02b>", highway_light);
    assert (farm_light == 2'b10) else $error("ASRT_FG_INIT_BAD=<%02b>", farm_light);
    $display("ASRT_HG_INIT=ok");

    // 5 cycles no car → still H_GREEN (timer < GREEN_TIME=10)
    car_waiting = 0;
    tick(5);
    assert (highway_light == 2'b01) else $error("ASRT_HG_STILL_BAD=<%02b>", highway_light);
    $display("ASRT_HG_STILL=ok");

    // wait until timer >= 10 with car → H_YELLOW (highway=10, farm=10)
    car_waiting = 1;
    tick(7); // total 12 cycles → timer 10+
    assert (highway_light == 2'b10) else $error("ASRT_HY_BAD=<%02b>", highway_light);
    assert (farm_light == 2'b10) else $error("ASRT_FY_EQ10_BAD=<%02b>", farm_light);
    $display("ASRT_HY=ok");

    // after YELLOW_TIME=3 → F_GREEN (highway=10, farm=01)
    car_waiting = 0;
    tick(4);
    assert (highway_light == 2'b10) else $error("ASRT_FG_HIGH_BAD=<%02b>", highway_light);
    assert (farm_light == 2'b01) else $error("ASRT_FG_FARM_BAD=<%02b>", farm_light);
    $display("ASRT_FG=ok");

    // after GREEN_TIME=10 → F_YELLOW
    tick(11);
    assert (farm_light == 2'b10) else $error("ASRT_FYL_BAD=<%02b>", farm_light);
    $display("ASRT_FYL=ok");

    // after YELLOW_TIME=3 → back H_GREEN
    tick(4);
    assert (highway_light == 2'b01) else $error("ASRT_BACK_HG_BAD=<%02b>", highway_light);
    $display("ASRT_BACK_HG=ok");

    $display("ASRT_END tb_traffic_light");
    $finish;
  end
endmodule