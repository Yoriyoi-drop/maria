//! maria-fuzz — fuzzer internal maria (dev-only).
//!
//! Landasan ilmiah: `doc/fuzzing.md` — 20 jurnal/paper dalam 6 pilar.
//! Konvensi penomoran `Paper #N` (1..20) dipakai di komentar seluruh kode.
//!
//! File ini = 1 tanggung jawab: **orkestrasi** loop fuzzer. Seluruh logika
//! spesifik tinggal di modul masing-masing.
//!
//! Aktivasi: `cargo run -p maria-fuzz --features dev` (bin `maria-fuzz`)
//! atau `cargo test -p maria-fuzz --features dev` (test suite).

#![cfg(feature = "dev")]

pub mod ast_mutate;
pub mod cdg;
pub mod corpus;
pub mod differential;
pub mod directed;
pub mod feature;
pub mod gen;
pub mod grammar;
pub mod guide;
pub mod harness;
pub mod oracle;

use std::path::PathBuf;
use std::time::Instant;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use gen::Generator;
use harness::RunStatus;

/// Konfigurasi satu kampanye fuzzing.
#[derive(Debug, Clone)]
pub struct FuzzConfig {
    /// Jumlah iterasi fuzz (tiap iterasi = 1+ eksekusi compile/simulasi).
    pub iters: u64,
    /// Seed RNG — replikasi deterministik.
    pub seed: u64,
    /// `max_time` simulasi (siklus).
    pub max_time: u64,
    /// Ambang hang per eksekusi (ms). Lewat = `Hang` + thread di-leak.
    pub hang_ms: u64,
    /// Direktori corpus seed SV nyata (Paper #12) — opsional.
    pub corpus_dirs: Vec<PathBuf>,
    /// Fitur target fuzzing terarah (Paper #17) — opsional, mis. ">>".
    pub target: Option<String>,
    /// Jumlah kampanye paralel (MARIA_FUZZ_WORKERS). Tiap kampanye punya
    /// seed sendiri; report digabung.
    pub workers: usize,
    /// Directori tujuan file bug terminimalkan (opsional; default tidak menulis).
    pub emit_dir: Option<PathBuf>,
    /// Cetak progress tiap `verbose_every` iterasi ke stderr.
    pub verbose: bool,
    pub verbose_every: u64,
}

impl Default for FuzzConfig {
    fn default() -> Self {
        FuzzConfig {
            iters: 300,
            seed: 0x6d61_7269_61,
            max_time: 100,
            hang_ms: 3000,
            corpus_dirs: Vec::new(),
            target: None,
            workers: 1,
            emit_dir: None,
            verbose: false,
            verbose_every: 100,
        }
    }
}

/// Klasifikasi bug yang ditemukan fuzzer.
#[derive(Debug, Clone, PartialEq)]
pub enum BugKind {
    /// Panic (catch_unwind) saat compile/simulasi — crash di pipeline maria.
    Panic,
    /// Eksekusi melewati ambang waktu → dicurigai infinite loop/regresi.
    Hang,
    /// Hasil simulasi menyimpang dari oracle differential (Paper #13/#19).
    Differential,
}

#[derive(Debug, Clone)]
pub struct BugRecord {
    pub kind: BugKind,
    /// Source (sudah terminimalkan untuk Panic/Differential).
    pub source: String,
    pub detail: String,
}

/// Laporan satu kampanye fuzzing.
#[derive(Debug, Clone, Default)]
pub struct FuzzReport {
    pub total: u64,
    pub compile_ok: u64,
    pub compile_err: u64,
    pub sim_ok: u64,
    pub sim_err: u64,
    pub panics: u64,
    pub hangs: u64,
    pub new_features: u64,
    pub determinism_mismatch: u64,
    pub emi_mismatch: u64,
    pub covered_features: usize,
    pub bugs: Vec<BugRecord>,
}

impl FuzzReport {
    pub fn merge(&mut self, other: &FuzzReport) {
        self.total += other.total;
        self.compile_ok += other.compile_ok;
        self.compile_err += other.compile_err;
        self.sim_ok += other.sim_ok;
        self.sim_err += other.sim_err;
        self.panics += other.panics;
        self.hangs += other.hangs;
        self.new_features += other.new_features;
        self.determinism_mismatch += other.determinism_mismatch;
        self.emi_mismatch += other.emi_mismatch;
        self.covered_features = self.covered_features.max(other.covered_features);
        self.bugs.extend(other.bugs.clone());
    }

