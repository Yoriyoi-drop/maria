//! Differential testing konkurensi — hasil single-thread vs multi-thread.
//!
//! Satu file = satu tanggung jawab: membandingkan output simulasi pada
//! 1/2/4/8 thread paralel terhadap baseline sekuensial. Perbedaan hasil,
//! panic, atau hang = temuan high-priority (race condition).
//!
//! Latar: P0 lama — VPI engine global menimpa pointer antar thread
//! (SIGSEGV saat test paralel). Test ini regression guard untuk kelas bug
//! yang sama di seluruh pipeline.

use super::gen::generate;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Simulasi paralel N worker atas seed berbeda; kembalikan nilai `y` per
/// seed (None = sim error). Panic ditangkap per testcase; worker panic
/// keras = kegagalan test.
fn parallel_sim(workers: usize, seeds: &[u64]) -> Vec<Option<u64>> {
    let seeds: Arc<Vec<u64>> = Arc::new(seeds.to_vec());
    let results: Vec<Option<Option<u64>>> = seeds.iter().map(|_| None).collect();
    let results = Arc::new(std::sync::Mutex::new(results));
    let failures = Arc::new(AtomicUsize::new(0));
    let next = Arc::new(AtomicUsize::new(0));

    let handles: Vec<_> = (0..workers)
        .map(|w| {
            let seeds = Arc::clone(&seeds);
            let results = Arc::clone(&results);
            let failures = Arc::clone(&failures);
            let next = Arc::clone(&next);
            std::thread::Builder::new()
                .name(format!("conc-diff-{}", w))
                .spawn(move || loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    if i >= seeds.len() {
                        break;
                    }
                    let input = generate(seeds[i]);
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        crate::simulate_signals(&input.to_source(), 20).ok()
                    }));
                    match r {
                        Ok(sigs_res) => {
                            // Sim error = None luar (konsisten dengan baseline);
                            // yang penting hasil paralel == hasil sekuensial.
                            let y = sigs_res.as_ref().and_then(|sigs| {
                                sigs.iter().find(|(n, _)| n == "y").map(|(_, v)| v.to_u64())
                            });
                            results.lock().unwrap()[i] = Some(y);
                        }
                        Err(p) => {
                            let msg = p
                                .downcast_ref::<String>()
                                .cloned()
                                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                                .unwrap_or_default();
                            eprintln!(
                                "[conc-diff] PANIC seed={} msg={} source:\n{}",
                                input.seed,
                                msg,
                                input.to_source()
                            );
                            failures.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                })
                .expect("spawn conc-diff worker")
        })
        .collect();
    for h in handles {
        h.join().expect("worker panic (bukan catch_unwind)");
    }
    assert_eq!(failures.load(Ordering::SeqCst), 0, "ada PANIC saat sim paralel");
    let out: Vec<Option<u64>> = results
        .lock()
        .unwrap()
        .iter()
        .map(|x| x.unwrap_or(None))
        .collect();
    out
}

#[test]
fn concurrency_differential_1_vs_n_threads() {
    // Baseline sekuensial.
    let seeds: Vec<u64> = (0..40).map(|i| i * 104_729 + 11).collect();
    let baseline: Vec<Option<u64>> = seeds
        .iter()
        .map(|s| {
            let input = generate(*s);
            crate::simulate_signals(&input.to_source(), 20)
                .ok()
                .and_then(|sigs| sigs.iter().find(|(n, _)| n == "y").map(|(_, v)| v.to_u64()))
        })
        .collect();

    for workers in [2usize, 4, 8] {
        let par = parallel_sim(workers, &seeds);
        for (i, (b, p)) in baseline.iter().zip(par.iter()).enumerate() {
            assert_eq!(
                b, p,
                "workers={} seed#{} ({}) hasil beda dari baseline → race",
                workers, i, seeds[i]
            );
        }
        eprintln!(
            "[conc-diff] workers={} {} kasus identik dengan baseline",
            workers,
            seeds.len()
        );
    }
}

#[test]
fn concurrency_compile_parallel_no_crash() {
    // Compile paralel dari banyak thread pada source beragam —
    // regression guard jalur MICD/symbol intern lintas thread.
    let sources = [
        "module c0; wire w = 1'b0; endmodule",
        "package cp; parameter P = 4; endpackage\nmodule c1; import cp::*; wire [P-1:0] w; endmodule",
        "module c2; reg [7:0] r; always @(posedge dummy) r <= r+1; endmodule",
        "`define D 5\nmodule c3; wire [`D-1:0] w; endmodule",
        "module c4; class k; int x; endclass k inst; endmodule",
    ];
    let n = 8usize;
    let per_thread = 25;
    let handles: Vec<_> = (0..n)
        .map(|w| {
            std::thread::Builder::new()
                .name(format!("conc-cmp-{}", w))
                .spawn(move || {
                    for it in 0..per_thread {
                        let src = sources[(w + it) % sources.len()];
                        // Ok/Err bebas — yang penting tidak panic/hang.
                        let _ = crate::compile_str(src);
                    }
                })
                .expect("spawn")
        })
        .collect();
    for h in handles {
        h.join().expect("compile thread panic");
    }
}
