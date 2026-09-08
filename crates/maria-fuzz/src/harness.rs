//! Harness eksekusi terisolasi — design-agnostic (Paper #18, Trippel et al.,
//! "Fuzzing Hardware Like Software"): input dikerjakan di thread sendiri,
//! panic di-catch (tidak meracuni state fuzzer), hang dideteksi via timeout.
//! Karena thread tidak bisa dibunuh di Rust, thread hang di-leak (jumlah
//! terbatasi `iters`) — trade-off standar fuzzer.
//!
//! Isolasi mutex-poison: panic maria dalam thread = state thread itu saja
//! yang rusak; thread dibuang setelah selesai (tidak dipakai ulang utk input
//! lain), sehingga run berikutnya bersih.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::oracle::{
    compile_verdict, sim_verdict, sim_verdict_cov, CompileVerdict, SimVerdict,
};

/// Stack thread worker maria (byte). Parsing/elaborasi file besar dengan
/// nesting dalam (file corpora nyata, mis. regtop OpenTitan) butuh stack
/// jauh di atas default 2MB — main.rs sudah memakai 256MB utk thread utama
/// (MARIA_STACK... helper di bawah). Stack overflow = abort proses tak
/// tertangkap catch_unwind (fatal), jadi worker fuzzing W AjIB punya stack
/// sebesar itu. Reserved virtual, ter-commit saat dipakai — aman walau
/// thread hang di-leak.
pub const WORKER_STACK_BYTES: usize = 256 * 1024 * 1024;

fn spawn_isolated<F>(f: F)
where
    F: FnOnce() + Send + 'static,
{
    let _ = std::thread::Builder::new()
        .name("maria-fuzz-worker".into())
        .stack_size(WORKER_STACK_BYTES)
        .spawn(f);
}

/// Status eksekusi satu input.
#[derive(Debug, Clone, PartialEq)]
pub enum RunStatus {
    /// Selesai normal (compile ok/err, sim ok/err) — lihat verdict-nya.
    Done,
    /// Panic tertangkap di pipeline maria — BUG.
    Panic(String),
    /// Melewati ambang waktu — diduga infinite loop — BUG.
    Hang,
}

/// Hasil satu eksekusi terisolasi.
#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub status: RunStatus,
    pub compile: CompileVerdict,
    pub sim: Option<SimVerdict>,
    /// Coverage keys eksekusi nyata (line/branch/toggle/FSM) — feedback
    /// coverage-guided yang diambil dalam SATU eksekusi (tanpa sim ulang).
    pub coverage: Vec<String>,
    pub duration_ms: u64,
}

/// Jalankan compile + simulasi dalam thread terisolasi.
pub fn run_isolated(source: &str, max_time: u64, hang_ms: u64) -> RunOutcome {
    let (tx, rx) = mpsc::channel();
    let source = source.to_string();
    spawn_isolated(move || {
        let started = Instant::now();
        let result = run_inner(&source, max_time);
        let _ = tx.send((result, started.elapsed().as_millis() as u64));
    });
    match rx.recv_timeout(Duration::from_millis(hang_ms)) {
        Ok((out, dur)) => RunOutcome {
            duration_ms: dur,
            ..out
        },
        Err(_) => {
            // Thread hang → di-leak (sampai selesai atau mati proses).
            RunOutcome {
                status: RunStatus::Hang,
                compile: CompileVerdict {
                    ok: false,
                    code: "HANG".to_string(),
                    message: format!(">{} ms", hang_ms),
                },
                sim: None,
                coverage: Vec::new(),
                duration_ms: hang_ms,
            }
        }
    }
}

/// Jalankan simulasi saja, kembalikan fingerprint (untuk differential #13/#19).
/// None = gagal compile/sim atau hang.
/// Catatan: fingerprint SUDAH berisi sinyal flatten child module (`u.t`) —
/// elaborator flatten mengangkat seluruh sinyal ke `top.signals`, jadi fault
/// internal child terlihat tanpa jalur terpisah (terverifikasi via test
/// `flattened_fingerprint_observes_child_internal_fault`).
pub fn fingerprint_isolated(source: &str, max_time: u64, hang_ms: u64) -> Option<String> {
    let (tx, rx) = mpsc::channel();
    let source = source.to_string();
    spawn_isolated(move || {
        let _ = tx.send(run_sim_fingerprint(&source, max_time));
    });
    rx.recv_timeout(Duration::from_millis(hang_ms)).ok().flatten()
}

