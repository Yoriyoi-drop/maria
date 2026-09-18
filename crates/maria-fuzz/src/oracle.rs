//! Oracle bug detection (O1-O5 style, diadaptasi untuk maria).

use crate::{CaseResult, Category, Oracle, Target};

use maria_api::{
    simulate_signals_with_flags_quiet, simulate_signals_with_trace_quiet, EngineFlags,
};

/// Evaluasi satu source terhadap target.
///
/// Pendekatan: in-process melalui maria-api (bukan subprocess), karena API
/// sudah menyediakan jalur quiet + differential. Panic/hang tetap ditangkap
/// via runner subprocess untuk target yang butuh izin eksekusi penuh.
pub fn evaluate(target: Target, source: &str, timeout_ms: u64) -> CaseResult {
    match target {
        Target::Lexer => evaluate_lexer(source),
        Target::Vcd => evaluate_vcd(source, timeout_ms),
        Target::Parser | Target::Elaborator => evaluate_compile(target, source, timeout_ms),
        Target::Simulator => evaluate_sim(source, timeout_ms),
        Target::Fmt => evaluate_fmt(source),
        Target::Cli => evaluate_cli(source, timeout_ms),
        Target::Preproc => evaluate_preproc(source),
        Target::Mv => evaluate_mv(source),
        Target::Sdf => evaluate_sdf(source, timeout_ms),
        Target::Micd => evaluate_micd(source, timeout_ms),
        Target::All => unreachable!("Target::All dipecah di run()"),
    }
}

fn mk(
    target: Target,
    oracle: Oracle,
    category: Category,
    detail: &str,
    source: &str,
) -> CaseResult {
    CaseResult {
        target,
        category,
        oracle: oracle.as_str(),
        detail: detail.to_string(),
        source: source.to_string(),
    }
}

/// Isolasi MICD DB per-case: `MARIA_MICD_DIR` → temp unik. Subprocess maria
/// (sim/sdf/cli/vcd/micd) TIDAK menyentuh `.maria/database` project →
/// bebas lock contention + stale-lock cascade dari subprocess yang di-kill
/// (terukur: 39 lock basi + hang palsu beruntun di kampanye SDF).
fn with_micd_isolated<T>(f: impl FnOnce() -> T) -> T {
    use std::ffi::OsString;
    let dir = std::env::temp_dir().join(format!(
        "mariafz_iso_{}_{}",
        std::process::id(),
        crate::next_crash_seq()
    ));
    let _ = std::fs::create_dir_all(&dir);
    let prev: Option<OsString> = std::env::var_os("MARIA_MICD_DIR");
    std::env::set_var("MARIA_MICD_DIR", &dir);
    let r = f();
    match prev {
        Some(p) => std::env::set_var("MARIA_MICD_DIR", p),
        None => std::env::remove_var("MARIA_MICD_DIR"),
    }
    let _ = std::fs::remove_dir_all(&dir);
    r
}

/// Error GLOBAL tanpa lokasi tunggal (by design) — bukan diag_missing.
fn is_global_error(msg: &str) -> bool {
    let pats = [
        "Unable to determine top-level design",
        "Top resolution failed",
        "no modules found",
        "No modules found",
        "no valid top",
        "Recovery mode enabled",
    ];
    pats.iter().any(|p| msg.contains(p))
}

/// O1 + O2 untuk compile pipeline (lexer/parser/elaborator) + deteksi HANG.
///
/// Dijalankan dalam thread stack BESAR (256MB) — parser rekursif pada
/// expression dalam hasil mutasi (contoh: `x0|x1|...` rantai panjang) bisa
/// overflow stack default 8MB (ditemukan fuzzer seed 7: stack overflow di
/// parser). Konsisten dgn simulate_in_thread.
///
/// FILEZERO (fuzzer gap): sebelumnya TANPA watchdog — input blowup
/// (mutasi duplicate chunk → seed 13MB, 537 module) membuat compile
/// super-lambat/freeze dan kampanye macet total tanpa deteksi. Sekarang
/// watchdog `recv_timeout` → Hang terdeteksi, worker thread dibiarkan
/// selesai di background (tidak bisa di-kill di Rust) — kampanye lanjut.
fn evaluate_compile(target: Target, source: &str, timeout_ms: u64) -> CaseResult {
    use std::time::Duration;
    let source_owned = source.to_string();
    let (tx, rx) = std::sync::mpsc::channel::<CaseResult>();
    let spawned = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .name("maria-fuzz-compile".into())
        .spawn(move || {
            let r = compile_in_thread(target, &source_owned);
            let _ = tx.send(r);
        });
    if spawned.is_err() {
        return mk(
            target,
            Oracle::O1NoCrash,
            Category::Panic,
            "thread compile gagal spawn",
            source,
        );
    }
    match rx.recv_timeout(Duration::from_millis(timeout_ms)) {
        Ok(r) => r,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => mk(
            target,
            Oracle::O1NoCrash,
            Category::Hang,
            &format!("compile hang/slow > {} ms (worker dilanjutkan di background)", timeout_ms),
            source,
        ),
        Err(_) => mk(
            target,
            Oracle::O1NoCrash,
            Category::Panic,
            "thread compile disconnected",
            source,
        ),
    }
}

fn compile_in_thread(target: Target, source: &str) -> CaseResult {
    // O1: no-crash — panic di compile = bug
    let caught = std::panic::catch_unwind(|| maria_api::compile_str_quiet(source));
    let ir_design = match caught {
        Ok(Ok(ir)) => ir,
        Ok(Err(err)) => {
            // Error bersih — O2: cek lokasi diagnostik.
            // LANGKAH ANALISA: error top-level/multi-modul tanpa top (E3006/
            // EL3001) TIDAK = source salah — seed RTL murni valid. Coba mode
            // recovery (AnalysisRecovery): kalau berhasil → Ok (analisis).
            let msg = err.to_string();
            if is_global_error(&msg) {
                let recovered = std::panic::catch_unwind(|| maria_api::compile_str_analyze(source));
                if let Ok(Ok(_ir)) = recovered {
                    return mk(
                        target,
                        Oracle::O1NoCrash,
                        Category::Ok,
                        &format!("sim ok — analysis recovery (multi-modul tanpa top): {msg}"),
                        source,
                    );
                }
                return mk(
                    target,
                    Oracle::O1NoCrash,
                    Category::CleanError,
                    &msg,
                    source,
                );
            }
            let has_loc = extract_loc(&msg);
            if !has_loc {
                return mk(
                    target,
                    Oracle::O2DiagLocation,
                    Category::DiagMissing,
                    &format!("diagnostik tanpa lokasi: {msg}"),
                    source,
                );
            }
            return mk(
                target,
                Oracle::O1NoCrash,
                Category::CleanError,
                &msg,
                source,
            );
        }
        Err(_) => {
            return mk(
                target,
                Oracle::O1NoCrash,
                Category::Panic,
                "panic saat compile_str_quiet",
                source,
            );
        }
    };

    // Compile sukses — determinism check (O4): kompilasi ulang sama
    let second = std::panic::catch_unwind(|| maria_api::compile_str_quiet(source));
    match second {
        Ok(Ok(ir2)) => {
            let diffs = maria_api::compare_asts(&ir_design, &ir2);
            if !diffs.is_empty() {
                return mk(
                    target,
                    Oracle::O4Determinism,
                    Category::NonDeterministic,
                    &format!("AST berbeda antar run ({} diffs)", diffs.len()),
                    source,
                );
            }
            mk(
                target,
                Oracle::O1NoCrash,
                Category::Ok,
                "compile ok",
                source,
            )
        }
        _ => mk(
            target,
            Oracle::O4Determinism,
            Category::NonDeterministic,
            "compile kedua panic/error — non-deterministik",
            source,
        ),
    }
}

