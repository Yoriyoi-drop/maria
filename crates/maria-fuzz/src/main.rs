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
  --emit-bugs DIR    tulis file bug terminimalkan ke direktori
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
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_usage();
        return;
    }

    let mut cfg = FuzzConfig {
        sim_sig_check: true,
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
            "--workers" => {
                i += 1;
                cfg.workers = parse_u64(&args[i], "--workers") as usize;
            }
            "--emit-bugs" => {
                i += 1;
                cfg.emit_dir = Some(PathBuf::from(&args[i]));
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
    if report.covered_features > 0 {
        println!("feature coverage: {} fitur tertutup", report.covered_features);
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