//! Fuzz differential while/do-while loops.

fn mask_of(w: u32) -> u64 {
    if w >= 64 {
        u64::MAX
    } else {
        (1u64 << w) - 1
    }
}

fn lit_sv(v: u64, w: u32) -> String {
    format!("{}'h{:x}", w, v & mask_of(w))
}

fn run_sim_r(src: &str) -> Option<u64> {
    let src = src.to_string();
    let handle = std::thread::Builder::new()
        .name("while-fuzz-sim".to_string())
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            crate::simulate_signals(&src, 60).ok().and_then(|sigs| {
                sigs.iter()
                    .find(|(n, _)| *n == "r")
                    .map(|(_, v)| v.to_u64())
            })
        })
        .expect("spawn");
    handle.join().expect("sim panic")
}

#[test]
fn while_accumulate_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xB1_01);
        let w = [8u32, 16][seed as usize % 2];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let limit = rng.usize(2..=8) as u64;
        let src = format!(
            r#"module test;
    reg [{hi}:0] a;
    reg [{hi}:0] r;
    integer i;
    initial begin
        a = {av};
        r = 0;
        i = 0;
        while (i < {limit} && i < {w}) begin
            r = r + a[i];
            i = i + 1;
        end
        #10 $finish;
    end
endmodule"#,
            hi = w - 1,
            av = lit_sv(a, w),
            limit = limit,
            w = w,
        );
        let n = limit.min(w as u64);
        let expected: u64 = (0..n).map(|i| (a >> i) & 1).sum();
        let actual = run_sim_r(&src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} limit={} harap={} dapat={:?}",
                seed, w, a, limit, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40);
    assert!(
        mismatch.is_empty(),
        "{} mismatch:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn do_while_accumulate_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xB1_02);
        let w = [8u32, 16][seed as usize % 2];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let n_iter = rng.usize(1..=6) as u64;
        let src = format!(
            r#"module test;
    reg [{hi}:0] a;
    reg [{hi}:0] r;
    integer i;
    initial begin
        a = {av};
        r = 0;
        i = 0;
        do begin
            r = r + a[i];
            i = i + 1;
        end while (i < {n_iter} && i < {w});
        #10 $finish;
    end
endmodule"#,
            hi = w - 1,
            av = lit_sv(a, w),
            n_iter = n_iter,
            w = w,
        );
        let actual_n = n_iter.min(w as u64);
        let expected: u64 = (0..actual_n).map(|i| (a >> i) & 1).sum();
        let actual = run_sim_r(&src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} n_iter={} harap={} dapat={:?}",
                seed, w, a, n_iter, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40);
    assert!(
        mismatch.is_empty(),
        "{} mismatch:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn while_break_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xB1_03);
        let w = [8u32, 16][seed as usize % 2];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let break_at = rng.usize(1..=6);
        let src = format!(
            r#"module test;
    reg [{hi}:0] a;
    reg [{hi}:0] r;
    integer i;
    initial begin
        a = {av};
        r = 0;
        i = 0;
        while (i < {w}) begin
            if (i == {break_at}) break;
            r = r + a[i];
            i = i + 1;
        end
        #10 $finish;
    end
endmodule"#,
            hi = w - 1,
            av = lit_sv(a, w),
            w = w,
            break_at = break_at,
        );
        let expected: u64 = (0..break_at as u64).map(|i| (a >> i) & 1).sum();
        let actual = run_sim_r(&src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} break_at={} harap={} dapat={:?}",
                seed, w, a, break_at, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40);
    assert!(
        mismatch.is_empty(),
        "{} mismatch:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn while_continue_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..80u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xB1_04);
        let w = [8u32, 16][seed as usize % 2];
        let m = mask_of(w);
        let a = rng.u64(..) & m;
        let n = rng.usize(4..=8) as u64;
        let src = format!(
            r#"module test;
    reg [{hi}:0] a;
    reg [{hi}:0] r;
    integer i;
    initial begin
        a = {av};
        r = 0;
        i = 0;
        while (i < {n} && i < {w}) begin
            if (a[i] == 0) begin
                i = i + 1;
                continue;
            end
            r = r + 1;
            i = i + 1;
        end
        #10 $finish;
    end
endmodule"#,
            hi = w - 1,
            av = lit_sv(a, w),
            n = n,
            w = w,
        );
        let actual_n = n.min(w as u64);
        let expected: u64 = (0..actual_n).filter(|&i| (a >> i) & 1 == 1).count() as u64;
        let actual = run_sim_r(&src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} a={:#x} n={} harap={} dapat={:?}",
                seed, w, a, n, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 40);
    assert!(
        mismatch.is_empty(),
        "{} mismatch:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}

#[test]
fn while_nested_multiply_matches_golden() {
    let mut mismatch = Vec::new();
    let mut checked = 0u32;
    for seed in 0..60u64 {
        let mut rng = fastrand::Rng::with_seed(seed ^ 0xB1_05);
        let w = [8u32, 16][seed as usize % 2];
        let m = mask_of(w);
        let a = rng.u64(0..8) & m;
        let b = rng.u64(0..8) & m;
        let src = format!(
            r#"module test;
    reg [{hi}:0] a;
    reg [{hi}:0] b;
    reg [{hi}:0] r;
    integer i;
    integer j;
    initial begin
        a = {av};
        b = {bv};
        r = 0;
        i = 0;
        while (i < a) begin
            j = 0;
            while (j < b) begin
                r = r + 1;
                j = j + 1;
            end
            i = i + 1;
        end
        #10 $finish;
    end
endmodule"#,
            hi = w - 1,
            av = lit_sv(a, w),
            bv = lit_sv(b, w),
        );
        let expected = (a * b) & m;
        let actual = run_sim_r(&src);
        if actual != Some(expected) {
            mismatch.push(format!(
                "seed={} w={} a={} b={} harap={} dapat={:?}",
                seed, w, a, b, expected, actual
            ));
        }
        checked += 1;
    }
    assert!(checked > 30);
    assert!(
        mismatch.is_empty(),
        "{} mismatch:\n{}",
        mismatch.len(),
        mismatch.join("\n")
    );
}
