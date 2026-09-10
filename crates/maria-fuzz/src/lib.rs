//! maria-fuzz — fuzzer internal (dev-only), bukan CLI user.
//!
//! Pendekatan: fuzz berbasis **corpus project nyata** (cva6, openc910,
//! opentitan). Semua seed dari RTL asli — TANPA template sintetis.
//! Kampanye: ambil seed nyata → mutasi 0-4x → evaluasi oracle.

pub mod corpus;
pub mod minimize;
pub mod mutator;
pub mod oracle;
pub mod runner;
pub mod triage;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

pub const VERSION: &str = "0.1.0";

/// Target pipeline stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Target {
    #[default]
    All,
    Lexer,
    Parser,
    Elaborator,
    Simulator,
    Fmt,
    Cli,
}

impl Target {
    pub const ALL: &[Target] = &[
        Target::Lexer,
        Target::Parser,
        Target::Elaborator,
        Target::Simulator,
        Target::Fmt,
        Target::Cli,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Target::All => "all",
            Target::Lexer => "lexer",
            Target::Parser => "parser",
            Target::Elaborator => "elab",
            Target::Simulator => "sim",
            Target::Fmt => "fmt",
            Target::Cli => "cli",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "all" => Some(Target::All),
            "lexer" | "lex" => Some(Target::Lexer),
            "parser" | "parse" => Some(Target::Parser),
            "elab" | "elaborator" => Some(Target::Elaborator),
            "sim" | "simulator" | "run" => Some(Target::Simulator),
            "fmt" => Some(Target::Fmt),
            "cli" => Some(Target::Cli),
            _ => None,
        }
    }
}

/// Kategori hasil kasus fuzz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    Ok,
    /// Error diagnostik yang valid (bukan bug).
    CleanError,
    // === Bug categories ===
    /// Crash: panic, abort, segfault.
    Panic,
    /// Process exit non-zero tanpa panic (kecuali clean error).
    Abort,
    /// Hang: timeout tanpa selesai.
    Hang,
    /// Diagnostik hilang: error seharusnya ada tapi tidak muncul.
    DiagMissing,
    /// Round-trip mismatch: fmt(fmt(s)) != fmt(s).
    RoundtripMismatch,
    /// Hasil berbeda antar run (non-deterministic).
    NonDeterministic,
    /// Guard bypass: assertion/safety property dilanggar.
    GuardBypass,
    /// Differential: hasil beda antar engine path.
    Differential,
}

impl Category {
    pub fn is_bug(self) -> bool {
        !matches!(self, Category::Ok | Category::CleanError)
    }

    pub fn label(self) -> &'static str {
        match self {
            Category::Ok => "ok",
            Category::CleanError => "clean_error",
            Category::Panic => "panic",
            Category::Abort => "abort",
            Category::Hang => "hang",
            Category::DiagMissing => "diag_missing",
            Category::RoundtripMismatch => "roundtrip_mismatch",
            Category::NonDeterministic => "nondeterministic",
            Category::GuardBypass => "guard_bypass",
            Category::Differential => "differential",
        }
    }

    pub fn from_label(s: &str) -> Option<Self> {
        match s {
            "ok" => Some(Category::Ok),
            "clean_error" => Some(Category::CleanError),
            "panic" => Some(Category::Panic),
            "abort" => Some(Category::Abort),
            "hang" => Some(Category::Hang),
            "diag_missing" => Some(Category::DiagMissing),
            "roundtrip_mismatch" => Some(Category::RoundtripMismatch),
            "nondeterministic" => Some(Category::NonDeterministic),
            "guard_bypass" => Some(Category::GuardBypass),
            "differential" => Some(Category::Differential),
            _ => None,
        }
    }
}

/// Oracle identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Oracle {
    /// Tidak ada crash/panic/abort/hang.
    O1NoCrash,
    /// Diagnostik punya lokasi file:line:col.
    O2DiagLocation,
    /// Round-trip: fmt(fmt(s)) == fmt(s).
    O3Roundtrip,
    /// Determinism: run dua kali hasil sama.
    O4Determinism,
    /// Differential: hasil beda antar engine path.
    O5Differential,
}

impl Oracle {
    pub fn as_str(self) -> &'static str {
        match self {
            Oracle::O1NoCrash => "O1-no-crash",
            Oracle::O2DiagLocation => "O2-diag-location",
            Oracle::O3Roundtrip => "O3-roundtrip",
            Oracle::O4Determinism => "O4-determinism",
            Oracle::O5Differential => "O5-differential",
        }
    }
}

/// Hasil satu kasus fuzz.
#[derive(Debug, Clone)]
pub struct CaseResult {
    pub target: Target,
    pub category: Category,
    pub oracle: &'static str,
    pub detail: String,
    pub source: String,
}

impl CaseResult {
    /// Signature untuk dedup (target|oracle|category|first-detail-line).
    pub fn signature(&self) -> String {
        let first_line = self.detail.lines().next().unwrap_or("");
        format!(
            "{}|{}|{}|{}",
            self.target.as_str(),
            self.oracle,
            self.category.label(),
            first_line
        )
    }
}

/// Konfigurasi kampanye fuzz.
#[derive(Debug, Clone)]
pub struct FuzzConfig {
    pub target: Target,
    pub cases: usize,
    pub seed: u64,
    pub timeout_ms: u64,
    pub corpus_dir: Option<PathBuf>,
    pub save_bugs: bool,
}

impl Default for FuzzConfig {
    fn default() -> Self {
        Self {
            target: Target::All,
            cases: 2000,
            seed: 0xC0FFEE,
            // Kasus in-process (lexer/parser/elab/fmt) < 500ms ideal.
            // Subprocess sim butuh waktu cold-start binary + compile real RTL:
            // 2000ms = hang asli (infinite loop), bukan compile lambat.
            timeout_ms: 2000,
            corpus_dir: None,
            save_bugs: true,
        }
    }
}