/// Fuzzing LEXER khusus (area belum tersentuh — target Lexer lama memakai
/// pipeline compile yang sama, bukan oracle token):
/// - lex dua kali segar → stream token (kind+line+col) harus identik
///   (non-determinisme token = bug)
/// - panic saat lex input gila = bug
fn evaluate_lexer(source: &str) -> CaseResult {
    use maria_parser::lexer::Lexer;
    let lex_once = || -> Vec<(String, usize, usize)> {
        let mut lx = Lexer::new(source);
        let mut toks = Vec::new();
        loop {
            let (tok, line, col) = lx.next_token();
            if tok == maria_parser::lexer::Token::Eof {
                break;
            }
            toks.push((format!("{:?}", tok), line, col));
        }
        toks
    };
    let r1 = std::panic::catch_unwind(lex_once);
    let r2 = std::panic::catch_unwind(lex_once);
    match (r1, r2) {
        (Ok(t1), Ok(t2)) => {
            if t1 != t2 {
                return mk(
                    Target::Lexer,
                    Oracle::O4Determinism,
                    Category::NonDeterministic,
                    &format!(
                        "lexer non-deterministik: dua lex identik hasil beda ({} vs {} tokens)",
                        t1.len(),
                        t2.len()
                    ),
                    source,
                );
            }
            mk(
                Target::Lexer,
                Oracle::O1NoCrash,
                Category::Ok,
                &format!("lexer ok ({} token)", t1.len()),
                source,
            )
        }
        (Err(_), _) | (_, Err(_)) => mk(
            Target::Lexer,
            Oracle::O1NoCrash,
            Category::Panic,
            "panic saat lex",
            source,
        ),
    }
}

/// O1 + O4 + O5 untuk simulator — verifikasi lengkap, bukan sekadar jalan.
///
/// Alur verifikasi:
/// 1. Subprocess check (O1): crash/abort/hang via binary maria black-box.
/// 2. In-process compile (O1): pastikan source compile tanpa panic.
/// 3. O4 determinism: jalankan 2× dengan flags sama → signal harus identik.
/// 4. O5 differential: jalankan default vs semua jalur alternatif → signal harus identik.
/// 5. Trace consistency: final state dan trace harus berhasil diambil serta konsisten.
/// 6. Assertion detection: deteksi assertion/violation di output.
fn evaluate_sim(source: &str, timeout_ms: u64) -> CaseResult {
    // ── 1. Subprocess check (O1) ──
    let outcome = with_micd_isolated(|| crate::runner::run_file(source, timeout_ms));
    match outcome.kind {
        crate::runner::Kind::CleanError => {
            return mk(
                Target::Simulator,
                Oracle::O1NoCrash,
                Category::CleanError,
                &outcome.stderr,
                source,
            );
        }
        crate::runner::Kind::Panic => {
            return mk(
                Target::Simulator,
                Oracle::O1NoCrash,
                Category::Panic,
                &outcome.stderr,
                source,
            );
        }
        crate::runner::Kind::Abort => {
            return mk(
                Target::Simulator,
                Oracle::O1NoCrash,
                Category::Abort,
                &outcome.stderr,
                source,
            );
        }
        crate::runner::Kind::Crash(code) => {
            return mk(
                Target::Simulator,
                Oracle::O1NoCrash,
                Category::Panic,
                &format!("crash code {code}: {}", outcome.stderr),
                source,
            );
        }
        crate::runner::Kind::Hang => {
            return mk(
                Target::Simulator,
                Oracle::O1NoCrash,
                Category::Hang,
                &format!("hang > {} ms", timeout_ms),
                source,
            );
        }
        crate::runner::Kind::Ok => {
            // Subprocess ok — deteksi assertion/violation di output
            if let Some(violation) = detect_violation(&outcome.stderr) {
                return mk(
                    Target::Simulator,
                    Oracle::O1NoCrash,
                    Category::GuardBypass,
                    &violation,
                    source,
                );
            }
            if let Some(violation) = detect_violation(&outcome.stdout) {
                return mk(
                    Target::Simulator,
                    Oracle::O1NoCrash,
                    Category::GuardBypass,
                    &violation,
                    source,
                );
            }
        }
    }

    // ── 2-6. In-process compile/determinism/differential/trace/evidence
    //         dijalankan dalam thread stack BESAR (256MB, konsisten dgn
    //         main.rs yang membungkus sim dgn stack besar) — evaluator
    //         rekursif pada chain BinaryOp panjang (`x0|x1|...|x63` di
    //         OpenC910 ct_rtu_encode_64) overflow stack default 8MB
    //         ("thread 'main' has overflowed its stack", ditemukan fuzzer).
    let source_owned = source.to_string();
    let t_ms = timeout_ms;
    let result: Option<CaseResult> = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .name("maria-fuzz-sim".into())
        .spawn(move || simulate_in_thread(&source_owned, t_ms))
        .ok()
        .and_then(|h| h.join().ok())
        .flatten();
    if let Some(r) = result {
        return r;
    }
    mk(
        Target::Simulator,
        Oracle::O1NoCrash,
        Category::Panic,
        "thread sim gagal (join err)",
        source,
    )
}

