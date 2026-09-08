//! Bin maria-fuzz — CLI internal pengembang (dev-only).
//!
//! `cargo run -p maria-fuzz --features dev -- [opsi]`
//! Lihat `doc/fuzzing.md` §7 untuk penggunaan dan env var.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use maria_fuzz::{FuzzConfig, FuzzReport, run_fuzz, corpus};


fn print_usage() {
    eprintln!(
        "maria-fuzz (dev-only) — fuzzer internal maria. Landasan doc/fuzzing.md (20 paper).

USAGE:
  cargo run -p maria-fuzz --features dev -- [OPTIONS]

OPTIONS:
  --iters N          Jumlah iterasi (env MARIA_FUZZ_N, default 300)
  --seed N           Seed RNG (default 0x6d61726961)
  --max-time T       max_time simulasi (default 100)
  --hang-ms M        ambang hang per eksekusi (default 12000; harus > settle
                     delta-storm engine ~10s debug, agar delta-storm yang
                     engine-settle tidak salah-klaim Hang)
  --corpus-dir DIR   direktori corpus SV nyata (bisa diulang)
  --target FEATURE   fuzzing terarah (mis. >>, case, $clog2)
  --workers W        kampanye paralel (env MARIA_FUZZ_WORKERS, default 1)
  --proc-iso         eksekusi dgn SUBPROCESS (GAP-9): hang di-kill sejati,
                     stack-overflow (SIGSEGV) terdeteksi via exit code;
                     biaya spawn ~ms — default thread untuk kecepatan
  --sweep N          validasi oracle: sweep N seed generated -> FP per oracle
                     + observability fault op-swap (GAP-5 eksperimen)
  --emit-bugs DIR    tulis file bug terminimalkan ke direktori
  --save-corpus DIR  persist seed menarik (parent/bug) utk kampanye berikut
  --sim-sig-check    aktifkan oracle nilai sinyal (3× pipeline/iterasi; OFF default)
  --verbose          progress tiap 100 iterasi ke stderr
  -h, --help         bantuan ini
"
    );
}