/// Ringkasan per target.
#[derive(Debug, Default, Clone)]
pub struct TargetSummary {
    pub target: Target,
    pub total: usize,
    pub bugs: usize,
    pub categories: std::collections::BTreeMap<Category, usize>,
}

/// Laporan kampanye fuzz.
#[derive(Debug, Default, Clone)]
pub struct FuzzReport {
    pub bugs: Vec<CaseResult>,
    pub summaries: Vec<TargetSummary>,
}

/// Deterministic PRNG (splitmix64, 16-iter warmup seperti mvm-fuzz).
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut rng = Self { state: seed };
        // 16-iter warmup
        for _ in 0..16 {
            rng.next_u64();
        }
        rng
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    /// Return value in [0, bound).
    pub fn below(&mut self, bound: usize) -> usize {
        (self.next_u64() as usize) % bound
    }

    /// Probability in [0, 100).
    pub fn chance(&mut self, pct: usize) -> bool {
        self.below(100) < pct
    }

    /// Pick random element.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// i64 in range.
    pub fn i64_in(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next_u64() as i64 % (hi - lo + 1).max(1))
    }
}

/// Counter global crash sequence.
static CRASH_SEQ: AtomicU32 = AtomicU32::new(0);

pub fn next_crash_seq() -> u32 {
    CRASH_SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Jalur fuzz default.
pub fn bugs_dir() -> PathBuf {
    PathBuf::from(".maria-fuzz-bugs")
}

/// Jalur corpus default.
pub fn default_corpus_dir() -> PathBuf {
    PathBuf::from("crates/maria-fuzz/fuzz/corpus/seeds")
}

/// Jalur reports.
pub fn reports_dir() -> PathBuf {
    PathBuf::from("fuzz/reports")
}

/// Jalur bug database.
pub fn bugdb_path() -> PathBuf {
    PathBuf::from(".maria-fuzz-bugdb.json")
}

/// Run kampanye fuzz.
pub fn run(cfg: FuzzConfig) -> FuzzReport {
    match cfg.target {
        Target::All => {
            let mut merged = FuzzReport::default();
            for &target in Target::ALL {
                let mut single_cfg = cfg.clone();
                single_cfg.target = target;
                let report = run_single(single_cfg);
                merged.bugs.extend(report.bugs);
                merged.summaries.extend(report.summaries);
            }
            merged
        }
        _ => run_single(cfg),
    }
}

fn run_single(cfg: FuzzConfig) -> FuzzReport {
    use std::collections::BTreeMap;

    let corpus = corpus::Corpus::load(cfg.corpus_dir.as_deref());
    if corpus.is_empty() {
        eprintln!(
            "WARNING: corpus kosong di {:?} — tidak ada seed nyata",
            cfg.corpus_dir
                .clone()
                .unwrap_or_else(default_corpus_dir)
        );
    }

    let mut rng = Rng::new(cfg.seed);

    let mut results: Vec<CaseResult> = Vec::new();
    let mut categories: BTreeMap<Category, usize> = BTreeMap::new();
    let mut seen_sigs: std::collections::HashSet<String> = std::collections::HashSet::new();

    for i in 0..cfg.cases {
        // Ambil seed nyata dari corpus (100% — tanpa template sintetis)
        let base_source = if corpus.is_empty() {
            break;
        } else {
            corpus.random_seed(&mut rng).unwrap_or_default()
        };

        // Mutasi 0-4x (mutator dibuat per-iterasi agar borrow rng singkat)
        let n_mut = rng.below(5);
        let mut source = base_source.clone();
        {
            let mut mutator = mutator::Mutator::new(&mut rng);
            for _ in 0..n_mut {
                source = mutator.mutate(&source, &corpus);
            }
        }

        // Env hook: tulis source pre-eval untuk repro
        if let Ok(trace_path) = std::env::var("MARIA_FUZZ_TRACE") {
            let _ = std::fs::write(&trace_path, &source);
        }

        let result = oracle::evaluate(cfg.target, &source, cfg.timeout_ms);
        *categories.entry(result.category).or_insert(0) += 1;

        if result.category.is_bug() {
            let sig = result.signature();
            if !seen_sigs.contains(&sig) {
                seen_sigs.insert(sig);
                if cfg.save_bugs {
                    save_crash(&result, i);
                }
                results.push(result);
            }
        }

        if (i + 1) % 500 == 0 {
            eprint!("\r  [{}/{}] bugs: {}", i + 1, cfg.cases, results.len());
        }
    }
    if cfg.cases >= 500 {
        eprintln!();
    }

    let summary = TargetSummary {
        target: cfg.target,
        total: cfg.cases,
        bugs: results.len(),
        categories,
    };

    FuzzReport {
        bugs: results,
        summaries: vec![summary],
    }
}

/// Simpan crash ke .maria-fuzz-bugs/
fn save_crash(result: &CaseResult, iter: usize) {
    let dir = bugs_dir();
    let _ = std::fs::create_dir_all(&dir);

    let seq = next_crash_seq();
    let kind = result.category.label();
    let filename = format!("bug_{:04}_{}.sv", seq, kind);
    let path = dir.join(&filename);
    let _ = std::fs::write(&path, &result.source);

    // Metadata .txt
    let meta_path = dir.join(format!("bug_{:04}_{}.txt", seq, kind));
    let meta = format!(
        "oracle: {}\ncategory: {}\nsignature: {}\ndetail: {}\niter: {}\n",
        result.oracle,
        result.category.label(),
        result.signature(),
        result.detail.lines().next().unwrap_or(""),
        iter,
    );
    let _ = std::fs::write(&meta_path, meta);
}