/// Jalankan seluruh validasi sim in-process dalam thread stack besar.
fn simulate_in_thread(source: &str, timeout_ms: u64) -> Option<CaseResult> {
    let mk_r = |c: Category, o: Oracle, d: &str| Some(mk(Target::Simulator, o, c, d, source));
    // ── 2. In-process compile check (O1) ──
    let compile_result = std::panic::catch_unwind(|| maria_api::compile_str_quiet(source));
    match compile_result {
        Ok(Err(e)) => {
            return mk_r(
                Category::CleanError,
                Oracle::O1NoCrash,
                &format!("compile gagal: {e}"),
            );
        }
        Err(_) => {
            return mk_r(
                Category::Panic,
                Oracle::O1NoCrash,
                "panic saat compile in-process",
            );
        }
        _ => {} // compile ok
    }

    // ── 3. O4 determinism: jalankan 2× dengan flags sama ──
    let default_flags = EngineFlags {
        use_packed_eval: false,
        use_dag_parallel: false,
        use_timing_wheel: false,
        use_mir_jit: false,
    };

    let r1 = std::panic::catch_unwind(|| {
        simulate_signals_with_flags_quiet(source, 1_000, &default_flags)
    });
    let r2 = std::panic::catch_unwind(|| {
        simulate_signals_with_flags_quiet(source, 1_000, &default_flags)
    });

    match (&r1, &r2) {
        (Ok(Ok(sigs1)), Ok(Ok(sigs2))) => {
            // Compare sebagai MAP (nama→nilai) — urutan Vec bisa beda antar
            // run (iterasi HashMap) walau isi sama → false nondeterminism.
            let map1: std::collections::BTreeMap<&str, &maria_ir::LogicVec> =
                sigs1.iter().map(|(n, v)| (n.as_str(), v)).collect();
            let map2: std::collections::BTreeMap<&str, &maria_ir::LogicVec> =
                sigs2.iter().map(|(n, v)| (n.as_str(), v)).collect();
            if map1 != map2 {
                return mk_r(
                    Category::NonDeterministic,
                    Oracle::O4Determinism,
                    &format!(
                        "sim non-deterministik: {} != {} signal values (2 run identik)",
                        signal_summary(sigs1),
                        signal_summary(sigs2)
                    ),
                );
            }
        }
        (Ok(Err(e1)), Ok(Err(_e2))) => {
            // Keduanya error — error deterministik, bukan bug
            return mk_r(
                Category::CleanError,
                Oracle::O1NoCrash,
                &format!("sim error deterministik: {e1}"),
            );
        }
        (Ok(Err(e)), _) | (_, Ok(Err(e))) => {
            // Salah satu error, satu ok — non-deterministik error
            return mk_r(
                Category::NonDeterministic,
                Oracle::O4Determinism,
                &format!("sim error non-deterministik: {e}"),
            );
        }
        (Err(_), _) | (_, Err(_)) => {
            return mk_r(
                Category::NonDeterministic,
                Oracle::O4Determinism,
                "sim panic tidak deterministik",
            );
        }
    }

    // Ambil hasil valid dari run pertama
    let sigs_default = match &r1 {
        Ok(Ok(s)) => s.clone(),
        _ => unreachable!("sudah di-handle di atas"),
    };

    // ── 4. O5 differential: semua jalur engine yang tersedia ──
    // Hasil akhir yang sama pada satu jalur belum membuktikan semantik benar.
    // Jalur packed, DAG, timing-wheel, dan MIR JIT memberi implementasi
    // alternatif untuk menemukan bug yang konsisten pada jalur default.
    let alternate_flags = [
        (
            "packed",
            EngineFlags {
                use_packed_eval: true,
                ..default_flags
            },
        ),
        (
            "dag",
            EngineFlags {
                use_dag_parallel: true,
                ..default_flags
            },
        ),
        (
            "timing-wheel",
            EngineFlags {
                use_timing_wheel: true,
                ..default_flags
            },
        ),
        (
            "mir-jit",
            EngineFlags {
                use_mir_jit: true,
                ..default_flags
            },
        ),
        // ── Kombinasi (interaksi flag) — validate.rs sudah definisikan 8 jalur
        // tapi oracle lama hanya bandingkan single-flag → kombinasi
        // packed+dag/packed+timing/dag+timing TIDAK pernah di-fuzz. Tambah
        // sekarang: interaksi antar jalur engine bisa memicu race/ordering
        // yang tidak terlihat pada jalur tunggal.
        (
            "packed+dag",
            EngineFlags {
                use_packed_eval: true,
                use_dag_parallel: true,
                ..default_flags
            },
        ),
        (
            "packed+timing",
            EngineFlags {
                use_packed_eval: true,
                use_timing_wheel: true,
                ..default_flags
            },
        ),
        (
            "dag+timing",
            EngineFlags {
                use_dag_parallel: true,
                use_timing_wheel: true,
                ..default_flags
            },
        ),
    ];
    for (name, flags) in alternate_flags {
        let result =
            std::panic::catch_unwind(|| simulate_signals_with_flags_quiet(source, 1_000, &flags));
        match result {
            Ok(Ok(signals))
                if {
                    // Compare map (urutan-insensitive) — urutan Vec bisa beda
                    // antar jalur engine walau isi sama.
                    let m_def: std::collections::BTreeMap<&str, &maria_ir::LogicVec> =
                        sigs_default.iter().map(|(n, v)| (n.as_str(), v)).collect();
                    let m_alt: std::collections::BTreeMap<&str, &maria_ir::LogicVec> =
                        signals.iter().map(|(n, v)| (n.as_str(), v)).collect();
                    m_def == m_alt
                } => {}
            Ok(Ok(signals)) => {
                return mk_r(
                    Category::Differential,
                    Oracle::O5Differential,
                    &format!(
                        "differential default vs {name}: {} != {}",
                        signal_summary(&sigs_default),
                        signal_summary(&signals)
                    ),
                );
            }
            Ok(Err(e)) => {
                return mk_r(
                    Category::Differential,
                    Oracle::O5Differential,
                    &format!("{name} path error, default ok: {e}"),
                );
            }
            Err(_) => {
                return mk_r(
                    Category::Differential,
                    Oracle::O5Differential,
                    &format!("{name} path panic, default ok"),
                );
            }
        }
    }

    // ── 5. Trace quality: pastikan trace punya data bermakna ──
    let trace_result =
        std::panic::catch_unwind(|| simulate_signals_with_trace_quiet(source, 1_000, 100));

    match trace_result {
        Ok(Ok((sigs_trace, trace))) => {
            if trace.is_empty() {
                return mk_r(
                    Category::Ok,
                    Oracle::O1NoCrash,
                    &format!("sim ok, no trace ({} signals)", sigs_trace.len()),
                );
            }

            // Trace ada — cek kualitas: minimal ada 1 trace entry yang bukan empty string
            let meaningful_traces: Vec<&String> = trace
                .iter()
                .filter(|t| !t.is_empty() && !t.trim().is_empty())
                .collect();
            if meaningful_traces.is_empty() {
                return mk_r(
                    Category::Ok,
                    Oracle::O1NoCrash,
                    &format!(
                        "sim ok, trace kosong ({} entries, {} signals)",
                        trace.len(),
                        sigs_trace.len()
                    ),
                );
            }

            let m_tr: std::collections::BTreeMap<&str, &maria_ir::LogicVec> =
                sigs_trace.iter().map(|(n, v)| (n.as_str(), v)).collect();
            let m_d: std::collections::BTreeMap<&str, &maria_ir::LogicVec> =
                sigs_default.iter().map(|(n, v)| (n.as_str(), v)).collect();
            if m_tr != m_d {
                return mk_r(
                    Category::Differential,
                    Oracle::O5Differential,
                    &format!(
                        "trace final state berbeda: {} != {}",
                        signal_summary(&sigs_default),
                        signal_summary(&sigs_trace)
                    ),
                );
            }

            let ev = std::panic::catch_unwind(|| crate::validate::evidence_only(source, 1_000))
                .unwrap_or_default();

            // ── 7. External reference (Icarus) — bukti KEBENARAN sim ──
            // Opsional via env MARIA_FUZZ_ICARUS=1: jalankan source juga di
            // iverilog+vvp (reference eksternal independen). Mismatch marker
            // = semantic divergence NYATA (hasil maria != reference).
            if std::env::var("MARIA_FUZZ_ICARUS")
                .map(|v| v == "1")
                .unwrap_or(false)
            {
                let ic = crate::oracle_icarus::evaluate_icarus(source, timeout_ms);
                match ic.verdict {
                    crate::oracle_icarus::Verdict::Mismatch => {
                        return mk_r(Category::Differential, Oracle::O5Differential, &ic.detail);
                    }
                    crate::oracle_icarus::Verdict::MariaBug => {
                        return mk_r(Category::Panic, Oracle::O1NoCrash, &ic.detail);
                    }
                    crate::oracle_icarus::Verdict::Match => {
                        return mk_r(
                            Category::Ok,
                            Oracle::O5Differential,
                            &format!(
                                "sim VERIFIED vs Icarus reference ({}) — hasil identik",
                                ic.detail
                            ),
                        );
                    }
                    crate::oracle_icarus::Verdict::RefUnavailable => {
                        // reference N/A (iverilog tak bisa compile source ini)
                        // — jatuh ke audit X/stimulus di bawah.
                    }
                }
            }

            // ── 8. X/stimulus audit — bagan verdict: pasif vs suspicious ──
            let is_passive = ev.process_count == 0;
            let x_heavy = ev.signal_count > 0
                && (ev.x_remain + ev.z_remain) * 100 >= ev.signal_count.max(1) * 60;

            let detail = format!(
                "sim verified: {} signals, {} trace entries ({} meaningful); \
                 default/packed/DAG/timing-wheel/MIR-JIT konsisten; [evidence] {}",
                sigs_trace.len(),
                trace.len(),
                meaningful_traces.len(),
                ev.summary(),
            );

            if is_passive {
                // Design TANPA blok prosedural — X/Z wajar (undriven).
                return mk_r(
                    Category::Ok,
                    Oracle::O1NoCrash,
                    &format!("{detail} — design pasif (0 proses), X/Z wajar"),
                );
            }
            if x_heavy {
                // Stimulus ada tapi signal dominan X → butuh perhatian:
                // bisa wajar (X-latch) atau bukti state-propagation bug.
                return mk_r(
                    Category::Suspicious,
                    Oracle::O1NoCrash,
                    &format!(
                        "{detail} — stimulus ada ({}) namun X/Z dominan ({}/{})",
                        ev.process_count,
                        ev.x_remain + ev.z_remain,
                        ev.signal_count
                    ),
                );
            }

            mk_r(Category::Ok, Oracle::O1NoCrash, &detail)
        }
        Ok(Err(e)) => {
            return mk_r(
                Category::Differential,
                Oracle::O5Differential,
                &format!("trace gagal setelah sim default sukses: {e}"),
            );
        }
        Err(_) => {
            return mk_r(
                Category::Differential,
                Oracle::O5Differential,
                "trace panic setelah sim default sukses",
            );
        }
    }
}

