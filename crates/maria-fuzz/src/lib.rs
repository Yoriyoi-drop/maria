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
pub mod bugdb;
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
    /// Default = 12000: HARUS melebihi settle delta-storm engine (build debug
    /// ~10s utk delta-limit 100k). Input `always @(posedge)` tanpa clock adalah
    /// delta-storm yang ENGINE SETTLE (delta-limit) — selesai normal, bukan
    /// hang. Hanya hang SEJATI (parser/stack infinite, tak pernah settle) yang
    /// melewati ambang ini → Hang. Default lebih rendah (2000) keliru
    /// mengklasifikasi delta-storm sebagai Hang (false positive massal).
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
    /// Corpus seed bersama (worker paralel): kalau diisi, worker paralel
    /// pakai corpus ini alih-alih memuat ulang dari `corpus_dirs`.
    pub corpus: Option<crate::corpus::Corpus>,
    /// Activekan oracle nilai sinyal (#19) — injeksi input →
    /// fprint awal vs akhir vs akhir-2 divalidasi konsistensi.
    pub sim_sig_check: bool,
}

impl Default for FuzzConfig {
    fn default() -> Self {
        FuzzConfig {
            iters: 300,
            seed: 0x6d61_7269_61,
            max_time: 100,
            hang_ms: 12_000,
            corpus_dirs: Vec::new(),
            target: None,
            workers: 1,
            emit_dir: None,
            corpus: None,
            verbose: false,
            verbose_every: 100,
            sim_sig_check: false,
        }
    }
}

/// Klasifikasi bug yang ditemukan fuzzer.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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
    pub sim_sig_anomalies: u64,
    /// Property-oracle (Paper #2/#3/#14, oracle #5): mirror `lhs !== rhs`
    /// bernilai 1 — hasil assign tidak konsisten dgn re-evaluasi.
    pub property_violations: u64,
    pub covered_features: usize,
    /// Progress CDG (#20): rasio target hit vs total & unreached targets.
    pub cdg_info: Option<crate::cdg::CdgInfo>,
    pub unreached_targets: Vec<String>,
    /// Statistik & bobot adaptif per operator mutasi (GAP-3) — untuk
    /// evaluasi palet & debug, bukan untuk determinisme laporan.
    pub op_stats: ast_mutate::OpStats,
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
        self.sim_sig_anomalies += other.sim_sig_anomalies;
        self.property_violations += other.property_violations;
        self.covered_features = self.covered_features.max(other.covered_features);
        if let Some(other_cdg) = &other.cdg_info {
            if self.cdg_info.as_ref().map_or(true, |c| other_cdg.ratio > c.ratio) {
                self.cdg_info = Some(other_cdg.clone());
                self.unreached_targets = other.unreached_targets.clone();
            }
        }
        self.op_stats.merge(&other.op_stats);
        self.bugs.extend(other.bugs.clone());
    }

    /// Ringkasan satu-baris untuk konsole / merge.
    pub fn summary(&self) -> String {
        format!(
            "iters={} compile_ok={} compile_err={} sim_ok={} sim_err={} panics={} hangs={} \
             new_features={} det_mismatch={} emi_mismatch={} sig_anom={} prop_viol={} covered={} bugs={}",
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
            self.sim_sig_anomalies,
            self.property_violations,
            self.covered_features,
            self.bugs.len(),
        )
    }
}

/// Property-oracle invariant (Paper #2/#3/#14, oracle #5): blok mirror
/// `_fz_rtA_<id>`/`_fz_rtB_<id>` — dua temp yang mengevaluasi ekspresi SAMA —
/// harus UTUH. Utuh = dua net ter-deklarasi `wire [W-1:0]` lebar sama, masing-
/// masing TEPAT satu driver `assign`, rhs kedua assign identik, dan
/// `_fz_viol_<id>` ter-deklarasi dengan `(A !== B)`.
///
/// Mengapa wajib utuh (bug maria-fuzz, bukan engine):
/// 1. Minimizer baris bisa menghapus deklarasi `wire [W-1:0]` → temp jadi
///    implicit net (lebar default 2 di maria) → `A !== B` = 1 walaupun
///    ekspresi sama nilainya → viol=1 palsu berkelanjutan.
/// 2. Mutasi `duplicate_line` bisa meng-drive temp dua kali → multi-driver →
///    resolusi X → viol=1 palsu.
/// Blok mirror yang rusak = artefak fuzzer, bukan bug engine → di-skip.
fn mirror_intact(source: &str) -> bool {
    let lines: Vec<&str> = source.lines().map(|l| l.trim()).collect();
    let mut ids: Vec<String> = Vec::new();
    for l in &lines {
        if let Some(rest) = l.strip_prefix("assign _fz_viol_") {
            let id: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            let ok = !id.is_empty()
                && rest.trim_start_matches(&id).starts_with(" = ")
                && l.contains(&format!("_fz_rtA_{}", id))
                && l.contains(&format!("_fz_rtB_{}", id))
                && l.contains("!==");
            if !ok {
                return false;
            }
            ids.push(id);
        }
    }
    if ids.is_empty() {
        return false;
    }
    ids.iter().all(|id| mirror_pair_intact(&lines, id))
}