/// Jalankan simulasi dgn trace sampling MID-SIMULATION (interval waktu) →
/// fingerprint trace: satu baris snapshot sinyal top per interval + nilai
/// final. Bug transient (salah di delta lalu pulih) hanya terlihat di sini.
/// None = gagal/hang.
pub fn trace_isolated(source: &str, max_time: u64, hang_ms: u64, interval: u64) -> Option<String> {
    let (tx, rx) = mpsc::channel();
    let source = source.to_string();
    spawn_isolated(move || {
        let r = catch_unwind(AssertUnwindSafe(|| {
            let (_, trace) =
                maria_api::simulate_signals_with_trace_quiet(&source, max_time, interval).ok()?;
            Some(trace.join("\n"))
        }));
        let _ = tx.send(r.unwrap_or(None));
    });
    rx.recv_timeout(Duration::from_millis(hang_ms)).ok().flatten()
}

/// Jalankan simulasi saja, kembalikan ERROR CODE dari SimError (mis. "RT7001"),
/// atau None bila compile/sim ok (tak ada error) / hang.
/// Dipakai assertion-oracle: pastikan minimasi mempertahankan error code yang
/// sama (RT7001 = assertion `$fatal` fail), bukan error lain (RT0001 dll.).
pub fn sim_err_isolated(source: &str, max_time: u64, hang_ms: u64) -> Option<String> {
    let (tx, rx) = mpsc::channel();
    let source = source.to_string();
    spawn_isolated(move || {
        let r = catch_unwind(AssertUnwindSafe(|| {
            sim_verdict(&source, max_time).code
        }));
        let _ = tx.send(r.ok().filter(|c| !c.is_empty()));
    });
    rx.recv_timeout(Duration::from_millis(hang_ms)).ok().flatten()
}

/// Jalankan compile saja, kembalikan Ok(message) / Err(panic message).
/// Dipakai minimizer (predikat = masih panic).
pub fn compile_only_isolated(source: &str, hang_ms: u64) -> Result<String, String> {
    let (tx, rx) = mpsc::channel();
    let source = source.to_string();
    spawn_isolated(move || {
        let r = catch_unwind(AssertUnwindSafe(|| {
            let v = compile_verdict(&source);
            (v.ok, v.message)
        }));
        let _ = tx.send(r);
    });
    match rx.recv_timeout(Duration::from_millis(hang_ms)) {
        Ok(Ok((ok, msg))) => {
            if ok {
                Ok(msg)
            } else {
                // Compile err = bukan panic → "Ok" bagi minimizer (tidak bug).
                Ok(msg)
            }
        }
        Ok(Err(panic_payload)) => Err(panic_string(panic_payload)),
        Err(_) => Err(format!("hang > {} ms", hang_ms)),
    }
}

/// Eksekusi inti: catch_unwind di sekitar compile + sim.
fn run_inner(source: &str, max_time: u64) -> RunOutcome {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let compile = compile_verdict(source);
        let (sim, coverage) = if compile.ok {
            let (b, c) = sim_verdict_cov(source, max_time);
            (Some(b), c)
        } else {
            (None, Vec::new())
        };
        RunOutcome {
            status: RunStatus::Done,
            compile,
            sim,
            coverage,
            duration_ms: 0,
        }
    }));
    match result {
        Ok(out) => out,
        Err(payload) => RunOutcome {
            status: RunStatus::Panic(panic_string(payload)),
            compile: CompileVerdict {
                ok: false,
                code: "PANIC".to_string(),
                message: String::new(),
            },
            sim: None,
            coverage: Vec::new(),
            duration_ms: 0,
        },
    }
}