    /// Ringkasan satu-baris untuk konsole / merge.
    pub fn summary(&self) -> String {
        format!(
            "iters={} compile_ok={} compile_err={} sim_ok={} sim_err={} panics={} hangs={} \
             new_features={} det_mismatch={} emi_mismatch={} covered={} bugs={}",
            self.total,
            self.compile_ok,
            self.compile_err,
            self.sim_ok,
            self.sim_err,
            self.panics,
            self.hangs,
            self.new_features,
            self.determinism_mismatch,
            self.emi_mismatch,
            self.covered_features,
            self.bugs.len(),
        )
    }
}

/// Jalankan satu kampanye fuzzing (deterministik utk seed diberikan).
///
/// Loop (Paper #2/#3 taxonomy):
/// 1. pilih seed parent dari corpus via energy schedule (#4/#5/#8)
/// 2. mutasi 1..3x (grammar #9/#11, AST #10, fragment #12)
/// 3. bias ke target jika ada (#17)
/// 4. eksekusi terisolasi + oracle (#1/#2/#3)
/// 5. update feature map (#6/#7) + adaptasi energy (#8)
/// 6. differential sampling (#13/#19) + CDG (#20)
pub fn run_fuzz(cfg: &FuzzConfig) -> FuzzReport {
    let mut rng = StdRng::seed_from_u64(cfg.seed);
    let gen = Generator::new(cfg.seed);
    let mut guide = guide::CoverageGuide::new(cfg.seed);
    let corpus = corpus::Corpus::from_dirs(&cfg.corpus_dirs);
    let mut report = FuzzReport::default();
    let started = Instant::now();

    // ── Seed awal: modul generated (Paper #14/#15) + fragment corpus (#12) ──
    for _ in 0..16 {
        let s = gen.random_module(&mut rng);
        let feats = feature::FeatureMap::extract(&s);
        guide.add(s, feats, false);
    }
    for frag in corpus.sample_batch(&mut rng, 8) {
        let feats = feature::FeatureMap::extract(&frag);
        guide.add(frag, feats, false);
    }

    for iter in 0..cfg.iters {
        // ── 1. pilih parent (energy schedule) ──
        let Some(parent) = guide.select() else { break };

        // ── 2. rantai mutasi ──
        let chain = rng.gen_range(1..=3);
        let mut src = parent.clone();
        for _ in 0..chain {
            src = ast_mutate::mutate(&mut rng, &src, &corpus);
        }

        // ── 3. bias terarah (#17) ──
        if let Some(t) = &cfg.target {
            if !directed::is_relevant(&src, t) {
                if let Some(b) = directed::bias_seed(&src, t) {
                    src = b;
                }
            }
        }

        // ── 4. eksekusi terisolasi + oracle ──
        let out = harness::run_isolated(&src, cfg.max_time, cfg.hang_ms);
        report.total += 1;

        let mut feats = feature::FeatureMap::extract(&src);
        match &out.status {
            RunStatus::Panic(p) => {
                report.panics += 1;
                let mut detail = format!("panic: {}", p);
                let minimized = corpus::Corpus::minimize(&src, &mut |cand| {
                    matches!(
                        harness::compile_only_isolated(cand, cfg.hang_ms),
                        Err(_) // masih panic
                    ) || cand.is_empty()
                });
                detail.push_str(&format!("\nminimized {} -> {} bytes", src.len(), minimized.len()));
                report.bugs.push(BugRecord {
                    kind: BugKind::Panic,
                    source: minimized,
                    detail,
                });
            }
            RunStatus::Hang => {
                report.hangs += 1;
                report.bugs.push(BugRecord {
                    kind: BugKind::Hang,
                    source: src.clone(),
                    detail: format!("hang > {} ms ({})", cfg.hang_ms, out.duration_ms),
                });
            }
            RunStatus::Done => {
                if out.compile.ok {
                    report.compile_ok += 1;
                    feats.push("stage:compile".to_string());
                    if let Some(sim) = &out.sim {
                        if sim.ok {
                            report.sim_ok += 1;
                            feats.push("stage:sim".to_string());

                            // ── 6a. differential determinism (#13 basis; #19 oracle) ──
                            if rng.gen_bool(0.30) {
                                match differential::determinism_check(&src, cfg) {
                                    differential::DiffVerdict::Same => {}
                                    differential::DiffVerdict::Mismatch(d) => {
                                        report.determinism_mismatch += 1;
                                        report.bugs.push(BugRecord {
                                            kind: BugKind::Differential,
                                            source: src.clone(),
                                            detail: format!("determinism: {}", d),
                                        });
                                    }
                                    differential::DiffVerdict::Skip => {}
                                }
                            }
                            // ── 6b. differential EMI dead-code (#13) ──
                            if rng.gen_bool(0.20) {
                                match differential::emi_check(&src, cfg) {
                                    differential::DiffVerdict::Same => {}
                                    differential::DiffVerdict::Mismatch(d) => {
                                        report.emi_mismatch += 1;
                                        report.bugs.push(BugRecord {
                                            kind: BugKind::Differential,
                                            source: src.clone(),
                                            detail: format!("emi: {}", d),
                                        });
                                    }
                                    differential::DiffVerdict::Skip => {}
                                }
                            }
                        } else {
                            report.sim_err += 1;
                            feats.push(format!("err:{}", sim.code));
                        }
                    }
                } else {
                    report.compile_err += 1;
                    feats.push(format!("err:{}", out.compile.code));
                }
            }
        }

        // ── 5. feature map + corpus + adaptasi (#6/#7/#8) ──
        let is_new = guide.record(&parent, &feats);
        if is_new {
            report.new_features += 1;
            guide.record_adapt(true);
        } else {
            guide.record_adapt(false);
        }
        guide.note_visit(&parent);
        // Seed menarik (fitur baru / sim ok) masuk corpus utk mutasi lanjut.
        if out.compile.ok && (is_new || out.sim.as_ref().is_some_and(|s| s.ok)) {
            guide.add(src, feats, false);
        }

        if cfg.verbose && iter > 0 && iter % cfg.verbose_every == 0 {
            eprintln!(
                "[fuzz] iter {} {} ({} ms)",
                iter,
                report.summary(),
                started.elapsed().as_millis()
            );
        }
    }

    report.covered_features = guide.coverage().covered();
    if let Some(dir) = &cfg.emit_dir {
        emit_bugs(dir, &report);
    }
    report
}