/// Cek apakah semua bit dalam LogicVec bernilai Zero.
fn is_all_zero(lv: &maria_ir::LogicVec) -> bool {
    lv.bits.iter().all(|b| *b == maria_ir::LogicVal::Zero)
}

/// Buat ringkasan signal untuk pesan error (nama + nilai non-zero).
fn signal_summary(sigs: &[(String, maria_ir::LogicVec)]) -> String {
    if sigs.is_empty() {
        return "0 signals".to_string();
    }
    let nonzero: Vec<String> = sigs
        .iter()
        .filter(|(_, lv)| !is_all_zero(lv))
        .map(|(name, lv)| format!("{}={}", name, lv.to_u64()))
        .take(5)
        .collect();
    if nonzero.is_empty() {
        format!("{} signals (all zero)", sigs.len())
    } else {
        format!(
            "{} signals ({} nonzero: {})",
            sigs.len(),
            nonzero.len(),
            nonzero.join(", ")
        )
    }
}

/// Deteksi assertion/violation di output (stderr/stdout).
fn detect_violation(output: &str) -> Option<String> {
    let violation_pats = [
        ("Assertion failed", "assertion failed"),
        ("assertion violation", "assertion violation"),
        ("$fatal", "$fatal hit"),
    ];
    for (pat, desc) in violation_pats {
        if let Some(line) = output.lines().find(|l| l.contains(pat)) {
            return Some(format!("{desc}: {}", line.trim()));
        }
    }
    None
}

/// O3: fmt round-trip (jika tool fmt tersedia via maria_api::tools).
fn evaluate_fmt(source: &str) -> CaseResult {
    // Fmt target dijalankan in-process — round-trip check. Thread stack besar
    // (lexer/parser rekursif bisa overflow pada input mutasi — ditemukan fuzzer
    // seed 7 stack overflow di fmt ~kasus 500, sama dgn compile/sim).
    let source_owned = source.to_string();
    let result: Option<CaseResult> = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .name("maria-fuzz-fmt".into())
        .spawn(move || fmt_in_thread(&source_owned))
        .ok()
        .and_then(|h| h.join().ok());
    match result {
        Some(r) => r,
        None => mk(
            Target::Fmt,
            Oracle::O1NoCrash,
            Category::Panic,
            "thread fmt gagal (join err)",
            source,
        ),
    }
}

fn fmt_in_thread(source: &str) -> CaseResult {
    let caught = std::panic::catch_unwind(|| fmt_roundtrip(source));
    match caught {
        Ok(Ok(())) => mk(
            Target::Fmt,
            Oracle::O3Roundtrip,
            Category::Ok,
            "fmt ok",
            source,
        ),
        Ok(Err(e)) => match e.category {
            Category::RoundtripMismatch => mk(
                Target::Fmt,
                Oracle::O3Roundtrip,
                Category::RoundtripMismatch,
                &e.detail,
                source,
            ),
            Category::CleanError => mk(
                Target::Fmt,
                Oracle::O1NoCrash,
                Category::CleanError,
                &e.detail,
                source,
            ),
            _ => mk(
                Target::Fmt,
                Oracle::O1NoCrash,
                Category::Panic,
                &e.detail,
                source,
            ),
        },
        Err(_) => mk(
            Target::Fmt,
            Oracle::O3Roundtrip,
            Category::Panic,
            "panic di fmt",
            source,
        ),
    }
}

struct FmtError {
    category: Category,
    detail: String,
}

/// fmt(fmt(s)) == fmt(s)? Idempotensi.
///
/// Jangan skip hasil yang tidak parseable — itu PELUANG EMAS: jika INPUT
/// valid tapi output fmt rusak, formatter merusak source (bug nyata).
fn fmt_roundtrip(source: &str) -> Result<(), FmtError> {
    // mfmt berbasis lexer MURNI (tanpa preprocessing). Source dengan directive
    // backtick (`` `define ``/`` `include ``/`` `ifdef ``) — apalagi inline di
    // tengah literal — di luar kontrak mfmt; lexing mentah menghasilkan output
    // tak stabil. Skip (bukan bug fmt).
    if source.contains('`') {
        return Err(FmtError {
            category: Category::CleanError,
            detail: "source mengandung preprocessor directive — di luar kontrak mfmt".to_string(),
        });
    }

    let input_parses =
        std::panic::catch_unwind(|| maria_api::compile_str_quiet(source).is_ok()).unwrap_or(false);

    let once = maria_api::tools::fmt::format_source(source, 4);
    if once.is_empty() && !source.trim().is_empty() {
        if input_parses {
            return Err(FmtError {
                category: Category::RoundtripMismatch,
                detail: "fmt output kosong padahal input VALID".to_string(),
            });
        }
        return Err(FmtError {
            category: Category::CleanError,
            detail: "input tidak valid — fmt output kosong wajar".to_string(),
        });
    }
    let once_parses =
        std::panic::catch_unwind(|| maria_api::compile_str_quiet(&once).is_ok()).unwrap_or(false);
    if !once_parses {
        if input_parses {
            // GOLDEN: fmt merusak source yang VALID — bug nyata.
            return Err(FmtError {
                category: Category::RoundtripMismatch,
                detail: format!(
                    "fmt output tidak parseable padahal input valid (fmt merusak source):\n{once}"
                ),
            });
        }
        // Input rusak (mutasi) → output rusak wajar, bukan bug fmt.
        return Err(FmtError {
            category: Category::CleanError,
            detail: "input tidak valid — skip (bukan bug fmt)".to_string(),
        });
    }
    let twice = maria_api::tools::fmt::format_source(&once, 4);
    if twice != once {
        return Err(FmtError {
            category: Category::RoundtripMismatch,
            detail: format!("fmt(fmt(s)) != fmt(s):\n--- once ---\n{once}\n--- twice ---\n{twice}"),
        });
    }
    Ok(())
}

