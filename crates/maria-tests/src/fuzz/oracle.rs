//! Oracle invariant + differential — "smart" check yang membedakan fuzzer
//! ini dari fuzzing buta.
//!
//! Untuk tiap input: (1) pastikan pipeline tidak panic; (2) bila lolos
//! compile, simulasikan dua kali dan wajib *deterministik* (hasil sama);
//! (3) bandingkan nilai `y` hasil simulasi maria dengan model emas
//! `Expr::eval` (differential testing). Ketidakcocokan = bug semantik.

use std::panic;

use maria_ir::LogicVec;

use super::gen::GenInput;

pub enum Verdict {
    /// Elaborasi menolak source (diharapkan untuk subset input) — bukan bug.
    CompileFail,
    /// Lolos semua invariant + differential cocok.
    Pass,
    /// Anomali pasti (panic / non-determinism / differential mismatch) →
    /// kegagalan keras.
    Bug(String),
}

pub struct OracleResult {
    pub verdict: Verdict,
    pub compiled: bool,
    pub expected: u64,
    pub actual: Option<u64>,
}

fn mask_of(w: u32) -> u64 {
    if w >= 64 {
        u64::MAX
    } else {
        (1u64 << w) - 1
    }
}

pub fn check(input: &GenInput) -> OracleResult {
    let src = input.to_source();
    let expected = input.expr.eval(input.w, input.a, input.b) & mask_of(input.w);

    // Jalankan di thread dengan stack besar: engine simulasi Maria rekursif
    // dalam & stack default thread test (2 MB) mudah overflow. Stack 256 MB
    // setara environment "multi-threaded" sehingga tidak crash proses.
    let runner = move || -> Option<(bool, Option<u64>, Option<u64>)> {
        let compiled = crate::compile_str(&src).is_ok();
        if !compiled {
            return Some((false, None, None));
        }
        let run1 = crate::simulate_signals(&src, 20).ok();
        let run2 = crate::simulate_signals(&src, 20).ok();
        let y1 = run1
            .as_ref()
            .and_then(|s| s.iter().find(|(n, _)| n == "y").map(|(_, v)| v.to_u64()));
        let y2 = run2
            .as_ref()
            .and_then(|s| s.iter().find(|(n, _)| n == "y").map(|(_, v)| v.to_u64()));
        Some((true, y1, y2))
    };

    let handle = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .name("fuzz-check".to_string())
        .spawn(move || std::panic::catch_unwind(std::panic::AssertUnwindSafe(runner)))
        .ok();

    let res = match handle {
        Some(h) => match h.join() {
            Ok(inner) => inner.map_err(|_| ()),
            Err(_) => Err(()),
        },
        None => Err(()),
    };

    match res {
        Ok(Some((compiled, y1, y2))) => {
            if !compiled {
                return OracleResult {
                    verdict: Verdict::CompileFail,
                    compiled: false,
                    expected,
                    actual: None,
                };
            }
            match (y1, y2) {
                (Some(a), Some(b)) => {
                    if a != b {
                        return OracleResult {
                            verdict: Verdict::Bug(format!(
                                "non-determinism: run1={:#x} run2={:#x}",
                                a & mask_of(input.w),
                                b & mask_of(input.w)
                            )),
                            compiled: true,
                            expected,
                            actual: Some(a),
                        };
                    }
                    let actual = a & mask_of(input.w);
                    if actual == expected {
                        OracleResult {
                            verdict: Verdict::Pass,
                            compiled: true,
                            expected,
                            actual: Some(actual),
                        }
                    } else {
                        OracleResult {
                            verdict: Verdict::Bug(format!(
                                "differential mismatch: expected {:#x} actual {:#x}",
                                expected, actual
                            )),
                            compiled: true,
                            expected,
                            actual: Some(actual),
                        }
                    }
                }
                _ => OracleResult {
                    verdict: Verdict::Bug("compiled but 'y' not readable".to_string()),
                    compiled: true,
                    expected,
                    actual: None,
                },
            }
        }
        Ok(None) => OracleResult {
            verdict: Verdict::Bug("panic during compile/simulate".to_string()),
            compiled: false,
            expected,
            actual: None,
        },
        Err(_) => OracleResult {
            verdict: Verdict::Bug("thread aborted (panic/stack-overflow)".to_string()),
            compiled: false,
            expected,
            actual: None,
        },
    }
}
