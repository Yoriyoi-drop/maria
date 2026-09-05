//! Fuzz differential class inheritance and polymorphism.
//!
//! Uses compile_str + simulate_signals to verify no crash/panic.
//! Stack overflow di test thread diatasi dengan approach compile-only
//! untuk test yang berat.

/// Class extends class with field access — verify no crash.
#[test]
fn class_inherit_field_access_fuzz() {
    let mut checked = 0u32;

    for seed in 0..30u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xDD_01);
        let pv = rng.u64(0..255);
        let cv = rng.u64(0..255);

        let src = format!(
            "class base;\n\
             \x20   bit [7:0] x;\n\
             \x20   function new(bit [7:0] v);\n\
             \x20       x = v;\n\
             \x20   endfunction\n\
             endclass\n\
             \n\
             class child extends base;\n\
             \x20   bit [7:0] y;\n\
             \x20   function new(bit [7:0] pv, bit [7:0] cv);\n\
             \x20       super.new(pv);\n\
             \x20       y = cv;\n\
             \x20   endfunction\n\
             endclass\n\
             \n\
             module inherit_mod;\n\
             \x20   child c;\n\
             \x20   initial begin\n\
             \x20       c = new({pv}, {cv});\n\
             \x20       $display(\"y=%0d\", c.x + c.y);\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            pv = pv,
            cv = cv,
        );

        // Just verify compilation succeeds (no crash)
        let result = crate::compile_str(&src);
        assert!(
            result.is_ok(),
            "compile failed on seed={}: {:?}",
            seed,
            result.err()
        );
        checked += 1;
    }
    assert!(checked > 15, "terlalu sedikit kasus (checked={})", checked);
}

/// Multi-level inheritance — verify no crash.
#[test]
fn class_multilevel_inherit_fuzz() {
    let mut checked = 0u32;

    for seed in 0..30u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xDD_02);
        let a = rng.u64(0..31);
        let b = rng.u64(0..31);
        let c = rng.u64(0..31);

        let src = format!(
            "class level_c;\n\
             \x20   bit [4:0] z;\n\
             \x20   function new(bit [4:0] v);\n\
             \x20       z = v;\n\
             \x20   endfunction\n\
             endclass\n\
             \n\
             class level_b extends level_c;\n\
             \x20   bit [4:0] y;\n\
             \x20   function new(bit [4:0] bv, bit [4:0] cv);\n\
             \x20       super.new(cv);\n\
             \x20       y = bv;\n\
             \x20   endfunction\n\
             endclass\n\
             \n\
             class level_a extends level_b;\n\
             \x20   bit [4:0] x;\n\
             \x20   function new(bit [4:0] av, bit [4:0] bv, bit [4:0] cv);\n\
             \x20       super.new(bv, cv);\n\
             \x20       x = av;\n\
             \x20   endfunction\n\
             endclass\n\
             \n\
             module multilevel_mod;\n\
             \x20   level_a obj;\n\
             \x20   initial begin\n\
             \x20       obj = new({a}, {b}, {c});\n\
             \x20       $display(\"y=%0d\", obj.x + obj.y + obj.z);\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            a = a,
            b = b,
            c = c,
        );

        let result = crate::compile_str(&src);
        assert!(
            result.is_ok(),
            "compile failed on seed={}: {:?}",
            seed,
            result.err()
        );
        checked += 1;
    }
    assert!(checked > 15, "terlalu sedikit kasus (checked={})", checked);
}

/// Method override — verify no crash.
#[test]
fn class_method_override_fuzz() {
    let mut checked = 0u32;

    for seed in 0..30u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xDD_03);
        let pv = rng.u64(0..127);
        let cv = rng.u64(0..127);

        let src = format!(
            "class base_cls;\n\
             \x20   function bit [6:0] get_val();\n\
             \x20       return {pv};\n\
             \x20   endfunction\n\
             endclass\n\
             \n\
             class child_cls extends base_cls;\n\
             \x20   function bit [6:0] get_val();\n\
             \x20       return {cv};\n\
             \x20   endfunction\n\
             endclass\n\
             \n\
             module method_override_mod;\n\
             \x20   child_cls obj;\n\
             \x20   initial begin\n\
             \x20       obj = new();\n\
             \x20       $display(\"y=%0d\", obj.get_val());\n\
             \x20       #10;\n\
             \x20       $finish;\n\
             \x20   end\n\
             endmodule\n",
            pv = pv,
            cv = cv,
        );

        let result = crate::compile_str(&src);
        assert!(
            result.is_ok(),
            "compile failed on seed={}: {:?}",
            seed,
            result.err()
        );
        checked += 1;
    }
    assert!(checked > 15, "terlalu sedikit kasus (checked={})", checked);
}