/// Tulis bug ke direktori (1 file per bug) — dev-reporting, bukan edit kode.
fn emit_bugs(dir: &std::path::Path, report: &FuzzReport) {
    let _ = std::fs::create_dir_all(dir);
    for (i, b) in report.bugs.iter().enumerate() {
        let kind = match b.kind {
            BugKind::Panic => "panic",
            BugKind::Hang => "hang",
            BugKind::Differential => "diff",
        };
        let path = dir.join(format!("bug_{:04}_{}.sv", i, kind));
        let _ = std::fs::write(&path, &b.source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_sane() {
        let c = FuzzConfig::default();
        assert!(c.iters > 0);
        assert!(c.max_time > 0);
        assert!(c.hang_ms > 0);
        assert_eq!(c.workers, 1);
    }

    #[test]
    fn report_add_and_summary() {
        let mut a = FuzzReport::default();
        a.total = 10;
        let mut b = FuzzReport::default();
        b.total = 5;
        b.panics = 2;
        a.merge(&b);
        assert_eq!(a.total, 15);
        assert_eq!(a.panics, 2);
        assert!(a.summary().contains("panics=2"));
    }

    #[test]
    fn fuzz_smoke_small_campaign() {
        // Kampanye kecil: semua seed generated — harus selesai tanpa panic
        // di driver fuzzer itu sendiri.
        let cfg = FuzzConfig {
            iters: 40,
            seed: 0xabcd,
            hang_ms: 2000,
            ..FuzzConfig::default()
        };
        let rep = run_fuzz(&cfg);
        assert_eq!(rep.total, 40);
        assert!(rep.compile_ok > 0, "seed generated harus banyak yg compile ok");
    }
}