/// Target CLI: jalankan maria binary dengan arg random di cwd temp.
fn evaluate_cli(source: &str, timeout_ms: u64) -> CaseResult {
    // CLI/tool dijalankan ATAS source mutasi nyata — sebelum ini argumen acak
    // tanpa file (`let _ = source`), jadi tools (mcheck/melab/msim/...) dan
    // flag pipeline tak pernah tersentuh source yang dimutasi. Tulis source ke
    // temp file → jalankan `maria <file> <flags>` / `maria <tool> <file>`.
    let mut path = std::env::temp_dir();
    path.push(format!(
        "mariafzcli_{}_{}.sv",
        std::process::id(),
        crate::next_crash_seq()
    ));
    if std::fs::write(&path, source).is_err() {
        return mk(
            Target::Cli,
            Oracle::O1NoCrash,
            Category::CleanError,
            "gagal tulis temp file",
            source,
        );
    }

    // RNG deterministik per-case (hash source) → kombinasi flags reproducible.
    let mut h: u64 = 0x9E3779B97F4A7C15;
    for b in source.bytes() {
        h = h.rotate_left(5) ^ u64::from(b).wrapping_mul(0x100000001B3);
        h = h.wrapping_mul(0x9E3779B97F4A7C15);
    }
    let mut rng = crate::Rng::new(h);

    let args = gen_cli_args(&mut rng, &path, source);
    let outcome = with_micd_isolated(|| crate::runner::run_args(&args, timeout_ms));
    let _ = std::fs::remove_file(&path);

    let args_desc = args.join(" ");

    // Double-run determinisme utk tool STATIS (mfmt/mcheck/mlint/minspect —
    // output logis harus identik antar run; baris timing di-strip). Msm/melab
    // punya timing output — tidak di-double-run. Oracle O4 atas tool CLI.
    let is_static_tool = matches!(
        args.first().map(String::as_str),
        Some("mfmt") | Some("mcheck") | Some("mlint") | Some("minspect")
    );
    if is_static_tool && outcome.kind == crate::runner::Kind::Ok && rng.chance(40) {
        let second = with_micd_isolated(|| crate::runner::run_args(&args, timeout_ms));
        if second.kind == crate::runner::Kind::Ok {
            let strip = |s: &str| -> Vec<String> {
                s.lines()
                    .filter(|l| !l.contains("time") && !l.contains("µs") && !l.contains("ms)"))
                    .map(|l| l.trim().to_string())
                    .collect()
            };
            if strip(&outcome.stdout) != strip(&second.stdout) {
                return mk(
                    Target::Cli,
                    Oracle::O4Determinism,
                    Category::NonDeterministic,
                    &format!("cli {args_desc}: tool output beda antar 2 run identik"),
                    source,
                );
            }
        }
    }

    match outcome.kind {
        crate::runner::Kind::Ok => mk(
            Target::Cli,
            Oracle::O1NoCrash,
            Category::Ok,
            &format!("cli ok: {args_desc}"),
            source,
        ),
        crate::runner::Kind::CleanError => mk(
            Target::Cli,
            Oracle::O1NoCrash,
            Category::CleanError,
            &format!("cli {args_desc}: {}", outcome.stderr),
            source,
        ),
        crate::runner::Kind::Panic => mk(
            Target::Cli,
            Oracle::O1NoCrash,
            Category::Panic,
            &format!("cli {args_desc}: {}", outcome.stderr),
            source,
        ),
        crate::runner::Kind::Abort => mk(
            Target::Cli,
            Oracle::O1NoCrash,
            Category::Abort,
            &format!("cli {args_desc}: {}", outcome.stderr),
            source,
        ),
        crate::runner::Kind::Crash(code) => mk(
            Target::Cli,
            Oracle::O1NoCrash,
            Category::Panic,
            &format!("cli {args_desc}: crash code {code}: {}", outcome.stderr),
            source,
        ),
        crate::runner::Kind::Hang => mk(
            Target::Cli,
            Oracle::O1NoCrash,
            Category::Hang,
            &format!("cli {args_desc}: hang > {} ms", timeout_ms),
            source,
        ),
    }
}

/// Fuzzing VCD WAVEFORM PIPELINE (area belum tersentuh — mwave/tools VCD
/// tidak pernah di-fuzz):
/// 1. Generate VCD base dari source yang sim-ok via `maria msim <sv> -o <vcd>`.
/// 2. Mutasi VCD (teks) 0-1x — VCD parser stress (format rusak, scope tak
///    seimbang, timestamp acak, dll).
/// 3. Jalankan subcommand mwave (stats/tree/search/export/compare/filter/merge/
///    get) atas VCD mutasi → O1 no-crash (panic/abort/hang di tool VCD = bug).
/// 4. O4: stats double-run → output harus identik.
fn evaluate_vcd(source: &str, timeout_ms: u64) -> CaseResult {
    use std::path::PathBuf;
    let mk_v = |c: Category, o: Oracle, d: &str| mk(Target::Vcd, o, c, d, source);

    // Hash source → RNG deterministik per-case.
    let mut h: u64 = 0x9E3779B97F4A7C15;
    for b in source.bytes() {
        h = h.rotate_left(5) ^ u64::from(b).wrapping_mul(0x100000001B3);
        h = h.wrapping_mul(0x9E3779B97F4A7C15);
    }
    let mut rng = crate::Rng::new(h);

    // ── 1. Generate VCD base ──
    let dir = std::env::temp_dir();
    let stem = format!(
        "mariafzv_{}_{}",
        std::process::id(),
        crate::next_crash_seq()
    );
    let sv = dir.join(format!("{stem}.sv"));
    let vcd_base = dir.join(format!("{stem}_base.vcd"));
    let vcd_mut = dir.join(format!("{stem}_mut.vcd"));
    let vcd_out = dir.join(format!("{stem}_out.vcd"));
    if std::fs::write(&sv, source).is_err() {
        return mk_v(
            Category::CleanError,
            Oracle::O1NoCrash,
            "gagal tulis temp sv",
        );
    }
    let args = vec![
        "msim".to_string(),
        sv.to_string_lossy().to_string(),
        "-T".to_string(),
        "200".to_string(),
        "-o".to_string(),
        vcd_base.to_string_lossy().to_string(),
    ];
    let outcome = crate::runner::run_args(&args, timeout_ms);
    let _ = std::fs::remove_file(&sv);
    if outcome.kind != crate::runner::Kind::Ok {
        // Source tidak sim-ok (mutasi merusak sintaks/elab) — bukan area VCD.
        let _ = std::fs::remove_file(&vcd_base);
        return mk_v(
            Category::CleanError,
            Oracle::O1NoCrash,
            "sim pendahulu gagal — VCD tidak dihasilkan",
        );
    }
    let vcd_src = match std::fs::read_to_string(&vcd_base) {
        Ok(s) if s.trim().len() >= 32 => s,
        _ => {
            let _ = std::fs::remove_file(&vcd_base);
            return mk_v(
                Category::CleanError,
                Oracle::O1NoCrash,
                "VCD kosong/tidak terbentuk",
            );
        }
    };

    // ── 2. Mutasi VCD 0-1x (tanpa splice corpus — Corpus kosong) ──
    let empty_corpus = crate::corpus::Corpus::load(Some(&PathBuf::from("/nonexistent-fz-vcd")));
    let mut vcd_target = vcd_src;
    if rng.chance(70) {
        let mut mutator = crate::mutator::Mutator::new(&mut rng);
        vcd_target = mutator.mutate(&vcd_target, &empty_corpus);
    }
    if std::fs::write(&vcd_mut, &vcd_target).is_err() {
        let _ = std::fs::remove_file(&vcd_base);
        return mk_v(
            Category::CleanError,
            Oracle::O1NoCrash,
            "gagal tulis VCD mutasi",
        );
    }

    let mcmd = |args2: Vec<String>| -> crate::runner::Outcome {
        with_micd_isolated(|| crate::runner::run_args(&args2, timeout_ms))
    };
    let vcd_mut_s = vcd_mut.to_string_lossy().to_string();
    let vcd_base_s = vcd_base.to_string_lossy().to_string();
    let vcd_out_s = vcd_out.to_string_lossy().to_string();

    // ── 3. Kombinasi mwave subcommand (2 acak dari 10) ──
    // WAV-16 decode (protokol APB/AXI4Lite/AHB) — area BELUM di-fuzz
    // sebelumnya (daftar lama 8 subcommand tanpa decode).
    let cmds: [Vec<String>; 10] = [
        vec!["mwave".into(), "stats".into(), vcd_mut_s.clone()],
        vec!["mwave".into(), "tree".into(), vcd_mut_s.clone()],
        vec![
            "mwave".into(),
            "search".into(),
            vcd_mut_s.clone(),
            "*".into(),
        ],
        vec!["mwave".into(), "export".into(), vcd_mut_s.clone()],
        vec![
            "mwave".into(),
            "compare".into(),
            vcd_base_s.clone(),
            vcd_mut_s.clone(),
        ],
        vec![
            "mwave".into(),
            "filter".into(),
            vcd_mut_s.clone(),
            "q".into(),
            "clk".into(),
        ],
        vec![
            "mwave".into(),
            "merge".into(),
            vcd_base_s.clone(),
            vcd_mut_s.clone(),
            "-o".into(),
            vcd_out_s.clone(),
        ],
        vec![
            "mwave".into(),
            "get".into(),
            vcd_mut_s.clone(),
            "--at".into(),
            "0".into(),
        ],
        vec![
            "mwave".into(),
            "decode".into(),
            vcd_mut_s.clone(),
            "--proto".into(),
            "apb".into(),
        ],
        vec![
            "mwave".into(),
            "decode".into(),
            vcd_mut_s.clone(),
            "--proto".into(),
            "axi4lite".into(),
        ],
    ];
    let n = 1 + rng.below(2);
    for _ in 0..n {
        let idx = rng.below(cmds.len());
        let o = mcmd(cmds[idx].clone());
        match o.kind {
            crate::runner::Kind::Ok => {}
            crate::runner::Kind::CleanError => {
                // VCD korup → error parser wajar (bukan bug).
                let _ = std::fs::remove_file(&vcd_base);
                let _ = std::fs::remove_file(&vcd_mut);
                return mk_v(
                    Category::CleanError,
                    Oracle::O1NoCrash,
                    "mwave clean error (VCD invalid)",
                );
            }
            crate::runner::Kind::Panic => {
                let _ = std::fs::remove_file(&vcd_base);
                let _ = std::fs::remove_file(&vcd_mut);
                return mk_v(
                    Category::Panic,
                    Oracle::O1NoCrash,
                    &format!("mwave panic: {}", o.stderr),
                );
            }
            crate::runner::Kind::Abort => {
                let _ = std::fs::remove_file(&vcd_base);
                let _ = std::fs::remove_file(&vcd_mut);
                return mk_v(
                    Category::Abort,
                    Oracle::O1NoCrash,
                    &format!("mwave abort: {}", o.stderr),
                );
            }
            crate::runner::Kind::Crash(code) => {
                let _ = std::fs::remove_file(&vcd_base);
                let _ = std::fs::remove_file(&vcd_mut);
                return mk_v(
                    Category::Panic,
                    Oracle::O1NoCrash,
                    &format!("mwave crash code {code}: {}", o.stderr),
                );
            }
            crate::runner::Kind::Hang => {
                let _ = std::fs::remove_file(&vcd_base);
                let _ = std::fs::remove_file(&vcd_mut);
                return mk_v(
                    Category::Hang,
                    Oracle::O1NoCrash,
                    &format!("mwave hang > {} ms", timeout_ms),
                );
            }
        }
    }

    // ── 4. O4 stats double-run (output identik antar run) ──
    let stats_args = vec!["mwave".into(), "stats".into(), vcd_mut_s.clone()];
    let o1 = mcmd(stats_args.clone());
    let o2 = mcmd(stats_args);
    if o1.kind == crate::runner::Kind::Ok && o2.kind == crate::runner::Kind::Ok {
        if o1.stdout != o2.stdout {
            let _ = std::fs::remove_file(&vcd_base);
            let _ = std::fs::remove_file(&vcd_mut);
            return mk_v(
                Category::NonDeterministic,
                Oracle::O4Determinism,
                "mwave stats non-deterministik: dua run identik hasil beda",
            );
        }
    }

    let _ = std::fs::remove_file(&vcd_base);
    let _ = std::fs::remove_file(&vcd_mut);
    let _ = std::fs::remove_file(&vcd_out);
    mk_v(
        Category::Ok,
        Oracle::O1NoCrash,
        "vcd pipeline ok: mwave robust + deterministic",
    )
}