fn parse_u64(v: &str, name: &str) -> u64 {
    v.parse().unwrap_or_else(|_| {
        eprintln!("error: {} harus angka, dapat '{}'", name, v);
        std::process::exit(2);
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Mode slave (GAP-9): dipanggil run_isolated_proc — baca source dari
    // stdin, compile+sim, cetak hasil, exit. Intercept SEBELUM parse cfg.
    if args.iter().any(|a| a == "--slave") {
        maria_fuzz::harness::run_slave();
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_usage();
        return;
    }

    // sim_sig_check OFF default (audit H5): biaya 3× pipeline per iterasi
    // (~4× total eksekusi), padahal determinism oracle sudah menutupi sebagian.
    // Aktifkan eksplisit lewat --sim-sig-check untuk eksperimen oracle nilai.
    let mut cfg = FuzzConfig {
        sim_sig_check: false,
        ..FuzzConfig::default()
    };
    let mut i = 0usize;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "--iters" => {
                i += 1;
                cfg.iters = parse_u64(&args[i], "--iters");
            }
            "--seed" => {
                i += 1;
                cfg.seed = parse_u64(&args[i], "--seed");
            }
            "--max-time" => {
                i += 1;
                cfg.max_time = parse_u64(&args[i], "--max-time");
            }
            "--hang-ms" => {
                i += 1;
                cfg.hang_ms = parse_u64(&args[i], "--hang-ms");
            }
            "--corpus-dir" => {
                i += 1;
                cfg.corpus_dirs.push(PathBuf::from(&args[i]));
            }
            "--target" => {
                i += 1;
                cfg.target = Some(args[i].clone());
            }
            "--proc-iso" => {
                cfg.proc_isolate = true;
            }
            "--workers" => {
                i += 1;
                cfg.workers = parse_u64(&args[i], "--workers") as usize;
            }
            "--sweep" => {
                i += 1;
                let n = parse_u64(&args[i], "--sweep") as usize;
                let res = maria_fuzz::faults::sweep(n, &cfg);
                println!("=== oracle validation sweep ({} seeds) ===", res.seeds_total);
                println!(
                    "clean={} fp_det={} fp_emi={} fp_meta={} faults_observable={}/{} | trace={}/{}",
                    res.seeds_clean,
                    res.fp_determinism,
                    res.fp_emi,
                    res.fp_meta,
                    res.fault_observable,
                    res.fault_total,
                    res.fault_observable_trace,
                    res.fault_total
                );
                std::process::exit(0);
            }
            "--emit-bugs" => {
                i += 1;
                cfg.emit_dir = Some(PathBuf::from(&args[i]));
            }
            "--sim-sig-check" => {
                cfg.sim_sig_check = true;
            }
            "--save-corpus" => {
                i += 1;
                cfg.save_corpus_dir = Some(PathBuf::from(&args[i]));
            }
            "--verbose" => {
                cfg.verbose = true;
            }
            other if other.starts_with("--") => {
                eprintln!("error: opsi tak dikenal '{}'", other);
                print_usage();
                std::process::exit(2);
            }
            other => {
                eprintln!("error: argumen posisional tak dikenal '{}'", other);
                print_usage();
                std::process::exit(2);
            }
        }
        i += 1;
    }

    // Env override (documented di doc/fuzzing.md).
    if let Ok(v) = std::env::var("MARIA_FUZZ_N") {
        if let Ok(n) = v.parse() {
            cfg.iters = n;
        }
    }
    if let Ok(v) = std::env::var("MARIA_FUZZ_WORKERS") {
        if let Ok(w) = v.parse() {
            cfg.workers = w;
        }
    }

    eprintln!(
        "maria-fuzz: iters={} seed={:#x} max_time={} hang_ms={} workers={} target={:?} corpus={:?}",
        cfg.iters, cfg.seed, cfg.max_time, cfg.hang_ms, cfg.workers, cfg.target, cfg.corpus_dirs
    );

    let started = Instant::now();
    let report = if cfg.workers <= 1 {
        run_fuzz(&cfg)
    } else {
        // Kampanye paralel: tiap worker seed berbeda (Paper #14 scale).
        // Corpus seed bersama (Paper #12): load sekali, bagi ke semua worker
        // supaya semua worker putar dari corpus yang sama.
        let shared_corpus = corpus::Corpus::from_dirs(&cfg.corpus_dirs);
        let cfg = Arc::new(cfg);
        let shared_corpus = Arc::new(shared_corpus);
        let handles: Vec<std::thread::JoinHandle<FuzzReport>> = (0..cfg.workers)
            .map(|w| {
                let cfg = Arc::clone(&cfg);
                let corpus = Arc::clone(&shared_corpus);
                std::thread::Builder::new()
                    .name(format!("fuzz-worker-{}", w))
                    .stack_size(maria_fuzz::harness::WORKER_STACK_BYTES)
                    .spawn(move || {
                        let mut c = (*cfg).clone();
                        c.seed = c.seed.wrapping_add(w as u64 * 0x9E37_79B9);
                        c.workers = 1;
                        // Per-worker corpus dir — hindari race tulis seed_N.sv.
                        c.save_corpus_dir = c.save_corpus_dir.map(|d| d.join(format!("w{}", w)));
                        // Inject corpus bersama ke worker ini.
                        c.corpus = Some((*corpus).clone());
                        run_fuzz(&c)
                    })
                    .expect("spawn fuzz worker")
            })
            .collect();
        let mut merged = FuzzReport::default();
        for h in handles {
            if let Ok(r) = h.join() {
                merged.merge(&r);
            }
        }
        merged
    };

    let elapsed = started.elapsed();
    // Ringkasan akhir.
    println!("=== maria-fuzz done in {} ms ===", elapsed.as_millis());
    println!("{}", report.summary());
    if let Some(cdg) = &report.cdg_info {
        println!(
            "CDG progress (Paper #20): {}/{} target hit ({:.1}%)",
            cdg.targets_hit,
            cdg.targets_total,
            cdg.ratio * 100.0
        );
        if !report.unreached_targets.is_empty() {
            let unreached_preview: Vec<&str> = report
                .unreached_targets
                .iter()
                .take(8)
                .map(|s| s.as_str())
                .collect();
            println!(
                "unreached targets ({} total, showing {}): {}",
                report.unreached_targets.len(),
                unreached_preview.len(),
                unreached_preview.join(", ")
            );
        }
    } else if report.covered_features > 0 {
        println!("feature coverage: {} fitur tertutup", report.covered_features);
    }
    // Statistik palet mutasi (GAP-3): op teratas menurut novelty.
    let mut ops: Vec<(usize, u64, u64)> = (0..maria_fuzz::ast_mutate::NUM_OPS)
        .map(|i| (i, report.op_stats.novels[i], report.op_stats.attempts[i]))
        .collect();
    ops.sort_by(|a, b| b.1.cmp(&a.1));
    println!("mutation ops (op: novel/attempt):");
    for (i, n, a) in ops.iter().take(6) {
        println!("  op {}: {}/{}", i, n, a);
    }
    for (idx, b) in report.bugs.iter().enumerate() {
        println!("--- bug #{} [{:?}] ---", idx, b.kind);
        println!("{}", b.detail);
        println!("source ({} bytes):\n{}", b.source.len(), b.source);
    }

    // Exit code: 1 bila ada bug ditemukan (utk CI internal dev).
    if report.bugs.is_empty() {
        std::process::exit(0);
    } else {
        std::process::exit(1);
    }
}