//! Fuzz differential event sensitivity — @(*), @(posedge), @(negedge).
//!
//! Tests: always @(*) triggers on any change, posedge/negedge detection.

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("event-sens-fuzz-sim".to_string())
        .stack_size(256 * 1024 * 1024)
        .spawn({
            move || {
                crate::simulate_signals(&src, 50).ok().and_then(|sigs| {
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

/// always @(*) with blocking assign — output tracks any input change.
#[test]
fn event_star_tracks_input() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xCC_01);
        let w = [4u32, 8, 16, 32][seed as usize % 4];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let a = rng.u64(..) & m;
        let b = rng.u64(..) & m;
        let expected = a | b; // OR

        let a_lit = format!("{}'h{:x}", w, a);
        let b_lit = format!("{}'h{:x}", w, b);

        let src = format!(
            "module test;\n\
             \x20   reg [{hi}:0] a, b;\n\
             \x20   reg [{hi}:0] y;\n\
             \x20   always @(*) y = a | b;\n\
             \x20   initial begin\n\
             \x20       a = {a}; b = {b}; #1;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            a = a_lit,
            b = b_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} b={:#x} harap={:#x} dapat={:?}",
                seed, w, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch event @(*) tracks:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// always @(posedge clk) — register captures on rising edge only.
#[test]
fn event_posedge_only() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xCC_02);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let v0 = rng.u64(..) & m;
        let v1 = rng.u64(..) & m;

        // After two posedges: y = v1 (last captured)
        let expected = v1;

        let v0_lit = format!("{}'h{:x}", w, v0);
        let v1_lit = format!("{}'h{:x}", w, v1);

        let src = format!(
            "module test;\n\
             \x20   reg clk = 0;\n\
             \x20   reg [{hi}:0] d;\n\
             \x20   reg [{hi}:0] y;\n\
             \x20   always @(posedge clk) y <= d;\n\
             \x20   initial begin\n\
             \x20       d = {v0};\n\
             \x20       clk = 1; #1; // posedge 1: y <= v0\n\
             \x20       clk = 0; #1;\n\
             \x20       d = {v1};\n\
             \x20       clk = 1; #1; // posedge 2: y <= v1\n\
             \x20       clk = 0; #1;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            v0 = v0_lit,
            v1 = v1_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} v0={:#x} v1={:#x} harap={:#x} dapat={:?}",
                seed, w, v0, v1, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch posedge only:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// always @(negedge clk) — register captures on falling edge only.
#[test]
fn event_negedge_only() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xCC_03);
        let w = [4u32, 8, 16][seed as usize % 3];
        let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };

        let v0 = rng.u64(..) & m;
        let v1 = rng.u64(..) & m;

        // clk starts at 0, first negedge is actually the initial state transition
        // We need to get past that. Start clk=1 so first transition is negedge.
        let expected = v1;

        let v0_lit = format!("{}'h{:x}", w, v0);
        let v1_lit = format!("{}'h{:x}", w, v1);

        let src = format!(
            "module test;\n\
             \x20   reg clk = 1; // start high\n\
             \x20   reg [{hi}:0] d;\n\
             \x20   reg [{hi}:0] y;\n\
             \x20   always @(negedge clk) y <= d;\n\
             \x20   initial begin\n\
             \x20       d = {v0};\n\
             \x20       clk = 0; #1; // negedge 1: y <= v0\n\
             \x20       clk = 1; #1; // posedge (ignored)\n\
             \x20       d = {v1};\n\
             \x20       clk = 0; #1; // negedge 2: y <= v1\n\
             \x20       clk = 1; #1;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            hi = w - 1,
            v0 = v0_lit,
            v1 = v1_lit,
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} v0={:#x} v1={:#x} harap={:#x} dapat={:?}",
                seed, w, v0, v1, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch negedge only:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
