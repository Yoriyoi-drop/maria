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
pub mod faults;
pub mod gen;
pub mod grammar;
pub mod guide;
pub mod harness;
pub mod interaction;
pub mod mvgen;
pub mod mv_lower;
pub mod mv_mutate;
pub mod oracle;
pub mod real;
pub mod semantic;
pub mod sweep;
pub mod testcase;

use std::path::PathBuf;
use std::time::Instant;

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};

use gen::Generator;
use harness::RunStatus;

/// Konfigurasi satu kampanye fuzzing.
#[derive(Debug, Clone)]
pub struct FuzzConfig {
    /// Jumlah iterasi fuzz (tiap iterasi = 1+ eksekusi compile/simulasi).
    pub iters: u64,
    /// Backend testcase: `Direct` (jalur lama — generator SV → Maria langsung)
    /// atau `MvMediated` (scenario `.mv` → Maria-MV → HDL → Maria). Task §14.
    pub backend: crate::testcase::Backend,
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
    /// Direktori tujuan persist seed menarik (GAP-10) — parent aktif kampanye
    /// ini jadi corpus kampanye berikutnya (opsional; default tidak menulis).
    pub save_corpus_dir: Option<PathBuf>,
    /// Eksekusi dgn SUBPROCESS (GAP-9, opt-in): hang di-KILL sejati (thread
    /// Rust tak bisa dibunuh → leak CPU), stack-overflow (SIGSEGV) terdeteksi
    /// via exit code (catch_unwind buta). Biaya spawn ~ms per run.
    pub proc_isolate: bool,
    /// Activekan oracle nilai sinyal (#19) — injeksi input →
    /// fprint awal vs akhir vs akhir-2 divalidasi konsistensi.
    pub sim_sig_check: bool,
    /// Differential vs tool referensi LRM (iverilog/verilator) — DUT core
    /// pasif + tb eksternal; bandingkan trace maria vs iverilog. Menangkap
    /// deviasi semantik yang konsisten-diri (buta bagi oracle lain).
    pub ref_diff: bool,
    /// Filelist proyek nyata (`.f`/`.maria`) untuk REAL-PROJECT error hunt
    /// (GAP-11): tiap file di-compile → error diklasifikasi & terminimalkan ke
    /// reproducer. Bukan feature-gap scorecard — kandidat bug internal/cascade.
    pub real_files: Vec<PathBuf>,
}

