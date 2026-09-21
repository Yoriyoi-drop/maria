// Seed 05: FSM moore — traffic light
module traffic_light (
  input  logic clk, rst_n,
  input  logic car_waiting,
  output logic [1:0] highway_light,
  output logic [1:0] farm_light
);
  typedef enum logic [1:0] { H_GREEN, H_YELLOW, F_GREEN, F_YELLOW } state_t;
  state_t state, next_state;
  logic [3:0] timer;

  parameter logic [3:0] GREEN_TIME = 4'd10;
  parameter logic [3:0] YELLOW_TIME = 4'd3;

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) state <= H_GREEN;
    else        state <= next_state;
  end

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) timer <= '0;
    else if (state != next_state) timer <= '0;
    else timer <= timer + 1'b1;
  end

  always_comb begin
    next_state = state;
    case (state)
      H_GREEN:
        if (car_waiting && timer >= GREEN_TIME) next_state = H_YELLOW;
      H_YELLOW:
        if (timer >= YELLOW_TIME) next_state = F_GREEN;
      F_GREEN:
        if (timer >= GREEN_TIME) next_state = F_YELLOW;
      F_YELLOW:
        if (timer >= YELLOW_TIME) next_state = H_GREEN;
    endcase
  end

  always_comb begin
    case (state)
      H_GREEN:  begin highway_light = 2'b01; farm_light = 2'b10; end
      H_YELLOW: begin highway_light = 2'b10; farm_light = 2'b10; end
      F_GREEN:  begin highway_light = 2'b10; farm_light = 2'b01; end
      F_YELLOW: begin highway_light = 2'b10; farm_light = 2'b10; end
      default:  begin highway_light = 2'b00; farm_light = 2'b00; end
    endcase
  end
endmodule