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