/// Validasi SATU blok mirror (lihat `mirror_intact`): deklarasi lebar sama,
/// tepat satu driver per temp, rhs identik, viol ter-deklarasi.
fn mirror_pair_intact(lines: &[&str], id: &str) -> bool {
    let rt_a = format!("assign _fz_rtA_{}", id);
    let rt_b = format!("assign _fz_rtB_{}", id);
    let net_a = format!("_fz_rtA_{}", id);
    let net_b = format!("_fz_rtB_{}", id);
    let viol_net = format!("_fz_viol_{}", id);

    let assigns_a: Vec<&str> = lines
        .iter()
        .filter(|l| l.starts_with(rt_a.as_str()) && l.ends_with(';'))
        .copied()
        .collect();
    let assigns_b: Vec<&str> = lines
        .iter()
        .filter(|l| l.starts_with(rt_b.as_str()) && l.ends_with(';'))
        .copied()
        .collect();
    if assigns_a.len() != 1 || assigns_b.len() != 1 {
        return false;
    }
    let rhs_of = |assign: &str| assign.split_once('=').map(|(_, r)| r.trim().to_string());
    match (rhs_of(assigns_a[0]), rhs_of(assigns_b[0])) {
        (Some(a), Some(b)) if a == b && !a.is_empty() => {}
        _ => return false,
    }
    // Deklarasi `wire [..-1:0] _fz_rtX_<id>;` tepat satu, spec lebar IDENTIK.
    let decl_spec = |net: &str| -> Option<String> {
        let hits: Vec<&str> = lines
            .iter()
            .filter(|l| l.starts_with("wire [") && l.contains(net) && l.ends_with(';') && l.contains("-1:0]"))
            .copied()
            .collect();
        if hits.len() != 1 {
            return None;
        }
        let l = hits[0];
        let s = l.find('[')?;
        let e = l[s..].find(']')? + s;
        Some(l[s..=e].to_string())
    };
    let (da, db) = (decl_spec(&net_a), decl_spec(&net_b));
    match (da, db) {
        (Some(a), Some(b)) if a == b => {}
        _ => return false,
    }
    // `wire _fz_viol_<id>;` ter-deklarasi tepat satu.
    let viol_decls = lines
        .iter()
        .filter(|l| l.starts_with("wire ") && l.contains(viol_net.as_str()) && l.ends_with(';'))
        .count();
    viol_decls == 1
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
    // Bug DB persisten (Paper #18): load bug dari kampanye sebelumnya → re-seed
    // supaya regressi terus diexercise (jika DB ada).
    let db_path = if let Some(ref dir) = cfg.emit_dir {
        bugdb::db_path(dir)
    } else {
        PathBuf::from(".maria-fuzz-bugdb.json")
    };
    let mut bug_db = bugdb::BugDb::load(&db_path).unwrap_or_else(|| bugdb::BugDb::empty());

    let mut rng = StdRng::seed_from_u64(cfg.seed);
    let gen = Generator::new(cfg.seed);
    let mut guide = guide::CoverageGuide::new(cfg.seed);
    let mut corpus = if let Some(c) = cfg.corpus.clone() {
        c
    } else {
        corpus::Corpus::from_dirs(&cfg.corpus_dirs)
    };

    // Re-seed bug lama (Paper #18: FSM-aware regressi): masukkan ke corpus
    // SEKAliGUS ke guide sebagai parent mutasi (is_bug=true → energi tinggi,
    // guide.rs #20 bug-boosted). Sebelumnya hanya masuk corpus.seeds
    // (dipakai fragment) → bug db tidak pernah jadi parent aktif = regressi
    // tidak benar-benar diexercise lintas kampanye.
    for src in bug_db.reseed_sources() {
        corpus.seeds.push(src.clone());
        let feats = feature::FeatureMap::extract(&src);
        guide.add(src, feats, true);
    }
    let mut report = FuzzReport {
        sim_sig_anomalies: 0,
        ..FuzzReport::default()
    };
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
    // Statistik & bobot adaptif palet mutasi (GAP-3).
    let mut op_stats = ast_mutate::OpStats::default();

    for iter in 0..cfg.iters {
        // ── 1. pilih parent (energy schedule) ──
        let Some(parent) = guide.select() else { break };

        // ── 2. rantai mutasi (1..3 op; tiap op dipilih adaptif) ──
        let chain = rng.gen_range(1..=3);
        let mut src = parent.clone();
        let mut ops_used: Vec<usize> = Vec::with_capacity(chain as usize);
        for _ in 0..chain {
            let (op, next) = ast_mutate::mutate(&mut rng, &src, &corpus, &mut op_stats);
            ops_used.push(op);
            src = next;
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
        if let Ok(dump_path) = std::env::var("MARIA_FUZZ_DUMP") {
            // Debug: tulis kandidat terakhir sebelum eksekusi — bila proses
            // abort (mis. stack overflow) file ini = input penyebab.
            std::fs::write(&dump_path, &src).ok();
        }
        let out = harness::run_isolated(&src, cfg.max_time, cfg.hang_ms);
        report.total += 1;

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
                detail.push_str(&format!("\nminimized {} → {} bytes", src.len(), minimized.len()));
                let bug = BugRecord {
                    kind: BugKind::Panic,
                    source: minimized,
                    detail,
                };
                report.bugs.push(bug.clone());
                bug_db.push(&bug, cfg.seed, iter + 1);
            }
            RunStatus::Hang => {
                report.hangs += 1;
                // Konfirmasi SEBELUM minimasi: input yang hanya LAMBAT tapi
                // SEBENARNYA selesai (mis. delta-storm `always @(posedge clk)`
                // tanpa clock yang di-hentikan engine lewat delta-limit 100k,
                // settle ~10s di build debug) bukan hang sejati — engine
                // menangguhkan, bukan infinite. Validasi dgn window jauh lebih
                // besar dari settle; bila selesai → bukan hang → minggir tanpa
                // biaya minimasi mahal.
                let confirm_ms = cfg.hang_ms.max(15_000);
                if matches!(
                    harness::run_isolated(&src, cfg.max_time, confirm_ms).status,
                    RunStatus::Done
                ) {
                    continue;
                }
                // Hang sejati (tak selesai bahkan melewati window besar): minimasi
                // baris sambil status tetap Hang. Ambang minimasi pendek (≤800ms)
                // agar tiap kandidat hang tak bakar penuh; kandidat non-hang
                // selesai cepat → cost total wajar.
                let min_ms = cfg.hang_ms.min(800).max(200);
                let minimized = corpus::Corpus::minimize(&src, &mut |cand| {
                    matches!(
                        harness::run_isolated(cand, cfg.max_time, min_ms).status,
                        RunStatus::Hang
                    ) || cand.is_empty()
                });
                let bug = BugRecord {
                    kind: BugKind::Hang,
                    source: minimized.clone(),
                    detail: format!(
                        "hang > {} ms ({}); minimized {} → {} bytes",
                        cfg.hang_ms,
                        out.duration_ms,
                        src.len(),
                        minimized.len()
                    ),
                };
                report.bugs.push(bug.clone());
                bug_db.push(&bug, cfg.seed, iter + 1);
            }
            RunStatus::Done => {
                if out.compile.ok {
                    report.compile_ok += 1;
                    if let Some(sim) = &out.sim {
                        if sim.ok {
                            report.sim_ok += 1;

                            // ── 6a. differential determinism (#13 basis; #19 oracle) ──
                            if rng.gen_bool(0.30) {
                                match differential::determinism_check(&src, cfg) {
                                    differential::DiffVerdict::Same => {}
                                    differential::DiffVerdict::Mismatch(d) => {
                                        report.determinism_mismatch += 1;
                                        // Minimizer determinism: kandidat masih harus
                                        // menghasilkan determinism mismatch yang sama PADA
                                        // source VALID (compile+sim ok). Minimizer baris bisa
                                        // menghapus deklarasi → implicit net / multi-driver /
                                        // referensi hilang → fingerprint beda `z` vs `x` BUKAN
                                        // bukti bug engine (artefak minimizer, sama pola M3/F1).
                                        let minimized = corpus::Corpus::minimize(
                                            &src,
                                            &mut |cand| {
                                                matches!(harness::run_isolated(cand, cfg.max_time, cfg.hang_ms).status, RunStatus::Done)
                                                    && harness::fingerprint_isolated(cand, cfg.max_time, cfg.hang_ms).is_some()
                                                    && matches!(
                                                        differential::determinism_check(cand, cfg),
                                                        differential::DiffVerdict::Mismatch(_)
                                                    )
                                            },
                                        );
                                        let mlen = minimized.len();
                                        report.bugs.push(BugRecord {
                                            kind: BugKind::Differential,
                                            source: minimized,
                                            detail: format!("determinism: {}; minimized {}→{} bytes",
                                                d, src.len(), mlen),
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
                                        // Minimizer EMI (#13): kandidat masih harus
                                        // menghasilkan mismatch vs dead-code variant PADA
                                        // source VALID (compile+sim ok). Source malformed
                                        // (implicit net / multi-driver) memberi fingerprint
                                        // `z` vs `x` beda yang bukan bug engine.
                                        let minimized = corpus::Corpus::minimize(
                                            &src,
                                            &mut |cand| {
                                                matches!(harness::run_isolated(cand, cfg.max_time, cfg.hang_ms).status, RunStatus::Done)
                                                    && harness::fingerprint_isolated(cand, cfg.max_time, cfg.hang_ms).is_some()
                                                    && matches!(
                                                        differential::emi_check(cand, cfg),
                                                        differential::DiffVerdict::Mismatch(_)
                                                    )
                                            },
                                        );
                                        let mlen = minimized.len();
                                        report.bugs.push(BugRecord {
                                            kind: BugKind::Differential,
                                            source: minimized,
                                            detail: format!("emi: {}; minimized {}→{} bytes",
                                                d, src.len(), mlen),
                                        });
                                    }
                                    differential::DiffVerdict::Skip => {}
                                }
                            }
                            // ── 6c. oracle nilai sinyal (#19): injeksi input →
                            //      fprint awal vs akhir berbeda-beda → anomali internal.
                            if cfg.sim_sig_check {
                                if let Some(info) = oracle::sim_signal_check(&src, cfg.max_time) {
                                    report.sim_sig_anomalies += 1;
                                    let minimized = corpus::Corpus::minimize(
                                        &src,
                                        &mut |cand| oracle::sim_signal_check(cand, cfg.max_time).is_some(),
                                    );
                                    let mlen = minimized.len();
                                    report.bugs.push(BugRecord {
                                        kind: BugKind::Differential,
                                        source: minimized,
                                        detail: format!("sim-sig: {}; minimized {}→{} bytes",
                                            info, src.len(), mlen),
                                    });
                                }
                            }
                        // ── 6d. property-oracle (#2/#3/#14, oracle #5):
                            //      dua temp mirror (`_fz_rtA`/`_fz_rtB`) yang
                            //      mengevaluasi ekspresi SAMA memberi hasil
                            //      beda → `_fz_viol = 1` = bug evaluasi.
                            if mirror_intact(&src) {
                                if let Some(viol) = oracle::property_violation(&sim.fingerprint) {
                                    report.property_violations += 1;
                                    let minimized = corpus::Corpus::minimize(
                                        &src,
                                        &mut |cand| {
                                            // Invariant: blok mirror masih utuh
                                            // (deklarasi+driver+rhs; viol==1).
                                            mirror_intact(cand)
                                                && harness::fingerprint_isolated(
                                                    cand,
                                                    cfg.max_time,
                                                    cfg.hang_ms,
                                                )
                                                .map(|fp| {
                                                    oracle::property_violation(&fp).is_some()
                                                })
                                                .unwrap_or(false)
                                        },
                                    );
                                    let mlen = minimized.len();
                                    report.bugs.push(BugRecord {
                                        kind: BugKind::Differential,
                                        source: minimized,
                                        detail: format!(
                                            "property: {}; minimized {}→{} bytes",
                                            viol,
                                            src.len(),
                                            mlen
                                        ),
                                    });
                                }
                            }
                        } else {
                            report.sim_err += 1;
                            // Dev diagnostic (env gate): cetak source + code + message
                            // SIM error yang bukan assert-oracle — utk inspeksi manual
                            // apakah sim_err = bug engine atau input tak-sah. Tidak
                            // termasuk campaign default (hanya bila env di-set).
                            if std::env::var("MARIA_FUZZ_SIMERR").is_ok() && !ast_mutate::has_assert_oracle(&src)
                            {
                                eprintln!(
                                    "[simerr] code={} msg={}\n---\n{}\n---",
                                    sim.code,
                                    sim.message,
                                    src
                                );
                            }
                            // Property-oracle assert (Paper #14/#2/#3, oracle #5):
                            // seed memuat assert-oracle (`_fz_atA/_fz_atB` eval ekspresi
                            // identik). Sim error = assertion FAIL = bug evaluasi.
                            // HANYA code RT7001 (dari `assert ... else $fatal`) yang
                            // menandakan assertion violate. sim_err lain (mis. RT0001
                            // hier-signal not found dari seed malformed yang punya
                            // referensi tak-resolved, RT2001 delta-storm) BUKAN bug
                            // engine — jangan salah-klaim.
                            if sim.code.contains("RT7001")
                                && ast_mutate::has_assert_oracle(&src)
                                && ast_mutate::has_assert_oracle_temps(&src)
                            {
                                report.property_violations += 1;
                                let minimized = corpus::Corpus::minimize(
                                    &src,
                                    &mut |cand| {
                                        ast_mutate::has_assert_oracle_temps(cand)
                                            && matches!(
                                                harness::run_isolated(cand, cfg.max_time, cfg.hang_ms).status,
                                                RunStatus::Done
                                            )
                                            && harness::sim_err_isolated(cand, cfg.max_time, cfg.hang_ms)
                                                .map(|c| c.contains("RT7001"))
                                                .unwrap_or(false)
                                    },
                                );
                                let mlen = minimized.len();
                                report.bugs.push(BugRecord {
                                    kind: BugKind::Differential,
                                    source: minimized,
                                    detail: format!(
                                        "assert-oracle: {}{} — ekspresi identik dievaluasi beda; minimized {}→{} bytes",
                                        sim.code, sim.message, src.len(), mlen
                                    ),
                                });
                            }
                        }
                    }
                } else {
                    report.compile_err += 1;
                }
            }
        }

        // ── 5. feature map + corpus + adaptasi (#6/#7/#8) ──
        // Execution-gated (audit GAP-2): HANYA hasil sim_ok yang membuktikan
        // fitur benar-benar tereksekusi. Panic/Hang/compile_err/sim_err TIDAK
        // masuk peta — mencegah "coverage" dari testcase invalid dan saturasi
        // dini (energi & CDG jadi jujur). Fitur eksekusi = fitur teks child +
        // stage + coverage keys nyata engine (line/branch/toggle/FSM).
        let executed = matches!(&out.status, RunStatus::Done)
            && out.compile.ok
            && out.sim.as_ref().map(|s| s.ok).unwrap_or(false);
        let mut is_new = false;
        if executed {
            let mut exec_feats = feature::FeatureMap::extract(&src);
            exec_feats.push("stage:compile".to_string());
            exec_feats.push("stage:sim".to_string());
            exec_feats.extend(out.coverage.iter().cloned());
            is_new = guide.map_record(&exec_feats);
            if is_new {
                report.new_features += 1;
                // Seed menarik (fitur eksekusi baru) masuk corpus utk mutasi
                // lanjut (mencegah corpus saturation & starvation).
                guide.add(src.clone(), exec_feats, false);
                // Atribusi: parent yang memicu novelty dapat dorongan energi.
                guide.boost_source(&parent);
            }
        }
        // Atribusi op-level (GAP-3): op yang memicu novelty naik bobot palet.
        for op in &ops_used {
            op_stats.record_outcome(*op, is_new);
        }
        guide.record_adapt(is_new);
        // Cost-aware visit (GAP-4): seed mahal dikecilkan energinya.
        guide.note_visit(&parent, out.duration_ms);

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
    let cov_map = guide.coverage();
    report.cdg_info = Some(cdg::report(&cov_map));
    report.unreached_targets = cdg::plan_targets(&cov_map);
    report.op_stats = op_stats;

    // Simpan bug DB persisten (Paper #18) — sebelum emit_bugs supaya
    // entri baru tercatat di disk. Direktori emit dibuat DULUAN: kampanye
    // seed 42 gagal `bug_db.save` dengan "No such file or directory"
    // karena dir emit belum ada (ordering bug).
    if let Some(dir) = &cfg.emit_dir {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = bug_db.save(&db_path) {
        eprintln!("[fuzz] warning: gagal simpan bug DB: {}", e);
    }

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

    #[test]
    fn mirror_intact_accepts_full_block() {
        let src = concat!(
            "module top;\n  logic [2-1:0] r;\n  assign r = 0;\n",
            "  wire [2-1:0] _fz_rtA_7;\n  assign _fz_rtA_7 = (r);\n",
            "  wire [2-1:0] _fz_rtB_7;\n  assign _fz_rtB_7 = (r);\n",
            "  wire _fz_viol_7;\n  assign _fz_viol_7 = (_fz_rtA_7 !== _fz_rtB_7);\nendmodule\n"
        );
        assert!(mirror_intact(src), "blok mirror utuh harus diterima");
    }

    #[test]
    fn mirror_intact_rejects_missing_wire_decl() {
        // Artefak minimizer: deklarasi `wire [W-1:0] _fz_rtB` hilang → implicit
        // net (lebar default) → width-mismatch → viol=1 palsu berkelanjutan.
        let src = concat!(
            "module top;\n  logic [2-1:0] r;\n",
            "  wire [2-1:0] _fz_rtA_7;\n  assign _fz_rtA_7 = (r);\n",
            "  assign _fz_rtB_7 = (r);\n",
            "  wire _fz_viol_7;\n  assign _fz_viol_7 = (_fz_rtA_7 !== _fz_rtB_7);\nendmodule\n"
        );
        assert!(!mirror_intact(src), "deklarasi temp hilang = artefak, bukan bug engine");
    }

    #[test]
    fn mirror_intact_rejects_duplicate_driver() {
        // Artefak dup-line: temp di-drive 2x → multi-driver → X → viol=1 palsu.
        let src = concat!(
            "module top;\n  logic [2-1:0] r;\n",
            "  wire [2-1:0] _fz_rtA_7;\n  assign _fz_rtA_7 = (r);\n  assign _fz_rtA_7 = (r);\n",
            "  wire [2-1:0] _fz_rtB_7;\n  assign _fz_rtB_7 = (r);\n",
            "  wire _fz_viol_7;\n  assign _fz_viol_7 = (_fz_rtA_7 !== _fz_rtB_7);\nendmodule\n"
        );
        assert!(!mirror_intact(src), "multi-driver temp = artefak fuzzer, bukan bug engine");
    }

    #[test]
    fn mirror_intact_rejects_width_mismatch() {
        let src = concat!(
            "module top;\n  logic [2-1:0] r;\n",
            "  wire [2-1:0] _fz_rtA_7;\n  assign _fz_rtA_7 = (r);\n",
            "  wire [1-1:0] _fz_rtB_7;\n  assign _fz_rtB_7 = (r);\n",
            "  wire _fz_viol_7;\n  assign _fz_viol_7 = (_fz_rtA_7 !== _fz_rtB_7);\nendmodule\n"
        );
        assert!(!mirror_intact(src), "lebar temp beda = invariant rusak");
    }

    #[test]
    fn mirror_intact_rejects_rhs_split() {
        let src = concat!(
            "module top;\n  logic [2-1:0] r;\n",
            "  wire [2-1:0] _fz_rtA_7;\n  assign _fz_rtA_7 = (r);\n",
            "  wire [2-1:0] _fz_rtB_7;\n  assign _fz_rtB_7 = (r + 1);\n",
            "  wire _fz_viol_7;\n  assign _fz_viol_7 = (_fz_rtA_7 !== _fz_rtB_7);\nendmodule\n"
        );
        assert!(!mirror_intact(src), "rhs berbeda = invariant rusak");
    }

    #[test]
    fn mirror_intact_rejects_no_viol_decl() {
        let src = concat!(
            "module top;\n  logic [2-1:0] r;\n",
            "  wire [2-1:0] _fz_rtA_7;\n  assign _fz_rtA_7 = (r);\n",
            "  wire [2-1:0] _fz_rtB_7;\n  assign _fz_rtB_7 = (r);\n",
            "  assign _fz_viol_7 = (_fz_rtA_7 !== _fz_rtB_7);\nendmodule\n"
        );
        assert!(!mirror_intact(src), "deklarasi viol hilang = artefak minimizer");
    }
}