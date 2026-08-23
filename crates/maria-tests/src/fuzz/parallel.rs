//! Engine fuzz paralel — N worker (default 10) berjalan serentak.
//!
//! Satu file = satu tanggung jawab: orkestrasi paralel + agregasi statistik
//! + deteksi bottleneck. Logika generasi/mutasi ada di `gen`/`guide`,
//! verifikasi ada di `oracle`.
//!
//! Desain:
//! - Iterasi dibagi *strided*: worker `w` mengerjakan counter global
//!   `i = w, w+N, w+2N, …` sehingga urutan input tiap worker deterministik
//!   terlepas dari scheduling → assertion hasil stabil antar run.
//! - Tiap worker punya `CoverageGuide` lokal (feedback loop tetap jalan
//!   tanpa lock per iterasi). Coverage/corpus digabung setelah join.
//! - Bug dikumpulkan lokal lalu di-sort by seed → laporan deterministik.
//! - Bottleneck: testcase yang memakan waktu > `BOTTLENECK_THRESHOLD_US`
//!   dicatat sebagai temuan performa (bukan kegagalan keras).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use super::gen::GenInput;
use super::guide::CoverageGuide;
use super::oracle::{check, Verdict};

/// Jumlah worker default (multi jalur).
pub const DEFAULT_WORKERS: usize = 10;

/// Ambang bottleneck per testcase (mikrodetik). Lebih dari ini = temuan perf.
pub const BOTTLENECK_THRESHOLD_US: u128 = 1_000_000;

#[derive(Debug, Clone)]
pub struct ParallelConfig {
    /// Jumlah worker paralel (jalur).
    pub workers: usize,
    /// Total iterasi lintas semua worker.
    pub iterations: u64,
}

impl Default for ParallelConfig {
    fn default() -> Self {
        ParallelConfig {
            workers: DEFAULT_WORKERS,
            iterations: 300,
        }
    }
}

/// Satu kasus bottleneck (testcase lambat).
#[derive(Debug, Clone)]
pub struct SlowCase {
    pub seed: u64,
    pub micros: u128,
}

/// Satu bug yang ditemukan fuzzer.
#[derive(Debug, Clone)]
pub struct BugCase {
    pub input: GenInput,
    pub message: String,
}

/// Statistik gabungan seluruh worker.
#[derive(Debug, Clone, Default)]
pub struct ParallelStats {
    pub iterations: u64,
    pub passed: u64,
    pub compile_failures: u64,
    pub bugs_found: usize,
    /// Fitur coverage maksimum antar worker (aproksimasi union).
    pub coverage_features: usize,
    /// Total corpus antar worker.
    pub corpus_size: usize,
    pub elapsed_ms: u64,
    /// Testcase lambat (bottleneck pipeline), terurut menurun.
    pub bottlenecks: Vec<SlowCase>,
}

struct WorkerResult {
    passed: u64,
    compile_failures: u64,
    bugs: Vec<BugCase>,
    slow: Vec<SlowCase>,
    coverage_features: usize,
    corpus_size: usize,
}

fn run_worker(worker_id: usize, workers: usize, iterations: u64, done: &Arc<AtomicU64>) -> WorkerResult {
    let mut guide = CoverageGuide::new();
    let mut bugs = Vec::new();
    let mut slow = Vec::new();
    let mut passed = 0u64;
    let mut compile_failures = 0u64;

    // Strided: worker w mulai dari index w, melangkah sebesar jumlah worker.
    let mut i = worker_id as u64;
    while i < iterations {
        let input = guide.next(i);

        let t0 = Instant::now();
        let result = check(&input);
        let us = t0.elapsed().as_micros();
        if us > BOTTLENECK_THRESHOLD_US {
            slow.push(SlowCase { seed: input.seed, micros: us });
        }

        match &result.verdict {
            Verdict::Bug(m) => bugs.push(BugCase {
                input: input.clone(),
                message: m.clone(),
            }),
            Verdict::Pass => passed += 1,
            Verdict::CompileFail => compile_failures += 1,
        }

        guide.observe(&input, result.compiled);

        i += workers as u64;
        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
        if n % 50 == 0 {
            eprintln!("[fuzz-par] {}/{} selesai", n, iterations);
        }
    }

    WorkerResult {
        passed,
        compile_failures,
        bugs,
        slow,
        coverage_features: guide.coverage_len(),
        corpus_size: guide.corpus_len(),
    }
}

/// Jalankan fuzzing paralel. Kembalikan (statistik gabungan, daftar bug
/// terurut by seed — deterministik untuk assertion).
pub fn run_parallel(cfg: ParallelConfig) -> (ParallelStats, Vec<BugCase>) {
    let start = Instant::now();
    let workers = cfg.workers.max(1);
    let done = Arc::new(AtomicU64::new(0));

    let handles: Vec<_> = (0..workers)
        .map(|w| {
            let done = Arc::clone(&done);
            let iterations = cfg.iterations;
            std::thread::Builder::new()
                .name(format!("fuzz-worker-{}", w))
                .spawn(move || run_worker(w, workers, iterations, &done))
                .expect("spawn fuzz worker")
        })
        .collect();

    let mut stats = ParallelStats {
        iterations: cfg.iterations,
        ..Default::default()
    };
    let mut all_bugs: Vec<BugCase> = Vec::new();

    for h in handles {
        let r = h.join().expect("fuzz worker panic");
        stats.passed += r.passed;
        stats.compile_failures += r.compile_failures;
        stats.coverage_features = stats.coverage_features.max(r.coverage_features);
        stats.corpus_size += r.corpus_size;
        stats.bottlenecks.extend(r.slow);
        all_bugs.extend(r.bugs);
    }

    // Deterministik: urutkan bug & bottleneck by seed.
    all_bugs.sort_by_key(|b| b.input.seed);
    stats.bottlenecks.sort_by(|a, b| b.micros.cmp(&a.micros));
    stats.bugs_found = all_bugs.len();
    stats.elapsed_ms = start.elapsed().as_millis() as u64;

    (stats, all_bugs)
}
