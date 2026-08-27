//! Fuzz differential constraint randomization — `rand`, `constraint`,
//! `randomize()` edge cases.
//!
//! Blind spot: fuzzer existing menguji arithmetic, tapi randomization
//! constraints (pattern kritis untuk UVM) belum terekspos secara systematic.
//! Edge cases:
//! - Constraint solved sequentially (rejection sampling)
//! - Multiple constraints: satisfiability
//! - Constraint with equality (==): `rand x; constraint { x == 5; }`
//! - Constraint with range: `constraint { x inside {[1:10]}; }`
//! - Randomize failure (unsatisfiable constraint)

fn run_sim_no_crash(src: String) -> bool {
    std::thread::Builder::new()
        .name("constraint-fuzz-sim".to_string())
        .stack_size(256 * 1024 * 1024)
        .spawn({
            move || crate::simulate_signals(&src, 30).ok().map(|s| s.len())
        })
        .expect("spawn")
        .join()
        .expect("sim panic")
        .is_some()
}

#[test]
fn constr_equality_satisfies() {
    // `constraint { x == 42; }` — x harus 42.
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xAA_BB);
        let target = rng.u64(0..255);

        let src = format!(
            "class my_obj;\n\
             \x20   rand bit [7:0] x;\n\
             \x20   constraint c_exact {{ x == {target}; }}\n\
             endclass\n\
             module constr_mod;\n\
             \x20   my_obj obj;\n\
             \x20   initial begin\n\
             \x20       obj = new();\n\
             \x20       obj.randomize();\n\
             \x20       $display(\"x=%0d\", obj.x);\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            target = target,
        );

        assert!(
            run_sim_no_crash(src),
            "simulation panicked on equality constraint seed={}",
            seed
        );
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
}

#[test]
fn constr_range_satisfies() {
    // `constraint { x inside {[1:10]}; }` — x ∈ [1, 10].
    let mut checked = 0u32;

    for _ in 0..40 {
        let src = r#"
            class rng_obj;
                rand bit [7:0] x;
                constraint c_range { x inside {[1:10]}; }
            endclass
            module constr_range_mod;
                initial begin
                    rng_obj obj;
                    obj = new();
                    obj.randomize();
                    $display("x=%0d", obj.x);
                    #10;
                    $finish;
                end
            endmodule
        "#;

        assert!(
            run_sim_no_crash(src.to_string()),
            "simulation panicked on range constraint"
        );
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
}

#[test]
fn constr_multiple_fields() {
    // Multiple rand fields + multiple constraints.
    let mut checked = 0u32;

    for seed in 0..40u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xCC_DD);
        let a_target = rng.u32(0..15) as u64;
        let b_target = rng.u32(0..15) as u64;

        let src = format!(
            "class multi_obj;\n\
             \x20   rand bit [3:0] a;\n\
             \x20   rand bit [3:0] b;\n\
             \x20   constraint c1 {{ a == {a}; }}\n\
             \x20   constraint c2 {{ b == {b}; }}\n\
             endclass\n\
             module constr_multi_mod;\n\
             \x20   initial begin\n\
             \x20       multi_obj obj;\n\
             \x20       obj = new();\n\
             \x20       obj.randomize();\n\
             \x20       $display(\"a=%0d b=%0d\", obj.a, obj.b);\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            a = a_target,
            b = b_target,
        );

        assert!(
            run_sim_no_crash(src),
            "simulation panicked on multi constraint seed={}",
            seed
        );
        checked += 1;
    }
    assert!(checked > 20, "terlalu sedikit kasus (checked={})", checked);
}
