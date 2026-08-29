//! Fuzz differential struct field access and operations.
//!
//! Tests:
//! - Struct field read/write
//! - Struct field in expressions
//! - Struct assignment
//! - Packed struct operations

fn mask_of(w: u32) -> u64 {
    if w >= 64 { u64::MAX } else { (1u64 << w) - 1 }
}

fn lit_sv(v: u64, w: u32) -> String {
    format!("{}'h{:x}", w, v & mask_of(w))
}

fn run_sim(src: String) -> Option<u64> {
    std::thread::Builder::new()
        .name("struct-field-sim".to_string())
        .stack_size(256 * 1024 * 1024)
        .spawn({
            move || {
                crate::simulate_signals(&src, 30)
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

/// Packed struct: read individual fields.
#[test]
fn packed_struct_field_read_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xC1_01);
        let a = rng.u64(0..16);
        let b = rng.u64(0..16);

        let expected = (a + b) & 0xF;

        let src = format!(
            r#"module struct_fuzz_mod;
    typedef struct packed {{
        logic [3:0] a;
        logic [3:0] b;
    }} ps_t;
    ps_t s;
    wire [3:0] y;
    assign y = s.a + s.b;
    initial begin
        s.a = 4'h{:x};
        s.b = 4'h{:x};
        #10;
        $finish;
    end
endmodule"#,
            a, b
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} a={} b={} harap={} dapat={:?}",
                seed, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch struct field read:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Packed struct: write to field and read back.
#[test]
fn packed_struct_field_write_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xC1_02);
        let a = rng.u64(0..16);
        let b = rng.u64(0..16);

        let expected = b;

        let src = format!(
            r#"module struct_fuzz_mod;
    typedef struct packed {{
        logic [3:0] a;
        logic [3:0] b;
    }} ps_t;
    ps_t s;
    wire [3:0] y;
    assign y = s.b;
    initial begin
        s.a = 4'h{:x};
        s.b = 4'h{:x};
        #10;
        $finish;
    end
endmodule"#,
            a, b
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} a={} b={} harap={} dapat={:?}",
                seed, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch struct field write:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Packed struct: whole struct assignment.
#[test]
fn packed_struct_whole_assign_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xC1_03);
        let a = rng.u64(0..16);
        let b = rng.u64(0..16);

        let expected = a;

        let src = format!(
            r#"module struct_fuzz_mod;
    typedef struct packed {{
        logic [3:0] a;
        logic [3:0] b;
    }} ps_t;
    ps_t s1;
    ps_t s2;
    wire [3:0] y;
    assign y = s2.a;
    initial begin
        s1.a = 4'h{:x};
        s1.b = 4'h{:x};
        s2 = s1;
        #10;
        $finish;
    end
endmodule"#,
            a, b
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} a={} b={} harap={} dapat={:?}",
                seed, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch struct whole assign:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

/// Unpacked struct: field operations.
#[test]
fn unpacked_struct_field_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;

    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xC1_04);
        let a = rng.u64(0..8);
        let b = rng.u64(0..8);

        let expected = (a * b) & 0xF;

        let src = format!(
            r#"module struct_fuzz_mod;
    typedef struct {{
        logic [3:0] a;
        logic [3:0] b;
    }} us_t;
    us_t s;
    wire [3:0] y;
    assign y = s.a * s.b;
    initial begin
        s.a = 4'h{:x};
        s.b = 4'h{:x};
        #10;
        $finish;
    end
endmodule"#,
            a, b
        );

        let actual = run_sim(src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} a={} b={} harap={} dapat={:?}",
                seed, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40, "terlalu sedikit kasus (checked={})", checked);
    assert!(
        mismatch.is_empty(),
        "{} mismatch unpacked struct:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