/// Simulasi → fingerprint (None bila gagal). Di-selubungi catch_unwind.
fn run_sim_fingerprint(source: &str, max_time: u64) -> Option<String> {
    let r = catch_unwind(AssertUnwindSafe(|| {
        let v = sim_verdict(source, max_time);
        if v.ok {
            Some(v.fingerprint)
        } else {
            None
        }
    }));
    r.unwrap_or(None)
}

/// Ekstrak pesan panic dari payload.
fn panic_string(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic non-string".to_string()
    }
}

// ──────────────────────────────────────────────────────────────────────
// Eksekusi SUBPROCESS (GAP-9, opt-in `--proc-iso`). Thread Rust tak bisa
// dibunuh → hang sejati me-leak thread yang terus membakar CPU. Subprocess
// bisa di-KILL sejati, dan stack-overflow (SIGSEGV/abort) yang tak terjangkau
// catch_unwind terdeteksi lewat exit code. Child = binary ini sendiri dalam
// mode `--slave`: baca source dari stdin, compile+sim senyap, cetak baris
// hasil, exit.
// ──────────────────────────────────────────────────────────────────────

/// Mode slave (`--slave`): baca source dari stdin → compile+sim senyap →
/// cetak `OK|compile_ok|compile_code|sim_ok|sim_code|dur|n_cov|cov,..` atau
/// `PANIC|<msg>` ke stdout, exit. Dipanggil hanya oleh `run_isolated_proc`.
pub fn run_slave() -> ! {
    use std::io::Read;
    let mut src = String::new();
    let _ = std::io::stdin().read_to_string(&mut src);
    let max_time = std::env::var("MARIA_FUZZ_SLAVE_MAX_TIME")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);
    let started = Instant::now();
    let result = catch_unwind(AssertUnwindSafe(|| {
        let compile = compile_verdict(&src);
        let (sim, coverage) = if compile.ok {
            let (b, c) = sim_verdict_cov(&src, max_time);
            (Some(b), c)
        } else {
            (None, Vec::new())
        };
        (compile, sim, coverage)
    }));
    let dur = started.elapsed().as_millis() as u64;
    match result {
        Ok((compile, sim, coverage)) => {
            let (sim_ok, sim_code) = match &sim {
                Some(s) => (s.ok as u8, s.code.clone()),
                None => (0u8, String::new()),
            };
            let code = if compile.ok {
                String::new()
            } else {
                compile.code.clone()
            };
            println!(
                "OK|{}|{}|{}|{}|{}|{}|{}",
                compile.ok as u8,
                code,
                sim_ok,
                sim_code,
                dur,
                coverage.len(),
                coverage.join(",")
            );
            std::process::exit(0);
        }
        Err(payload) => {
            println!("PANIC|{}", panic_string(payload).replace('|', "/"));
            std::process::exit(101);
        }
    }
}