impl Default for FuzzConfig {
    fn default() -> Self {
        FuzzConfig {
            iters: 300,
            backend: crate::testcase::Backend::Direct,
            seed: 0x6d61_7269_61,
            max_time: 100,
            hang_ms: 12_000,
            corpus_dirs: Vec::new(),
            target: None,
            workers: 1,
            emit_dir: None,
            corpus: None,
            save_corpus_dir: None,
            proc_isolate: false,
            verbose: false,
            verbose_every: 100,
            sim_sig_check: true,
            ref_diff: false,
            real_files: Vec::new(),
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
    /// Backend MvMediated: HDL hasil lower (minimized) — MV canonical ada di `mv`.
    pub source: String,
    pub detail: String,
    /// Metadata testcase MV (backend MvMediated): canonical `.mv` (reproducible).
    pub mv: Option<String>,
    /// HDL hasil lower testcase (sebelum minimasi) — task §8: simpan keduanya.
    pub hdl: Option<String>,
    /// Mutation history (op ids) — task §8/§9.
    pub mv_history: Vec<String>,
}

impl BugRecord {
    /// Bangun bug dgn metadata testcase MV (None utk backend Direct).
    pub fn new(
        kind: BugKind,
        source: String,
        detail: String,
        tc: Option<&crate::testcase::Testcase>,
    ) -> Self {
        BugRecord {
            kind,
            source,
            detail,
            mv: tc.map(|t| t.mv.clone()),
            hdl: tc.map(|t| t.hdl.clone()),
            mv_history: tc.map(|t| t.history.clone()).unwrap_or_default(),
        }
    }
}

/// Laporan satu kampanye fuzzing.
#[derive(Debug, Clone, Default)]
pub struct FuzzReport {
    pub total: u64,
    pub compile_ok: u64,
    pub compile_err: u64,
    /// Compile error dari HDL valid (lower-check-ok MV → HDL ditolak maria —
    /// kandidat bug maria parser/elaborator / maria-mv lowering mismatch).
    pub lower_good_compile_err: u64,
    pub sim_ok: u64,
    pub sim_err: u64,
    /// Runtime error (RT####) pada input parse-clean — kandidat bug engine.
    pub sim_err_clean: u64,
    pub panics: u64,
    pub hangs: u64,
    pub new_features: u64,
    pub determinism_mismatch: u64,
    pub emi_mismatch: u64,
    /// Metamorphic-identity oracle (GAP-5): `rhs op 0 ≡ rhs` dilanggar.
    pub meta_mismatch: u64,
    pub sim_sig_anomalies: u64,
    /// Property-oracle (Paper #2/#3/#14, oracle #5): mirror `lhs !== rhs`
    /// bernilai 1 — hasil assign tidak konsisten dgn re-evaluasi.
    pub property_violations: u64,
    /// Differential vs tool referensi LRM: trace maria berbeda dari iverilog.
    pub ref_mismatch: u64,
    pub covered_features: usize,
    /// Progress CDG (#20): rasio target hit vs total & unreached targets.
    pub cdg_info: Option<crate::cdg::CdgInfo>,
    pub unreached_targets: Vec<String>,
    /// Gap kepatuhan IEEE 1800: seed corpus SV NYATA yang maria tolak saat
    /// compile (fitur LRM belum didukung / regressi parse). Bukan crash —
    /// dicatat terpisah utk roadmap kepatuhan.
    pub corpus_gap_total: u64,
    pub corpus_gap_samples: Vec<String>,
    /// Kode diagnostic (E####/EL####) tiap gap — peta fitur LRM yang belum
    /// didukung (scorecard kepatuhan IEEE 1800), dedup.
    pub corpus_gap_codes: Vec<String>,
    /// Seed ditolak di ELABORASI (code EL####) — lunak: sering file fragment/
    /// header non-standalone, bukan fitur LRM hilang.
    pub corpus_elab_reject: u64,
    /// Jumlah seed corpus yang diuji oracle gap.
    pub corpus_tested: u64,
    /// Hasil project-wide sweep korpus nyata (Paper #12/#18): SEMUA error
    /// (parse+elab) saat seluruh proyek di-compile sebagai satu design —
    /// dengan file:line:col & klasifikasi kategori. `None` bila tidak diminta.
    pub project_sweep: Option<crate::sweep::ProjectSweep>,
    /// Hasil REAL-PROJECT error hunt (GAP-11): per-file compile →
    /// klasifikasi + minimasi ke reproducer. Kandidat bug internal/cascade.
    pub real_hunt: Option<crate::real::RealHuntReport>,
    /// Statistik & bobot adaptif per operator mutasi (GAP-3) — untuk
    /// evaluasi palet & debug, bukan untuk determinisme laporan.
    pub op_stats: ast_mutate::OpStats,
    /// Statistik operator mutasi MV (backend MvMediated) — setara op_stats.
    pub mv_op_stats: mv_mutate::MvOpStats,
    pub bugs: Vec<BugRecord>,
}

impl FuzzReport {
    pub fn merge(&mut self, other: &FuzzReport) {
        self.total += other.total;
        self.compile_ok += other.compile_ok;
        self.compile_err += other.compile_err;
        self.lower_good_compile_err += other.lower_good_compile_err;
        self.sim_ok += other.sim_ok;
        self.sim_err += other.sim_err;
        self.sim_err_clean += other.sim_err_clean;
        self.panics += other.panics;
        self.hangs += other.hangs;
        self.new_features += other.new_features;
        self.determinism_mismatch += other.determinism_mismatch;
        self.emi_mismatch += other.emi_mismatch;
        self.meta_mismatch += other.meta_mismatch;
        self.sim_sig_anomalies += other.sim_sig_anomalies;
        self.property_violations += other.property_violations;
        self.ref_mismatch += other.ref_mismatch;
        self.covered_features = self.covered_features.max(other.covered_features);
        if let Some(other_cdg) = &other.cdg_info {
            if self.cdg_info.as_ref().map_or(true, |c| other_cdg.ratio > c.ratio) {
                self.cdg_info = Some(other_cdg.clone());
                self.unreached_targets = other.unreached_targets.clone();
            }
        }
        self.op_stats.merge(&other.op_stats);
        self.mv_op_stats.merge(&other.mv_op_stats);
        self.bugs.extend(other.bugs.clone());
        self.corpus_gap_total += other.corpus_gap_total;
        self.corpus_tested += other.corpus_tested;
        self.corpus_elab_reject += other.corpus_elab_reject;
        for s in &other.corpus_gap_samples {
            if !self.corpus_gap_samples.contains(s) {
                self.corpus_gap_samples.push(s.clone());
            }
        }
        for c in &other.corpus_gap_codes {
            if !self.corpus_gap_codes.contains(c) {
                self.corpus_gap_codes.push(c.clone());
            }
        }
        if self.project_sweep.is_none() {
            self.project_sweep = other.project_sweep.clone();
        }
        if self.real_hunt.is_none() {
            self.real_hunt = other.real_hunt.clone();
        }
    }

    /// Ringkasan satu-baris untuk konsole / merge.
    pub fn summary(&self) -> String {
        format!(
            "iters={} compile_ok={} compile_err={}(hdl_ok={}) sim_ok={} sim_err={} sim_err_clean={} panics={} hangs={} \
             new_features={} det_mismatch={} emi_mismatch={} meta_mismatch={} sig_anom={} prop_viol={} ref_mismatch={} covered={} bugs={}",
            self.total,
            self.compile_ok,
            self.compile_err,
            self.lower_good_compile_err,
            self.sim_ok,
            self.sim_err,
            self.sim_err_clean,
            self.panics,
            self.hangs,
            self.new_features,
            self.determinism_mismatch,
            self.emi_mismatch,
            self.meta_mismatch,
            self.sim_sig_anomalies,
            self.property_violations,
            self.ref_mismatch,
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
    // guide.rs #20 bug-boosted). HANYA backend Direct — MvMediated memakai
    // sumber MV (lihat bootstrap MV di bawah).
    if cfg.backend == crate::testcase::Backend::Direct {
        for src in bug_db.reseed_sources() {
            corpus.seeds.push(src.clone());
            let feats = feature::FeatureMap::extract(&src);
            guide.add(src, feats, true);
        }
    }
    // ── Gap kepatuhan IEEE 1800 (Tahap: proyek NYATA): seed corpus SV nyata
    //    (test/, opentitan/, cva6/, examples/) yang maria TOLAK saat compile
    //    = fitur LRM belum didukung / regressi parse. Dua kelas:
    //    • PARSE (code E####) = gap parser keras (fitur sintaks LRM) → scorecard.
    //    • ELAB (code EL####) = lunak — sering file fragment/header non-standalone
    //      (include guard, definisi tanpa modul) → counter terpisah, bukan gap.
    //    Artifak fuzzer (nama `fz_`/`_fuzz`) di-skip.
    let mut corpus_gap_total: u64 = 0;
    let mut corpus_gap_samples: Vec<String> = Vec::new();
    let mut corpus_gap_codes: Vec<String> = Vec::new();
    let mut corpus_elab_reject: u64 = 0;
    // Gap-check HANYA backend Direct: corpus SV nyata dipakai compile langsung.
    // MvMediated memakai corpus MV (mvgen) — compile SV di main thread (stack
    // kecil) atas file raksasa (regtop opentitan) = stack overflow (bug fuzz
    // runner: main thread 8MB vs rekursi parser/elaborator dalam).
    if cfg.backend == crate::testcase::Backend::Direct {
        for seed in &corpus.seeds {
        if seed.contains("fz_") || seed.contains("_fuzz") {
            continue;
        }
        if let Err(e) = maria_api::compile_str_quiet(seed) {
            let code = e.error_code().to_string();
            // Parse keras = kode E1xxx (UnexpectedToken/ExpectedToken/
            // ExpectedSemi/UnclosedBlock/InvalidSyntax). E2xxx semantic,
            // E3xxx/EL3xxx elaborasi, E9xxx runtime — bukan gap sintaks.
            let is_syntax = code.starts_with("E1");
            if is_syntax {
                corpus_gap_total += 1;
                if !corpus_gap_codes.contains(&code) {
                    corpus_gap_codes.push(code);
                }
                if corpus_gap_samples.len() < 5 {
                    let head: Vec<&str> = seed.lines().take(8).collect();
                    corpus_gap_samples.push(head.join(" | "));
                }
            } else {
                corpus_elab_reject += 1;
            }
        }
        }
    }
    let mut report = FuzzReport {
        sim_sig_anomalies: 0,
        corpus_gap_total,
        corpus_gap_samples,
        corpus_gap_codes,
        corpus_elab_reject,
        corpus_tested: corpus.seeds.len() as u64,
        ..FuzzReport::default()
    };
    // ── Project-wide seed sweep (Paper #12/#18): korpus nyata di-compile
    //    sebagai SATU design → SEMUA error (parse+elab) dengan file:line:col.
    //    Gap-check per-file standalone di atas MELEWATKAN error yang hanya
    //    muncul lintas-file (dependensi/macro/package/parameter) — mis. 1679
    //    parse + 8 semantik + 23 hierarki opentitan penuh. Sweep menutupnya:
    //    seed nyata menjadi target pencarian error, bukan sekadar fragment.
    //    Sample deterministik `SWEEP_CAP` file (stride) agar tidak membakar
    //    budget kampanye; `cap=0` via env MARIA_FUZZ_SWEEP_FULL utk semua.
    let sweep_dirs: Vec<PathBuf> = if cfg.corpus_dirs.is_empty() {
        vec![
            PathBuf::from("test"),
            PathBuf::from("examples"),
            PathBuf::from("fuzz"),
            PathBuf::from("opentitan"),
            PathBuf::from("cva6"),
        ]
        .into_iter()
        .filter(|d| d.exists())
        .collect()
    } else {
        cfg.corpus_dirs.clone()
    };
    let sweep_cap: usize = if std::env::var("MARIA_FUZZ_SWEEP_FULL").is_ok() {
        0
    } else {
        300
    };
    if !sweep_dirs.is_empty() {
        let t0 = std::time::Instant::now();
        let sweep = crate::sweep::sweep_corpus(&sweep_dirs, sweep_cap);
        eprintln!(
            "[fuzz] project-sweep: {} file → {} error ({} ms) — kode: {}",
            sweep.files_total,
            sweep.errors_total,
            t0.elapsed().as_millis(),
            if sweep.all_codes.is_empty() {
                "-".to_string()
            } else {
                sweep.all_codes.join(",")
            }
        );
        report.project_sweep = Some(sweep);
    }

    // ── REAL-PROJECT error hunt (GAP-11): per-file compile → klasifikasi →
    //    minimasi ke reproducer. Menyerang KEDALAMAN: 1679 error parse
    //    opentitan dari CLI `--filelist ... --recompile` tidak pernah
    //    direduksi—bisa jadi cuma 1 fitur LRM + recovery cascade ribuan
    //    error lanjutan. Hunt menemukan konstruk SEBENARNYA yang gagal
    //    (minimized reproducer per error code).
    if !cfg.real_files.is_empty() {
        let t0 = std::time::Instant::now();
        // Batasi file yang diproses per kampanye (budget) — filelist besar
        // (3920 file) dengan compile per-file mahal.
        let cap: usize = std::env::var("MARIA_FUZZ_REAL_FILES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(600);
        let mut files: Vec<PathBuf> = Vec::new();
        for f in &cfg.real_files {
            if f.is_file() {
                files.push(f.clone());
            } else if f.is_dir() {
                let mut collected: Vec<PathBuf> = Vec::new();
                collect_sv_paths(f, &mut collected);
                files.extend(collected);
            }
        }
        // File besar dulu (kedalaman struktural) — deterministik.
        files.sort_by(|a, b| b.metadata().map(|m| m.len()).unwrap_or(0)
            .cmp(&a.metadata().map(|m| m.len()).unwrap_or(0)));
        files.truncate(cap);
        let hunt = crate::real::hunt_files(&files);
        eprintln!(
            "[fuzz] real-hunt: {} file → ok={} gap={} internal_bug={} compile_err={} ({} ms)",
            hunt.files_scanned,
            hunt.ok,
            hunt.feature_gaps,
            hunt.internal_bugs,
            hunt.compile_errs,
            t0.elapsed().as_millis()
        );
        for b in &hunt.bug_candidates {
            eprintln!(
                "  [real] {} | {} ({} error) — reproduce {} bytes",
                b.kind.label(),
                b.path.display(),
                b.total_errors,
                b.minimized.len()
            );
        }
        report.real_hunt = Some(hunt);
    }
    let started = Instant::now();

    // ── Seed awal (Tahap: proyek NYATA, Paper #12/#18): corpus SV nyata
    //    (test/, opentitan/, cva6/, examples/) jadi parent mutasi utama —
    //    bukan modul generated. HANYA backend Direct: MvMediated memakai
    //    corpus MV (mvgen) — seed SV tak valid sebagai parent .mv.
    if cfg.backend == crate::testcase::Backend::Direct {
        let real_seeds = corpus.sample_batch(&mut rng, 48);
        if real_seeds.is_empty() {
            for _ in 0..16 {
                let s = gen.random_module(&mut rng);
                let feats = feature::FeatureMap::extract(&s);
                guide.add(s, feats, false);
            }
        } else {
            // Proyek nyata PRIMARY (48) + generator bootstrap (11) — campuran
            // mencegah starvation fitur: dengan real-only, shape generator tak
            // pernah masuk tilikan → fitur unreached (`<<`, `$clog2`, covergroup).
            // SATU per shape (0..NUM_SHAPES) — deterministik: semua fitur shape
            // ikut corpus, bukan keberuntungan sampling acak.
            for s in real_seeds {
                let feats = feature::FeatureMap::extract(&s);
                guide.add(s, feats, false);
            }
            for k in 0..gen::NUM_SHAPES {
                let s = gen.module_for_shape(&mut rng, k);
                let feats = feature::FeatureMap::extract(&s);
                guide.add(s, feats, false);
            }
            // Composed seeds — rakit dari INTERACTION GRAPH × SemanticPressure
            // (P0 scheduler/concurrency dulu). Variasi dari assembly pool, bukan
            // template: tiap campuran interaksi = modul struktur beda.
            let pressure = semantic::SemanticPressure::scheduler_directed();
            let graph = interaction::InteractionGraph::default();
            let cw = [4usize, 8].choose(&mut rng).copied().unwrap_or(4);
            for _ in 0..6 {
                let ed = graph.sample(&mut rng, &pressure);
                let s = gen.compose_from_interaction(&mut rng, cw, ed.a, ed.b);
                let feats = feature::FeatureMap::extract(&s);
                guide.add(s, feats, false);
            }
        }
    }

    // ── Backend MvMediated: guide di-seed dengan scenario `.mv` (mvgen),
    //      bukan SV generated — parent mutasi MV harus MV canonical.
    if cfg.backend == crate::testcase::Backend::MvMediated {
        for k in 0..mvgen::NUM_MV_SHAPES {
            let s = mvgen::gen_random_module(&mut rng, k);
            let feats = feature::FeatureMap::extract(&s);
            guide.add(s, feats, false);
        }
        // Bug lama (Paper #18 re-seed): source MV bila ada — HANYA yang
        // ter-parse sebagai .mv (bugdb lintas-kampanye bisa menyimpan SV dari
        // kampanye backend Direct; source SV bukan parent mutasi MV valid).
        for src in bug_db.reseed_sources() {
            if maria_api::mv::parser::parse(&src).is_err() {
                continue;
            }
            let feats = feature::FeatureMap::extract(&src);
            guide.add(src, feats, true);
        }
    }
    // Statistik & bobot adaptif palet mutasi (GAP-3).
    let mut op_stats = ast_mutate::OpStats::default();
    let mut mv_op_stats = mv_mutate::MvOpStats::default();
    // Ambang hang adaptif (temuan seed 12345 — hang flaky: run valid lambat
    // 8–15s terklasifikasi Hang saat beban sistem naik; hang > 12000 ms
    // tercatat tapi tak ter-reproduksi di re-run). Ambang efektif per iterasi
    // = max(hang_ms, EMA durasi run-OK × 16) — klasifikasi tahan terhadap
    // perlambatan GLOBAL, bukan hanya absolut (false positive timeout).
    //
    // PERFORM-1 (fix): EMA TIDAK boleh diangkat oleh outlier delta-storm.
    // Input `always @(posedge clk)`/comb-loop dengan `forever #5 clk` SEDANGKAN
    // SENDIRI (engine settle lewat delta-limit 100k, warning WR0301, selesai
    // dalam 5–7s wall) — bukan indikator beban GLOBAL. Sebelumnya EMA naik ke
    // ~7000ms → hang_ms_eff melonjak 16× ke 112s → konfirmasi hang jadi 224s
    // → kampanye melambat tanpa batas. Fix:
    //   • Hanya input yang selesai dalam `OK_BUDGET_MS` (normal, bukan
    //     delta-storm) yang memengaruhi EMA.
    //   • EMA di-CAP absolute `HANG_CAP_MS` sehingga tak pernah membengkak
    //     tak terkendali oleh outlier.
    const OK_BUDGET_MS: f64 = 3_000.0;
    const HANG_CAP_MS: u64 = 30_000;
    let mut avg_ok_ms: f64 = 0.0;
    let mut n_ok: u64 = 0;

    for iter in 0..cfg.iters {
        // ── 1. pilih parent (energy schedule) ──
        let Some(parent) = guide.select() else { break };

        // ── 1b. Backend adaptation ──
        // Direct (jalur lama, task §14): parent = SV → mutasi string → eksekusi.
        // MvMediated: parent = canonical `.mv` → mutasi AST → lower → HDL
        // eksekusi. Testcase MV (canonical + history) dilacak utk reproducibility,
        // dan bug dibawa sebagai metadata (task §8/§9).
        let mut src = parent.clone();
        let mut ops_used: Vec<usize> = Vec::new();
        let mut mv_tc: Option<crate::testcase::Testcase> = None;
        let mut mv_hdl_checked = false;
        if cfg.backend == crate::testcase::Backend::MvMediated {
            let mut mv = parent.clone();
            let mut hist: Vec<String> = Vec::new();
            let chain = rng.gen_range(1..=2);
            for _ in 0..chain {
                let (op, next) = mv_mutate::mutate(&mut rng, &mv, &mut mv_op_stats);
                hist.push(format!("mvop{op}"));
                mv = next;
            }
            match mv_lower::lower_mv(&mv, "fz_mv") {
                mv_lower::LowerVerdict::Hdl(h) => {
                    src = h;
                    mv_hdl_checked = true;
                    let cfg_fp = crate::testcase::hash_bytes(
                        &format!("{}:{}", cfg.hang_ms, cfg.max_time).into_bytes(),
                    );
                    mv_tc = Some(crate::testcase::Testcase::from_mv(
                        mv, src.clone(), cfg.seed, iter, hist, cfg_fp,
                    ));
                }
                mv_lower::LowerVerdict::HdlNoCheck(h) => {
                    src = h;
                    let cfg_fp = crate::testcase::hash_bytes(
                        &format!("{}:{}", cfg.hang_ms, cfg.max_time).into_bytes(),
                    );
                    mv_tc = Some(crate::testcase::Testcase::from_mv(
                        mv, src.clone(), cfg.seed, iter, hist, cfg_fp,
                    ));
                }
                mv_lower::LowerVerdict::MvReject { msg, .. } => {
                    // BUKAN jalan ke Maria: source ditolak maria-mv (bug di
                    // maria-mv atau fuzzer) — dicatat terpisah, bukan Panic/
                    // Hang/Differential. Pipeline TIDAK melakukan silent fallback
                    // ke direct (task acceptance).
                    report.total += 1;
                    report.compile_err += 1;
                    if let Ok(dump_path) = std::env::var("MARIA_FUZZ_DUMP") {
                        std::fs::write(&dump_path, &mv).ok();
                    }
                    let mvsnip: String = mv.lines().take(14).collect::<Vec<_>>().join("\n");
                    eprintln!(
                        "[fuzz] mv-reject iter {}: {} (seed {:#x}, {} bytes)",
                        iter,
                        msg.lines().next().unwrap_or(""),
                        cfg.seed,
                        mv.len()
                    );
                    continue;
                }
            }
        } else {
            // ── 2. rantai mutasi (1..2 op; tiap op dipilih adaptif) ──
            // GATE validitas (audit compile_err): child mutasi yang gagal
            // compile di-retry (maks 1×) dengan rantai mutasi FRESH. Bila
            // retry tetap gagal, child invalid tetap dipakai (parser-robustness).
            let chain = rng.gen_range(1..=2);
            ops_used = Vec::with_capacity(chain as usize);
            for _ in 0..chain {
                let (op, next) = ast_mutate::mutate(&mut rng, &src, &corpus, &mut op_stats);
                ops_used.push(op);
                src = next;
            }
            if !harness::compile_only_isolated(&src, cfg.hang_ms).is_ok() {
                let chain2 = rng.gen_range(1..=2);
                let mut src2 = parent.clone();
                let mut ops2: Vec<usize> = Vec::with_capacity(chain2 as usize);
                for _ in 0..chain2 {
                    let (op, next) = ast_mutate::mutate(&mut rng, &src2, &corpus, &mut op_stats);
                    ops2.push(op);
                    src2 = next;
                }
                if harness::compile_only_isolated(&src2, cfg.hang_ms).is_ok() {
                    src = src2;
                    ops_used = ops2;
                }
            }
        }

        // ── 3. bias terarah (#17) + steering CDG (#20) — HANYA backend Direct
        //      (snippet SV pada HDL hasil lower MV bisa merusak korespondensi
        //      MV canonical; perbesaran fitur pada jalur MV via mutasi AST).
        if cfg.backend == crate::testcase::Backend::Direct {
            if let Some(t) = &cfg.target {
                if !directed::is_relevant(&src, t) {
                    if let Some(b) = directed::bias_seed(&src, t) {
                        src = b;
                    }
                }
            }
            if cfg.target.is_none() && iter % 10 == 0 {
                let unreached = cdg::plan_targets(&guide.coverage());
                if let Some(t) = unreached.choose(&mut rng) {
                    if directed::is_steerable(t) && !directed::is_relevant(&src, t) {
                        if let Some(b) = directed::bias_seed(&src, t) {
                            src = b;
                        }
                    }
                }
            }
        }

        // ── 4. eksekusi terisolasi + oracle ──
        if let Ok(dump_path) = std::env::var("MARIA_FUZZ_DUMP") {
            // Debug: tulis kandidat terakhir sebelum eksekusi — bila proses
            // abort (mis. stack overflow) file ini = input penyebab.
            std::fs::write(&dump_path, &src).ok();
        }
        let hang_ms_eff = if n_ok == 0 {
            cfg.hang_ms
        } else {
            cfg.hang_ms.max((avg_ok_ms * 16.0) as u64).min(HANG_CAP_MS)
        };
        let out = if cfg.proc_isolate {
            harness::run_isolated_proc(&src, cfg.max_time, hang_ms_eff)
        } else {
            harness::run_isolated(&src, cfg.max_time, hang_ms_eff)
        };
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
                let bug = BugRecord::new(
                    BugKind::Panic,
                    minimized,
                    detail,
                    mv_tc.as_ref(),
                );
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
                // Window ×2: kelas loop-guarded yang terminate lambat (mis.
                // `for` step termutasi `i=i-1` → MAX_LOOP_ITER 100k ×2
                // re-trigger ≈ 17s, temuan korpus cva6/opentitan) butuh
                // >12s/15s; ×2 (>24s) memisahkan "lambat tapi selesai" dari
                // hang sejati (parser/infinite tak pernah settle).
                let confirm_ms = (hang_ms_eff * 2).max(15_000);
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
                let bug = BugRecord::new(
                    BugKind::Hang,
                    minimized.clone(),
                    format!(
                        "hang > {} ms ({}); minimized {} → {} bytes",
                        cfg.hang_ms,
                        out.duration_ms,
                        src.len(),
                        minimized.len()
                    ),
                    mv_tc.as_ref(),
                );
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
                                        report.bugs.push(BugRecord::new(
                                            BugKind::Differential,
                                            minimized,
                                            format!("determinism: {}; minimized {}→{} bytes",
                                                d, src.len(), mlen),
                                            mv_tc.as_ref(),
                                        ));
                                        if let Some(b) = report.bugs.last() {
                                            bug_db.push(b, cfg.seed, iter + 1);
                                        }
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
                                        report.bugs.push(BugRecord::new(
                                            BugKind::Differential,
                                            minimized,
                                            format!("emi: {}; minimized {}→{} bytes",
                                                d, src.len(), mlen),
                                            mv_tc.as_ref(),
                                        ));
                                        if let Some(b) = report.bugs.last() {
                                            bug_db.push(b, cfg.seed, iter + 1);
                                        }
                                    }
                                    differential::DiffVerdict::Skip => {}
                                }
                            }
                            // ── 6b'. oracle metamorfik-identitas (GAP-5):
                            //      `assign y = rhs` ≡ `assign y = (rhs op 0)`
                            //      utk op ∈ {+,-,|,^} — identitas eksplisit SV
                            //      4-state. Menangkap KESALAHAN SEMANTIK yang
                            //      konsisten-diri (bukan hanya inkonsistensi).
                            // Gate parse-BERSIH: source di-recovery parser
                            // (E1005 stray endtask dsb) dapat melahirkan
                            // semantik tak konsisten — bukan bukti bug engine.
                            if rng.gen_bool(0.15)
                                && maria_api::compile_diag_counts(&src).0 == 0
                            {
                                match differential::meta_identity_check(&src, cfg) {
                                    differential::DiffVerdict::Same => {}
                                    differential::DiffVerdict::Mismatch(d) => {
                                        report.meta_mismatch += 1;
                                        let minimized = corpus::Corpus::minimize(
                                            &src,
                                            &mut |cand| {
                                                matches!(harness::run_isolated(cand, cfg.max_time, cfg.hang_ms).status, RunStatus::Done)
                                                    && harness::fingerprint_isolated(cand, cfg.max_time, cfg.hang_ms).is_some()
                                                    && matches!(
                                                        differential::meta_identity_check(cand, cfg),
                                                        differential::DiffVerdict::Mismatch(_)
                                                    )
                                            },
                                        );
                                        let mlen = minimized.len();
                                        report.bugs.push(BugRecord::new(
                                            BugKind::Differential,
                                            minimized,
                                            format!("meta-identity: {}; minimized {}→{} bytes",
                                                d, src.len(), mlen),
                                            mv_tc.as_ref(),
                                        ));
                                        if let Some(b) = report.bugs.last() {
                                            bug_db.push(b, cfg.seed, iter + 1);
                                        }
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
                                    report.bugs.push(BugRecord::new(
                                        BugKind::Differential,
                                        minimized,
                                        format!("sim-sig: {}; minimized {}→{} bytes",
                                            info, src.len(), mlen),
                                        mv_tc.as_ref(),
                                    ));
                                    if let Some(b) = report.bugs.last() {
                                        bug_db.push(b, cfg.seed, iter + 1);
                                    }
                                }
                            }
                        // ── 6d. property-oracle (#2/#3/#14, oracle #5):
                            //      dua temp mirror (`_fz_rtA`/`_fz_rtB`) yang
                            //      mengevaluasi ekspresi SAMA memberi hasil
                            //      beda → `_fz_viol = 1` = bug evaluasi.
                            if mirror_intact(&src) && maria_api::compile_diag_counts(&src).0 == 0 {
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
                                    report.bugs.push(BugRecord::new(
                                        BugKind::Differential,
                                        minimized,
                                        format!(
                                            "property: {}; minimized {}→{} bytes",
                                            viol,
                                            src.len(),
                                            mlen
                                        ),
                                        mv_tc.as_ref(),
                                    ));
                                    if let Some(b) = report.bugs.last() {
                                        bug_db.push(b, cfg.seed, iter + 1);
                                    }
                                }
                            }
                        } else {
                            report.sim_err += 1;
                            // ── Reklasifikasi (agresif): runtime error pada
                            // input PARSE-CLEAN = kandidat bug engine, KECUALI
                            // kelas legit-LRM (null-handle/mailbox/assert/delta-
                            // storm/timeout = perilaku SV benar). RT yang keluar
                            // di input valid = engine terlalu ketat / salah kode.
                            // Sebelumnya semua sim_err dibuang → "jarang ketemu
                            // error runtime" (padahal ada).
                            let legit_runtime = [
                                "RT0001", "RT0002", "RT0003", "RT0005", "RT2001",
                                "RT2003", "RT3001", "RT7001", "RT7002", "RT7003",
                            ];
                            let clean_src = maria_api::compile_diag_counts(&src).0 == 0
                                && !ast_mutate::has_assert_oracle(&src);
                            let rt_is_suspicious = sim
                                .code
                                .starts_with("RT")
                                && !legit_runtime
                                    .iter()
                                    .any(|c| sim.code.contains(c));
                            if clean_src && rt_is_suspicious {
                                report.sim_err_clean += 1;
                                let minimized = corpus::Corpus::minimize(
                                    &src,
                                    &mut |cand| {
                                        maria_api::compile_diag_counts(cand).0 == 0
                                            && !ast_mutate::has_assert_oracle(cand)
                                            && harness::sim_err_isolated(
                                                cand,
                                                cfg.max_time,
                                                cfg.hang_ms,
                                            )
                                            .map(|c| {
                                                c.starts_with("RT")
                                                    && !legit_runtime
                                                        .iter()
                                                        .any(|x| c.contains(x))
                                            })
                                            .unwrap_or(false)
                                    },
                                );
                                let mlen = minimized.len();
                                report.bugs.push(BugRecord::new(
                                    BugKind::Differential,
                                    minimized,
                                    format!(
                                        "runtime-error pada input parse-clean: {}{};\
                                         minimized {}→{} bytes",
                                        sim.code,
                                        sim.message,
                                        src.len(),
                                        mlen
                                    ),
                                    mv_tc.as_ref(),
                                ));
                                if let Some(b) = report.bugs.last() {
                                    bug_db.push(b, cfg.seed, iter + 1);
                                }
                            }
                            // Property-oracle assert (Paper #14/#2/#3, oracle #5):
                            // seed memuat assert-oracle (`_fz_atA/_fz_atB` eval ekspresi
                            // identik). Sim error = assertion FAIL = bug evaluasi.
                            // HANYA code RT7001 (dari `assert ... else $fatal`) yang
                            // menandakan assertion violate. sim_err lain (mis. RT0001
                            // hier-signal not found dari seed malformed yang punya
                            // referensi tak-resolved, RT2001 delta-storm) BUKAN bug
                            // engine — jangan salah-klaim.
                            if maria_api::compile_diag_counts(&src).0 == 0
                                && sim.code.contains("RT7001")
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
                                report.bugs.push(BugRecord::new(
                                    BugKind::Differential,
                                    minimized,
                                    format!(
                                        "assert-oracle: {}{} — ekspresi identik dievaluasi beda; minimized {}→{} bytes",
                                        sim.code, sim.message, src.len(), mlen
                                    ),
                                    mv_tc.as_ref(),
                                ));
                                if let Some(b) = report.bugs.last() {
                                    bug_db.push(b, cfg.seed, iter + 1);
                                }
                            }
                        }
                    }
                } else {
                    report.compile_err += 1;
                    // ── Triase compile-err (task tambahan: perbaiki error
                    // compile maria) — MV yang LOLOS type-check maria-mv
                    // (LowerVerdict::Hdl) tapi HDL-nya DITOLAK maria =
                    // BUG lowering maria-mv atau parser/elab maria → catat
                    // sebagai bug kandidat (bukan buang diam).
                    if cfg.backend == crate::testcase::Backend::MvMediated && mv_hdl_checked {
                        report.lower_good_compile_err += 1;
                        if let Some(tc) = &mv_tc {
                            let v = harness::compile_only_isolated(&tc.hdl, cfg.hang_ms);
                            let code = match v {
                                Ok(m) => m.lines().next().unwrap_or("").chars().take(40).collect::<String>(),
                                Err(e) => format!("panic:{e}"),
                            };
                            std::fs::create_dir_all("/tmp/opencode/mv_compile_triage").ok();
                            let _ = std::fs::write(
                                format!("/tmp/opencode/mv_compile_triage/iter{}.mv", iter),
                                &tc.mv,
                            );
                            let _ = std::fs::write(
                                format!("/tmp/opencode/mv_compile_triage/iter{}.sv", iter),
                                &tc.hdl,
                            );
                            eprintln!(
                                "[fuzz] compile-err hdl-ok iter {} seed {:#x} code: {}",
                                iter, cfg.seed, code
                            );
                        }
                    }
                }
            }
        }

        // EMA durasi run-OK — umpan ambang hang adaptif (lihat di atas).
        // PERFORM-1: HANYA input yang selesai dalam OK_BUDGET_MS (bukan
        // delta-storm / comb-loop yang settle 5–7s) yang meng-update EMA.
        // Input lambat-tapi-selesai (delta-storm) TIDAK mengangkat threshold
        // → hang_sejati tetap terdeteksi cepat, kampanye tidak melambat.
        if matches!(&out.status, RunStatus::Done) && out.compile.ok && out.duration_ms < OK_BUDGET_MS as u64 {
            let d = out.duration_ms as f64;
            avg_ok_ms = if n_ok == 0 { d } else { 0.9 * avg_ok_ms + 0.1 * d };
            n_ok += 1;
        }

        /// Oracle referensi (LRM proxy) — jalankan SEBAGIAN iterasi: bangun core
    /// pasif fresh (dari generator, bukan mutasi — tb eksternal drive tanpa
    /// konflik wire), bandingkan trace maria vs iverilog. Mismatch = deviasi
    /// semantik (buta bagi oracle konsistensi-diri).
    if cfg.ref_diff && rng.gen_bool(0.30) {
        let w = gen::WIDTHS.choose(&mut rng).copied().unwrap_or(8);
        let core = gen.passive_core(&mut rng, w);
        if !oracle::has_nondeterministic_src(&core) {
            match differential::reference_vs_ivl(&core, cfg) {
                differential::DiffVerdict::Same => {}
                differential::DiffVerdict::Mismatch(d) => {
                    report.ref_mismatch += 1;
                    let minimized = corpus::Corpus::minimize(&core, &mut |cand| {
                        matches!(
                            differential::reference_vs_ivl(cand, cfg),
                            differential::DiffVerdict::Mismatch(_)
                        )
                    });
                    let mlen = minimized.len();
                    report.bugs.push(BugRecord::new(
                        BugKind::Differential,
                        minimized,
                        format!(
                            "reference(iverilog): {}; minimized {}→{} bytes",
                            d,
                            core.len(),
                            mlen
                        ),
                        mv_tc.as_ref(),
                    ));
                    if let Some(b) = report.bugs.last() {
                        bug_db.push(b, cfg.seed, iter + 1);
                    }
                }
                differential::DiffVerdict::Skip => {}
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
                // Backend MvMediated: simpan parent MV CANONICAL (testcase.mv),
                // BUKAN HDL hasil lower — HDL sebagai parent .mv = reject
                // (pipeline korpus MV murni; task: fuzzing di level MV).
                let seed_for_corpus = match &mv_tc {
                    Some(t) => t.mv.clone(),
                    None => src.clone(),
                };
                guide.add(seed_for_corpus, exec_feats, false);
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
    report.mv_op_stats = mv_op_stats;

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

    // Auto-simpan bug source (.sv) — prinsip "0 file saat normal, file
    // HANYA saat bug/error baru": bila `--emit-bugs` tidak diberikan tapi
    // ada bug, tulis ke `.maria-fuzz-bugs/` (bukan polusi tanpa bug).
    if let Some(dir) = &cfg.emit_dir {
        emit_bugs(dir, &report);
    } else if !report.bugs.is_empty() {
        emit_bugs(std::path::Path::new(".maria-fuzz-bugs"), &report);
    }

    // Persist corpus menarik (GAP-10): parent aktif/bug → seed kampanye
    // berikutnya via --corpus-dir (evolusi lintas kampanye).
    if let Some(dir) = &cfg.save_corpus_dir {
        if let Err(e) = guide.persist_to(dir, 500) {
            eprintln!("[fuzz] warning: gagal simpan corpus: {}", e);
        }
    }

    report
}

/// Kumpulkan path file `.sv`/`.v` rekursif (untuk real-project hunt).
fn collect_sv_paths(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_sv_paths(&p, out);
        } else if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
            if ext == "sv" || ext == "v" {
                out.push(p);
            }
        }
    }
}

/// Tulis bug ke direktori (1 file per bug) — dev-reporting, bukan edit kode.
/// Backend MvMediated: tulis BOTH canonical `.mv` dan HDL `.sv` (task §8:
/// simpan MV DSL testcase + generated HDL; `.mv` = reproducible sumber).
fn emit_bugs(dir: &std::path::Path, report: &FuzzReport) {
    let _ = std::fs::create_dir_all(dir);
    for (i, b) in report.bugs.iter().enumerate() {
        let kind = match b.kind {
            BugKind::Panic => "panic",
            BugKind::Hang => "hang",
            BugKind::Differential => "diff",
        };
        let base = dir.join(format!("bug_{:04}_{}", i, kind));
        let _ = std::fs::write(format!("{}.sv", base.display()), &b.source);
        if let Some(mv) = &b.mv {
            let _ = std::fs::write(format!("{}.mv", base.display()), mv);
        }
        if let Some(hdl) = &b.hdl {
            let _ = std::fs::write(format!("{}.hdl.sv", base.display()), hdl);
        }
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