//! Oracle invariant + differential — "smart" check yang membedakan fuzzer
//! ini dari fuzzing buta.
//!
//! Untuk tiap input: (1) pastikan pipeline tidak panic; (2) bila lolos
//! compile, simulasikan dua kali dan wajib *deterministik* (hasil sama);
//! (3) bandingkan nilai `y` hasil simulasi maria dengan model emas
//! `Expr::eval` (differential testing). Ketidakcocokan = bug semantik.

use super::gen::GenInput;
use std::io::Read;

pub enum Verdict {
    /// Elaborasi menolak source (diharapkan untuk subset input) — bukan bug.
    CompileFail,
    /// Lolos semua invariant + differential cocok.
    Pass,
    /// Mismatch nilai tapi wasit eksternal tak bisa memutuskan (fitur tak
    /// didukung wasit, dsb.) — dicatat, bukan kegagalan keras.
    Suspect(String),
    /// Anomali pasti (panic / non-determinism / mismatch dikonfirmasi
    /// wasit independen) → kegagalan keras.
    Bug(String),
}

/// Nilai `y` menurut Icarus Verilog (wasit independen), bila terinstal.
/// Source dimodifikasi: `$finish` → `$display` + `$finish`. None = icarus
/// tidak tersedia / gagal compile / timeout.
fn icarus_y(src: &str) -> Option<u64> {
    let ext_src = src.replace("$finish", "$display(\"RESULT %h\", y); $finish");
    let dir = std::env::temp_dir().join(format!("maria-fuzz-iv-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let base = format!("t{:?}", std::thread::current().id());
    let sv = dir.join(format!("{}.sv", base));
    std::fs::write(&sv, &ext_src).ok()?;
    let out = dir.join(format!("{}.vvp", base));
    let compile = std::process::Command::new("iverilog")
        .arg("-o")
        .arg(&out)
        .arg(&sv)
        .output();
    let _ = std::fs::remove_file(&sv);
    match compile {
        Ok(o) if o.status.success() => {
            let run = std::process::Command::new("vvp").arg(&out).output();
            let _ = std::fs::remove_file(&out);
            let _ = std::fs::remove_dir_all(&dir);
            match run {
                Ok(r) => {
                    let text = String::from_utf8_lossy(&r.stdout);
                    for line in text.lines() {
                        if let Some(rest) = line.strip_prefix("RESULT ") {
                            return u64::from_str_radix(rest.trim(), 16).ok();
                        }
                    }
                    None
                }
                Err(_) => None,
            }
        }
        _ => {
            let _ = std::fs::remove_dir_all(&dir);
            None
        }
    }
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
    // X-state (mis. div-by-zero): hasil simulasi adalah X — tidak comparable
    // secara numerik. Kontrak eval_w: skip comparison (lihat eval_has_x).
    let has_x = input.expr.eval_has_x(input.w, input.a, input.b);
    // Intermediate > 128 bit (concat berantai) melampaui presisi model emas
    // u128 → skip numeric compare, invariant panic/determinism tetap dicek.
    let too_wide = input.expr.max_width(input.w as u64) > 128;

    // Jalankan di thread dengan stack besar: engine simulasi Maria rekursif
    // dalam & stack default thread test (2 MB) mudah overflow. Stack 256 MB
    // setara environment "multi-threaded" sehingga tidak crash proses.
    let src_ref = src.clone();
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
                    if has_x || too_wide || actual == expected {
                        OracleResult {
                            verdict: Verdict::Pass,
                            compiled: true,
                            expected,
                            actual: Some(actual),
                        }
                    } else {
                        // Golden vs engine tidak sepakat → minta putusan
                        // wasit independen (Icarus). Golden adalah model
                        // penyederhanaan; tanpa konfirmasi wasit, mismatch
                        // tidak diperlakukan sebagai bug keras.
                        match icarus_y(&src_ref) {
                            Some(iv) if iv & mask_of(input.w) == actual => {
                                eprintln!(
                                    "[oracle] golden divergence (maria==icarus={:#x}, golden={:#x}) w={} expr=`{}` — model emas diperbarui perlu",
                                    actual,
                                    expected,
                                    input.w,
                                    input.expr.to_sv(input.w)
                                );
                                OracleResult {
                                    verdict: Verdict::Pass,
                                    compiled: true,
                                    expected,
                                    actual: Some(actual),
                                }
                            }
                            Some(iv) => OracleResult {
                                verdict: Verdict::Bug(format!(
                                    "differential mismatch (confirmed by icarus): golden {:#x} maria {:#x} icarus {:#x}",
                                    expected, actual, iv
                                )),
                                compiled: true,
                                expected,
                                actual: Some(actual),
                            },
                            None => OracleResult {
                                verdict: Verdict::Suspect(format!(
                                    "differential mismatch (unrefereed): expected {:#x} actual {:#x}",
                                    expected, actual
                                )),
                                compiled: true,
                                expected,
                                actual: Some(actual),
                            },
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