/// Eksekusi subprocess (GAP-9): hang di-KILL sejati; SIGSEGV/abort (stack
/// overflow) terdeteksi via exit code non-zero (buta di jalur thread).
/// Opt-in `--proc-iso`; gagal spawn → fallback thread `run_isolated`.
pub fn run_isolated_proc(source: &str, max_time: u64, hang_ms: u64) -> RunOutcome {
    let Some(exe) = std::env::current_exe().ok() else {
        return run_isolated(source, max_time, hang_ms);
    };
    let mut child = match std::process::Command::new(&exe)
        .arg("--slave")
        .env("MARIA_FUZZ_SLAVE_MAX_TIME", max_time.to_string())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return run_isolated(source, max_time, hang_ms),
    };
    if let Some(mut si) = child.stdin.take() {
        use std::io::Write;
        let _ = si.write_all(source.as_bytes());
        // drop si → EOF bagi child.
    }
    let reader = child.stdout.take().map(|mut so| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut v: Vec<u8> = Vec::new();
            let _ = so.read_to_end(&mut v);
            v
        })
    });
    let deadline = Instant::now() + Duration::from_millis(hang_ms);
    let mut timed_out = false;
    loop {
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let out_buf = match reader {
        Some(h) => h.join().unwrap_or_default(),
        None => Vec::new(),
    };
    let exit = child.wait().ok();
    if timed_out {
        return RunOutcome {
            status: RunStatus::Hang,
            compile: CompileVerdict {
                ok: false,
                code: "HANG".to_string(),
                message: format!(">{} ms", hang_ms),
            },
            sim: None,
            coverage: Vec::new(),
            duration_ms: hang_ms,
        };
    }
    let line = String::from_utf8_lossy(&out_buf).lines().next().unwrap_or("").to_string();
    let mut parts = line.split('|');
    match parts.next() {
        Some("OK") => {
            let compile_ok = parts.next().unwrap_or("0") == "1";
            let compile_code = parts.next().unwrap_or("").to_string();
            let sim_ok = parts.next().unwrap_or("0") == "1";
            let sim_code = parts.next().unwrap_or("").to_string();
            let dur: u64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
            let _n_cov: usize = parts.next().unwrap_or("0").parse().unwrap_or(0);
            let covs: Vec<String> = parts
                .next()
                .unwrap_or("")
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            let sim = if compile_ok {
                Some(SimVerdict {
                    ok: sim_ok,
                    code: sim_code,
                    message: String::new(),
                    fingerprint: String::new(),
                    assertion: false,
                })
            } else {
                None
            };
            RunOutcome {
                status: RunStatus::Done,
                compile: CompileVerdict {
                    ok: compile_ok,
                    code: compile_code,
                    message: String::new(),
                },
                sim,
                coverage: covs,
                duration_ms: dur,
            }
        }
        Some("PANIC") => {
            let rest: Vec<&str> = parts.collect();
            RunOutcome {
                status: RunStatus::Panic(format!("procpanic: {}", rest.join("|"))),
                compile: CompileVerdict {
                    ok: false,
                    code: "PANIC".to_string(),
                    message: String::new(),
                },
                sim: None,
                coverage: Vec::new(),
                duration_ms: 0,
            }
        }
        _ => {
            // Exit non-zero tanpa baris hasil = SIGSEGV/abort/tak dikenal.
            let code = exit.and_then(|e| e.code()).unwrap_or(-1);
            if code == 0 {
                // Child keluar NORMAL tapi tanpa baris OK — anomali transien
                // (stdout race saat spawn massal). Jangan salah-klaim crash;
                // ulangi via jalur thread utk verdict yang benar.
                return run_isolated(source, max_time, hang_ms);
            }
            RunOutcome {
                status: RunStatus::Panic(format!("proc crashed (exit {:?})", code)),
                compile: CompileVerdict {
                    ok: false,
                    code: "PANIC".to_string(),
                    message: String::new(),
                },
                sim: None,
                coverage: Vec::new(),
                duration_ms: 0,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COUNTER: &str = r#"
module top(input logic clk, input logic rst_n,
           input logic [3:0] a, input logic [3:0] b,
           output logic [3:0] y);
  logic [3:0] r;
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) r <= '0;
    else r <= a + b;
  end
  assign y = r;
  initial begin clk = 0; forever #5 clk = ~clk; end
  initial begin rst_n = 0; a = 1; b = 2; #7 rst_n = 1; #3 a = 3; b = 4; end
endmodule
"#;

    #[test]
    fn valid_seed_done_and_sim_ok() {
        let out = run_isolated(COUNTER, 40, 3000);
        assert_eq!(out.status, RunStatus::Done);
        assert!(out.compile.ok);
        assert!(out.sim.as_ref().unwrap().ok);
    }

    #[test]
    fn garbage_seed_done_compile_err() {
        let out = run_isolated("module broken { not verilog", 40, 3000);
        assert_eq!(out.status, RunStatus::Done);
        assert!(!out.compile.ok);
        assert!(out.sim.is_none());
    }

    #[test]
    fn fingerprint_isolated_ok() {
        let f = fingerprint_isolated(COUNTER, 40, 3000);
        assert!(f.is_some());
        assert!(f.unwrap().contains("y="));
    }

    #[test]
    fn compile_only_no_panic_for_bad_input() {
        // Input rusak = compile err biasa, bukan panic → Ok dikembalikan.
        let r = compile_only_isolated("module broken {", 3000);
        assert!(r.is_ok());
    }
}