/// SDF base valid minimal — seed transisi utk mutasi teks SDF. SDF parser
/// (`crates/maria-simulator/src/simulator/sdf.rs`) TIDAK pernah di-fuzz.
const SDF_BASE: &str = "(DELAYFILE
  (SDFVERSION \"3.0\")
  (DESIGN \"top\")
  (VENDOR \"maria\")
  (PROGRAM \"maria-fuzz\")
  (VERSION \"1.0\")
  (DIVIDER .)
  (TIMESCALE 1ns)
  (CELL (CELLTYPE \"top\") (INSTANCE u1)
    (DELAY (ABSOLUTE (IOPATH clk q (0.1:0.2:0.3) (0.4:0.5:0.6))))
  )
)
";

/// Fuzzing SDF timing pipeline (SIM-09) — area BELUM tersentuh:
/// 1. SDF teks di-mutasi 0-2x dari SDF_BASE (parser SDF stress).
/// 2. SV source TIDAK di-mutasi di sini — run_single sudah mutasi SV 0-4x
///    sebelum evaluate; mutasi ganda = amplifier blowup (terukur 267MB).
/// 3. Subprocess `maria <sv> --sdf <sdf> -T 200` — parse_file + annotate_sdf
///    + sim dengan timing delay.
/// 4. O1 no-crash: panic/abort/hang di jalur SDF = bug (parser SDF, annotator,
///    atau engine timing). CleanError = SDF/SV invalid wajar.
fn evaluate_sdf(source: &str, timeout_ms: u64) -> CaseResult {
    use std::path::PathBuf;
    let mk_s = |c: Category, o: Oracle, d: &str| mk(Target::Sdf, o, c, d, source);

    // RNG deterministik per-case (hash source) → mutasi reproducible.
    let mut h: u64 = 0x9E3779B97F4A7C15;
    for b in source.bytes() {
        h = h.rotate_left(5) ^ u64::from(b).wrapping_mul(0x100000001B3);
        h = h.wrapping_mul(0x9E3779B97F4A7C15);
    }
    let mut rng = crate::Rng::new(h);
    let empty_corpus = crate::corpus::Corpus::load(Some(&PathBuf::from("/nonexistent-fz-sdf")));

    // 1. SDF text mutasi 0-2x.
    let mut sdf_text = SDF_BASE.to_string();
    let n_sdf = rng.below(3);
    if n_sdf > 0 {
        let mut mutator = crate::mutator::Mutator::new(&mut rng);
        for _ in 0..n_sdf {
            sdf_text = mutator.mutate(&sdf_text, &empty_corpus);
        }
    }

    // SV: seed fuzz as-is (sudah di-mutasi oleh run_single).
    let sv_text = source;

    // Env hook: tulis SDF pre-eval untuk repro hang/panic (pasangan dari
    // MARIA_FUZZ_TRACE — SV saja tidak cukup: kasus hang butuh SDF mutasi
    // yang juga per-case).
    if let Ok(trace_path) = std::env::var("MARIA_FUZZ_SDF_TRACE") {
        let _ = std::fs::write(&trace_path, &sdf_text);
    }

    // Capture per-case (sv+sdf persisten) — untuk lokalasi kasus hang/panic
    // yang butuh PASANGAN persis. Dipakai debug; normalnya tidak di-set.
    if let Ok(cap_dir) = std::env::var("MARIA_FUZZ_CAPTURE_DIR") {
        let _ = std::fs::create_dir_all(&cap_dir);
        let seq = crate::next_crash_seq();
        let _ = std::fs::write(
            std::path::Path::new(&cap_dir).join(format!("{seq}.sv")),
            sv_text,
        );
        let _ = std::fs::write(
            std::path::Path::new(&cap_dir).join(format!("{seq}.sdf")),
            &sdf_text,
        );
    }

    // 3. Tulis temp + jalankan.
    let dir = std::env::temp_dir();
    let stem = format!(
        "mariafzs_{}_{}",
        std::process::id(),
        crate::next_crash_seq()
    );
    let sv = dir.join(format!("{stem}.sv"));
    let sdf = dir.join(format!("{stem}.sdf"));
    if std::fs::write(&sv, &sv_text).is_err() || std::fs::write(&sdf, &sdf_text).is_err() {
        return mk_s(
            Category::CleanError,
            Oracle::O1NoCrash,
            "gagal tulis temp sv/sdf",
        );
    }
    let args = vec![
        sv.to_string_lossy().to_string(),
        "--sdf".to_string(),
        sdf.to_string_lossy().to_string(),
        "-T".to_string(),
        "200".to_string(),
    ];
    let outcome = with_micd_isolated(|| crate::runner::run_args(&args, timeout_ms));
    let _ = std::fs::remove_file(&sv);
    let _ = std::fs::remove_file(&sdf);

    match outcome.kind {
        crate::runner::Kind::Ok => mk_s(
            Category::Ok,
            Oracle::O1NoCrash,
            &format!("sdf pipeline ok: parse+annotate+sim ({:?})", stem),
        ),
        crate::runner::Kind::CleanError => mk_s(
            Category::CleanError,
            Oracle::O1NoCrash,
            &format!("sdf clean error: {}", outcome.stderr.lines().next().unwrap_or("")),
        ),
        crate::runner::Kind::Panic => mk_s(
            Category::Panic,
            Oracle::O1NoCrash,
            &format!("sdf panic: {}", outcome.stderr),
        ),
        crate::runner::Kind::Abort => mk_s(
            Category::Abort,
            Oracle::O1NoCrash,
            &format!("sdf abort: {}", outcome.stderr),
        ),
        crate::runner::Kind::Crash(code) => mk_s(
            Category::Panic,
            Oracle::O1NoCrash,
            &format!("sdf crash code {code}: {}", outcome.stderr),
        ),
        crate::runner::Kind::Hang => mk_s(
            Category::Hang,
            Oracle::O1NoCrash,
            &format!("sdf hang > {} ms", timeout_ms),
        ),
    }
}

/// Fuzzing MICD incremental database (`--fast` = run_fast + CompileSession +
/// MICD cache) — area BELUM tersentuh:
/// 1. Base: `maria --fast <sv> -T 200` (cold cache, seed).
/// 2. Incremental: run ulang (cache hit) → hasil harus == fresh.
/// 3. Fresh: `--recompile` (lewati MICD) → base of truth.
/// 4. O4: incremental vs recompile output identik (setelah strip timing)?
///    Jika beda → cache drift / silent miscompilation = bug CRITICAL.
/// 5. O1: panic/hang di jalur --fast = bug.
///
/// MARIA_MICD_DIR diarahkan ke temp per-case agar tidak mencemari
/// `.maria/database` project.
fn evaluate_micd(source: &str, timeout_ms: u64) -> CaseResult {
    let mk_m = |c: Category, o: Oracle, d: &str| mk(Target::Micd, o, c, d, source);

    let dir = std::env::temp_dir();
    let stem = format!(
        "mariafzm_{}_{}",
        std::process::id(),
        crate::next_crash_seq()
    );
    let sv = dir.join(format!("{stem}.sv"));
    if std::fs::write(&sv, source).is_err() {
        return mk_m(
            Category::CleanError,
            Oracle::O1NoCrash,
            "gagal tulis temp sv",
        );
    }

    let sv_s = sv.to_string_lossy().to_string();
    let base: Vec<String> = vec![
        "--fast".to_string(),
        sv_s.clone(),
        "-T".to_string(),
        "200".to_string(),
    ];
    let recompile: Vec<String> = vec![
        "--fast".to_string(),
        sv_s.clone(),
        "-T".to_string(),
        "200".to_string(),
        "--recompile".to_string(),
    ];

    // Isolasi MICD per-case via helper (temp unik + cleanup otomatis).
    let (r1, r2, r3) = with_micd_isolated(|| {
        (
            crate::runner::run_args(&base, timeout_ms),      // cold cache — seed
            crate::runner::run_args(&base, timeout_ms),      // incremental
            crate::runner::run_args(&recompile, timeout_ms), // fresh
        )
    });
    let _ = std::fs::remove_file(&sv);

    if r1.kind != crate::runner::Kind::Ok {
        // Compile/sim gagal di run pertama (SV invalid) — bukan area MICD.
        return mk_m(
            Category::CleanError,
            Oracle::O1NoCrash,
            &format!("run pertama gagal: {}", r1.stderr.lines().next().unwrap_or("")),
        );
    }

    // O1: crash/hang pada jalur incremental/fresh.
    for (label, r) in [("incremental", &r2), ("recompile", &r3)] {
        match r.kind {
            crate::runner::Kind::Ok => {}
            crate::runner::Kind::CleanError => {
                return mk_m(
                    Category::Differential,
                    Oracle::O5Differential,
                    &format!(
                        "{label} clean-error padahal run pertama ok: {}",
                        r.stderr.lines().next().unwrap_or("")
                    ),
                );
            }
            crate::runner::Kind::Panic => {
                return mk_m(
                    Category::Panic,
                    Oracle::O1NoCrash,
                    &format!("{label} panic: {}", r.stderr),
                );
            }
            crate::runner::Kind::Abort => {
                return mk_m(
                    Category::Abort,
                    Oracle::O1NoCrash,
                    &format!("{label} abort: {}", r.stderr),
                );
            }
            crate::runner::Kind::Crash(code) => {
                return mk_m(
                    Category::Panic,
                    Oracle::O1NoCrash,
                    &format!("{label} crash code {code}: {}", r.stderr),
                );
            }
            crate::runner::Kind::Hang => {
                return mk_m(
                    Category::Hang,
                    Oracle::O1NoCrash,
                    &format!("{label} hang > {} ms", timeout_ms),
                );
            }
        }
    }

    // O4: MICD incremental == fresh (strip baris timing/`[TIMING]` stderr tak
    // dipakai — stdout saja; baris metrik waktu di-strip).
    let strip = |s: &str| -> Vec<String> {
        s.lines()
            .filter(|l| {
                !l.contains("time") && !l.contains("µs") && !l.contains("ms)")
            })
            .map(|l| l.trim().to_string())
            .collect()
    };
    let out_incr = strip(&r2.stdout);
    let out_fresh = strip(&r3.stdout);
    if out_incr != out_fresh {
        return mk_m(
            Category::NonDeterministic,
            Oracle::O4Determinism,
            "MICD drift: incremental != --recompile (cache mengubah hasil sim)",
        );
    }

    // Bandingkan juga dengan run pertama (cold) — konsistensi tiga arah.
    let out_cold = strip(&r1.stdout);
    if out_cold != out_incr {
        return mk_m(
            Category::NonDeterministic,
            Oracle::O4Determinism,
            "MICD drift: cold run != incremental run",
        );
    }

    let _ = r1;
    mk_m(
        Category::Ok,
        Oracle::O1NoCrash,
        "micd ok: incremental == recompile == cold",
    )
}

/// Argumen CLI untuk satu kasus fuzz: tool subcommand ATAU pipeline flags,
/// atas satu file temp. RNG per-case (bukan global) → deterministik.
fn gen_cli_args(rng: &mut crate::Rng, path: &std::path::Path, source: &str) -> Vec<String> {
    let file = path.to_string_lossy().to_string();
    let mut args = Vec::new();

    // 50%: tool subcommand `maria <tool> <file> [flags]` — area maria-tools
    // (mcheck/melab/msim/mfmt/mlint/minspect/mprof) tak pernah di-fuzz atas
    // source mutasi sebelumnya. mbench/mcov/mwave/synth sengaja dilewatkan
    // (berat/butuh VCD — timeout palsu).
    if rng.chance(50) {
        let tools = [
            "mcheck", "melab", "msim", "mfmt", "mlint", "minspect", "mprof",
        ];
        let t = tools[rng.below(tools.len())];
        args.push(t.to_string());
        args.push(file.clone());
        match t {
            "msim" => {
                args.push("-T".to_string());
                args.push(["50", "100", "200", "1000"][rng.below(4)].to_string());
            }
            "mfmt" => {
                if rng.chance(25) {
                    args.push("--check".to_string());
                }
            }
            _ => {}
        }
        return args;
    }

    // Pipeline: `maria <file> [-T N] <flags>` — flag nondestruktif.
    // --debug/--step dulu dilewatkan (interaktif) — diuji manual: semua flag
    // debug keluar EXIT 0 dgn stdin-null, tak hang. Tambah sekarang:
    // --deep-debug/--break-cycle/--timeline/--print-signal/--snap-interval/
    // --watch/--debug = area debugger yang belum pernah di-fuzz.
    args.push(file.clone());
    if rng.chance(70) {
        args.push("-T".to_string());
        args.push(["50", "100", "200", "1000"][rng.below(4)].to_string());
    }
    // mode run_fast: 20% jalur MICD penuh (`--fast` memicu run_fast — NOTA:
    // dulu sisipkan "run_fast" sbg argv[0] yang dianggap FILE oleh maria →
    // clean error noise; sekarang flag `--fast` yang benar).
    if rng.chance(20) {
        args.push("--fast".to_string());
    }

    let flags = [
        "--ast",
        "--tokens",
        "--tree",
        "--print-state",
        "--coverage",
        "--fast",
        "--recompile",
        "--deep-debug",
        "--debug",
    ];
    let n = 1 + rng.below(3);
    for _ in 0..n {
        if rng.chance(55) {
            args.push(flags[rng.below(flags.len())].to_string());
        }
    }
    // Flag debug bernilai: break-cycle / snap-interval / timeline / print-signal
    if rng.chance(45) {
        let cv = rng.below(3);
        match cv {
            0 => {
                args.push("--break-cycle".to_string());
                args.push(format!("{}", 1 + rng.below(50)));
            }
            1 => {
                args.push("--snap-interval".to_string());
                args.push(format!("{}", 10 + rng.below(200)));
            }
            _ => {
                // --timeline/--print-signal/--watch butuh nama signal — pilih
                // ident pendek dari source (nama signal nyata bila ada).
                let mut name = "q".to_string();
                'outer: for w in source.split([' ', '\n', '\t', '(', ')', ',', ';', '[', ']', '.'])
                {
                    let w = w.trim();
                    if w.len() >= 2
                        && w.len() <= 16
                        && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                        && !w.is_empty()
                    {
                        name = w.to_string();
                        break 'outer;
                    }
                }
                let kind = rng.below(3);
                match kind {
                    0 => {
                        args.push("--timeline".to_string());
                        args.push(name.clone());
                    }
                    1 => {
                        args.push("--print-signal".to_string());
                        args.push(name.clone());
                    }
                    _ => {
                        args.push("--watch".to_string());
                        args.push(name.clone());
                    }
                }
            }
        }
    }
    args
}

/// Fuzzing PREPROCESSOR (area belum tersentuh):
/// - output preprocess BERUBAH antar dua run identik = non-deterministik (bug)
/// - panic saat directive imbalance (`ifdef` ganda dsb) = bug
/// - error tanpa lokasi file:line:col = diag_missing
fn evaluate_preproc(source: &str) -> CaseResult {
    // Jalankan preprocess 2x — determinisme.
    let r1 = std::panic::catch_unwind(|| maria_preproc(&source));
    let r2 = std::panic::catch_unwind(|| maria_preproc(&source));
    match (r1, r2) {
        (Ok(Ok(o1)), Ok(Ok(o2))) => {
            if o1 != o2 {
                return mk(
                    Target::Preproc,
                    Oracle::O4Determinism,
                    Category::NonDeterministic,
                    "preprocess non-deterministik: dua run identik hasil beda",
                    source,
                );
            }
            mk(
                Target::Preproc,
                Oracle::O1NoCrash,
                Category::Ok,
                "preproc deterministik",
                source,
            )
        }
        (Ok(Err(e1)), Ok(Err(e2))) => {
            if e1 == e2 {
                let has_loc = extract_loc(&e1);
                if has_loc {
                    mk(
                        Target::Preproc,
                        Oracle::O1NoCrash,
                        Category::CleanError,
                        &e1,
                        source,
                    )
                } else {
                    mk(
                        Target::Preproc,
                        Oracle::O2DiagLocation,
                        Category::DiagMissing,
                        &format!("preproc tanpa lokasi: {e1}"),
                        source,
                    )
                }
            } else {
                mk(
                    Target::Preproc,
                    Oracle::O4Determinism,
                    Category::NonDeterministic,
                    &format!("error preproc beda: {e1} vs {e2}"),
                    source,
                )
            }
        }
        (Ok(Err(e)), _) | (_, Ok(Err(e))) => mk(
            Target::Preproc,
            Oracle::O4Determinism,
            Category::NonDeterministic,
            &format!("satu run error satu ok: {e}"),
            source,
        ),
        _ => mk(
            Target::Preproc,
            Oracle::O1NoCrash,
            Category::Panic,
            "panic saat preprocess",
            source,
        ),
    }
}

/// Jalankan preprocessor maria (public API) — kembalikan output atau error.
fn maria_preproc(source: &str) -> Result<String, String> {
    use maria_parser::preprocessor::Preprocessor;
    let mut pp = Preprocessor::new();
    match pp.preprocess(source, None) {
        Ok(out) => {
            // Tambah folder include default agar dirwasa; ok.
            Ok(out)
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Fuzzing TRANSPILER MV → SV (area belum tersentuh):
/// - transpile 2x identik = determinisme (macro/state leak)
/// - output SV harus parseable (transpile merusak source = bug)
/// - panic saat MV aneh = bug
fn evaluate_mv(source: &str) -> CaseResult {
    let r1 = std::panic::catch_unwind(|| maria_api::mv::transpile(source, "fz"));
    let r2 = std::panic::catch_unwind(|| maria_api::mv::transpile(source, "fz"));
    match (r1, r2) {
        (Ok(Ok(t1)), Ok(Ok(t2))) => {
            if t1.sv != t2.sv || t1.svh != t2.svh {
                return mk(
                    Target::Mv,
                    Oracle::O4Determinism,
                    Category::NonDeterministic,
                    "transpile non-deterministik",
                    source,
                );
            }
            // Output SV harus parseable.
            let combined = format!("{}\n{}", t1.svh, t1.sv);
            let parses =
                std::panic::catch_unwind(|| maria_api::compile_str_quiet(&combined).is_ok())
                    .unwrap_or(false);
            if !parses {
                return mk(
                    Target::Mv,
                    Oracle::O3Roundtrip,
                    Category::RoundtripMismatch,
                    "transpile output tidak parseable (transpiler merusak source)",
                    source,
                );
            }
            mk(
                Target::Mv,
                Oracle::O1NoCrash,
                Category::Ok,
                "mv transpile deterministik + output parseable",
                source,
            )
        }
        (Ok(Err(_e1)), Ok(Err(_e2))) => {
            // Error deterministik — MV tak valid, bukan bug.
            mk(
                Target::Mv,
                Oracle::O1NoCrash,
                Category::CleanError,
                "mv transpile error deterministik",
                source,
            )
        }
        _ => mk(
            Target::Mv,
            Oracle::O4Determinism,
            Category::NonDeterministic,
            "transpile panic/error non-deterministik",
            source,
        ),
    }
}

/// Ekstrak lokasi `file:line:col` dari pesan error.
fn extract_loc(msg: &str) -> bool {
    // Pola umum maria: "file.sv:12:7: error: ..." atau "path:12:7:  ..."
    has_num_colon(msg)
}

fn has_num_colon(msg: &str) -> bool {
    let bytes = msg.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i + 2 < n {
        if bytes[i].is_ascii_digit() && bytes[i + 1] == b':' && bytes[i + 2].is_ascii_digit() {
            return true;
        }
        i += 1;
    }
    false
}
