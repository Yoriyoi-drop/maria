//! Fuzz differential complex design patterns resembling real projects.
//!
//! Tests: pipelined ALU with random operations, priority arbiter,
//! parameterized counter with enable/load, FSM-like logic.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("complex-design-fuzz-sim".to_string())
        .stack_size(256 * 1024 * 1024)
        .spawn({
            move || {
                crate::simulate_signals(&src, 100)
                    .ok()
                    .and_then(|sigs| {
                        sigs.iter()
                            .find(|(n, _)| *n == "y")
                            .map(|(_, v)| v.to_u64())
                    })
            }
        })
        .expect("spawn")
        .join()
        .expect("sim panic")
}

/// Parameterized ALU: random opcode → random result.
#[test]
fn complex_alu_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_01);
        let a = rng.u64(0..255);
        let b = rng.u64(0..255);
        let op = rng.u32(0..5);

        let (expected, op_name) = match op {
            0 => ((a + b) & 0xFF, "ADD"),
            1 => (a.wrapping_sub(b) & 0xFF, "SUB"),
            2 => ((a & b) & 0xFF, "AND"),
            3 => ((a | b) & 0xFF, "OR"),
            4 => ((a ^ b) & 0xFF, "XOR"),
            _ => unreachable!(),
        };

        let src = format!(
            "module alu_mod;\n\
             \x20   reg [7:0] a, b;\n\
             \x20   reg [2:0] op;\n\
             \x20   wire [7:0] y;\n\
             \x20   always @(*) begin\n\
             \x20       case (op)\n\
             \x20           3'd0: y = a + b;\n\
             \x20           3'd1: y = a - b;\n\
             \x20           3'd2: y = a & b;\n\
             \x20           3'd3: y = a | b;\n\
             \x20           3'd4: y = a ^ b;\n\
             \x20           default: y = 8'd0;\n\
             \x20       endcase\n\
             \x20   end\n\
             \x20   initial begin\n\
             \x20       a = 8'h{a:02x};\n\
             \x20       b = 8'h{b:02x};\n\
             \x20       op = 3'd{op};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            a = a,
            b = b,
            op = op,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} op={}({}) a={:#x} b={:#x} harap={:#x} can={:?}",
                seed, op, op_name, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch ALU:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Priority arbiter: 4 requestors, grant highest priority.
#[test]
fn complex_arbiter_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_02);
        let req = rng.u32(0..15);
        let expected = if req & 8 != 0 { 3 }
        else if req & 4 != 0 { 2 }
        else if req & 2 != 0 { 1 }
        else if req & 1 != 0 { 0 }
        else { 0 };

        let req_hex = format!("{:x}", req);
        let src = format!(
            "module arbiter_mod;\n\
             \x20   reg [3:0] req;\n\
             \x20   reg [1:0] grant;\n\
             \x20   always @(*) begin\n\
             \x20       if (req[3]) grant = 2'd3;\n\
             \x20       else if (req[2]) grant = 2'd2;\n\
             \x20       else if (req[1]) grant = 2'd1;\n\
             \x20       else if (req[0]) grant = 2'd0;\n\
             \x20       else grant = 2'd0;\n\
             \x20   end\n\
             \x20   wire [7:0] y;\n\
             \x20   assign y = {{6'd0, grant}};\n\
             \x20   initial begin\n\
             \x20       req = 4'h{req_hex};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            req_hex = req_hex,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} req={:#x} harap={} can={:?}",
                seed, req, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch arbiter:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Parameterized counter with enable and load.
#[test]
fn complex_counter_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_03);
        let w = [4u32, 8, 16][rng.usize(0..3)];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let max_val = rng.u64(1..=m.min(255));
        let init = rng.u64(0..) & m;

        // Counter counts init, init+1, ..., max_val, wraps to 0
        let steps = rng.u32(1..=10);
        let mut val = init;
        for _ in 0..steps {
            val = if val >= max_val { 0 } else { val + 1 };
        }

        let h = w - 1;
        let init_hex = format!("{:x}", init);
        let max_hex = format!("{:x}", max_val);
        let src = format!(
            "module counter_mod;\n\
             \x20   reg [{h}:0] cnt;\n\
             \x20   reg [{h}:0] max_v;\n\
             \x20   wire [{h}:0] y;\n\
             \x20   assign y = cnt;\n\
             \x20   initial begin\n\
             \x20       cnt = {w}'h{init_hex};\n\
             \x20       max_v = {w}'h{max_hex};\n\
             \x20       repeat ({steps}) begin\n\
             \x20           #1;\n\
             \x20           if (cnt >= max_v)\n\
             \x20               cnt = 0;\n\
             \x20           else\n\
             \x20               cnt = cnt + 1;\n\
             \x20       end\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            h = h,
            w = w,
            init_hex = init_hex,
            max_hex = max_hex,
            steps = steps,
        );

        let actual = run_sim(src);
        if actual != Some(val & m) {
            mismatch.push(format!(
                "seed={} w={} init={:#x} max={:#x} steps={} harap={:#x} can={:?}",
                seed, w, init, max_val, steps, val & m, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch counter:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Barrel shifter: combinational shift by random amount.
#[test]
fn complex_barrel_shifter_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_04);
        let a = rng.u64(0..255);
        let shamt = rng.u32(0..8);
        let arith = rng.bool();
        let expected = if arith {
            ((a as i8) >> shamt) as u64 & 0xFF
        } else {
            (a >> shamt) & 0xFF
        };

        let op = if arith { ">>>" } else { ">>" };
        let a_hex = format!("{:02x}", a);
        let src = format!(
            "module barrel_mod;\n\
             \x20   wire [7:0] y;\n\
             \x20   assign y = $signed(8'h{a_hex}) {op} {shamt};\n\
             \x20   initial begin\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            a_hex = a_hex,
            op = op,
            shamt = shamt,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} a={:#x} shamt={} {} harap={:#x} can={:?}",
                seed, a, shamt, op, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch barrel shifter:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// FSM-like: 4-state controller with random inputs, verify state transitions.
#[test]
fn complex_fsm_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_05);
        // Input: start, stop (2 bits), run for up to 15 cycles
        let start = rng.bool();
        let stop = rng.bool();
        let cycles: u32 = rng.u32(1..=12);

        // Expected FSM: IDLE(0) -> RUN(1) -> DONE(2)
        // SV: state <= NBA, so state seen by posedge is BEFORE update
        // cycle N posedge: evaluate based on current state, NBA sets next_state
        let mut state: u32 = 0; // 0=IDLE, 1=RUN, 2=DONE
        for _ in 0..cycles {
            let next = match state {
                0 => { if start { 1 } else { 0 } }
                1 => { if stop { 0 } else { 2 } }
                2 => { if stop { 0 } else { 2 } }
                _ => 0,
            };
            state = next;
        }
        let expected = state;

        let start_d = if start { "1'b1" } else { "1'b0" };
        let stop_d = if stop { "1'b1" } else { "1'b0" };
        let src = format!(
            "module fsm_mod;\n\
             \x20   reg clk, start_i, stop_i;\n\
             \x20   reg [1:0] state;\n\
             \x20   wire [7:0] y;\n\
             \x20   assign y = {{6'd0, state}};\n\
             \x20   always @(posedge clk) begin\n\
             \x20       case (state)\n\
             \x20           2'd0: if (start_i) state <= 2'd1;\n\
             \x20           2'd1: begin\n\
             \x20               if (stop_i) state <= 2'd0;\n\
             \x20               else state <= 2'd2;\n\
             \x20           end\n\
             \x20           2'd2: if (stop_i) state <= 2'd0;\n\
             \x20           default: state <= 2'd0;\n\
             \x20       endcase\n\
             \x20   end\n\
             \x20   initial begin\n\
             \x20       clk = 0;\n\
             \x20       state = 0;\n\
             \x20       start_i = {start_d};\n\
             \x20       stop_i = {stop_d};\n\
             \x20       repeat ({cycles}) begin\n\
             \x20           #1; clk = 1; #1; clk = 0;\n\
             \x20       end\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            start_d = start_d,
            stop_d = stop_d,
            cycles = cycles,
        );

        let actual = run_sim(src);
        if actual != Some(expected as u64) {
            mismatch.push(format!(
                "seed={} start={} stop={} cycles={} harap={} can={:?}",
                seed, start, stop, cycles, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch FSM:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Ripple carry adder: structural full-adder chain.
#[test]
fn complex_rca_fuzz() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..30u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_06);
        let a = rng.u64(0..255);
        let b = rng.u64(0..255);
        let expected = (a + b) & 0xFF;

        let a_hex = format!("{:02x}", a);
        let b_hex = format!("{:02x}", b);
        // Structural: manually compute full-adder chain in always block
        let src = format!(
            "module rca_mod;\n\
             \x20   reg [7:0] a_i, b_i;\n\
             \x20   wire [7:0] sum;\n\
             \x20   wire [7:0] y;\n\
             \x20   assign y = sum;\n\
             \x20   reg [8:0] carry;\n\
             \x20   integer i;\n\
             \x20   always @(*) begin\n\
             \x20       carry[0] = 0;\n\
             \x20       for (i = 0; i < 8; i = i + 1) begin\n\
             \x20           sum[i] = a_i[i] ^ b_i[i] ^ carry[i];\n\
             \x20           carry[i+1] = (a_i[i] & b_i[i]) | (a_i[i] & carry[i]) | (b_i[i] & carry[i]);\n\
             \x20       end\n\
             \x20   end\n\
             \x20   initial begin\n\
             \x20       a_i = 8'h{a_hex};\n\
             \x20       b_i = 8'h{b_hex};\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            a_hex = a_hex,
            b_hex = b_hex,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} a={:#x} b={:#x} harap={:#x} can={:?}",
                seed, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 15, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch RCA:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
