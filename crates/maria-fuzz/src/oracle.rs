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
        Target::Lexer | Target::Parser | Target::Elaborator => {
            evaluate_compile(target, source)
        }
        Target::Simulator => evaluate_sim(source, timeout_ms),
        Target::Fmt => evaluate_fmt(source),
        Target::Cli => evaluate_cli(source, timeout_ms),
        Target::Preproc => evaluate_preproc(source),
        Target::Mv => evaluate_mv(source),
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

/// O1 + O2 untuk compile pipeline (lexer/parser/elaborator).
fn evaluate_compile(target: Target, source: &str) -> CaseResult {
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
                let recovered =
                    std::panic::catch_unwind(|| maria_api::compile_str_analyze(source));
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
            mk(target, Oracle::O1NoCrash, Category::Ok, "compile ok", source)
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
    let outcome = crate::runner::run_file(source, timeout_ms);
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

    // ── 2. In-process compile check (O1) ──
    let compile_result = std::panic::catch_unwind(|| maria_api::compile_str_quiet(source));
    match compile_result {
        Ok(Err(e)) => {
            return mk(
                Target::Simulator,
                Oracle::O1NoCrash,
                Category::CleanError,
                &format!("compile gagal: {e}"),
                source,
            );
        }
        Err(_) => {
            return mk(
                Target::Simulator,
                Oracle::O1NoCrash,
                Category::Panic,
                "panic saat compile in-process",
                source,
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
            if sigs1 != sigs2 {
                return mk(
                    Target::Simulator,
                    Oracle::O4Determinism,
                    Category::NonDeterministic,
                    &format!(
                        "sim non-deterministik: {} != {} signal values (2 run identik)",
                        signal_summary(sigs1),
                        signal_summary(sigs2)
                    ),
                    source,
                );
            }
        }
        (Ok(Err(e1)), Ok(Err(_e2))) => {
            // Keduanya error — error deterministik, bukan bug
            return mk(
                Target::Simulator,
                Oracle::O1NoCrash,
                Category::CleanError,
                &format!("sim error deterministik: {e1}"),
                source,
            );
        }
        (Ok(Err(e)), _) | (_, Ok(Err(e))) => {
            // Salah satu error, satu ok — non-deterministik error
            return mk(
                Target::Simulator,
                Oracle::O4Determinism,
                Category::NonDeterministic,
                &format!("sim error non-deterministik: {e}"),
                source,
            );
        }
        (Err(_), _) | (_, Err(_)) => {
            return mk(
                Target::Simulator,
                Oracle::O4Determinism,
                Category::NonDeterministic,
                "sim panic tidak deterministik",
                source,
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
        ("packed", EngineFlags {
            use_packed_eval: true,
            ..default_flags
        }),
        ("dag", EngineFlags {
            use_dag_parallel: true,
            ..default_flags
        }),
        ("timing-wheel", EngineFlags {
            use_timing_wheel: true,
            ..default_flags
        }),
        ("mir-jit", EngineFlags {
            use_mir_jit: true,
            ..default_flags
        }),
    ];
    for (name, flags) in alternate_flags {
        let result = std::panic::catch_unwind(|| {
            simulate_signals_with_flags_quiet(source, 1_000, &flags)
        });
        match result {
            Ok(Ok(signals)) if signals == sigs_default => {}
            Ok(Ok(signals)) => {
                return mk(
                    Target::Simulator,
                    Oracle::O5Differential,
                    Category::Differential,
                    &format!(
                        "differential default vs {name}: {} != {}",
                        signal_summary(&sigs_default),
                        signal_summary(&signals)
                    ),
                    source,
                );
            }
            Ok(Err(e)) => {
                return mk(
                    Target::Simulator,
                    Oracle::O5Differential,
                    Category::Differential,
                    &format!("{name} path error, default ok: {e}"),
                    source,
                );
            }
            Err(_) => {
                return mk(
                    Target::Simulator,
                    Oracle::O5Differential,
                    Category::Differential,
                    &format!("{name} path panic, default ok"),
                    source,
                );
            }
        }
    }

    // ── 5. Trace quality: pastikan trace punya data bermakna ──
    let trace_result = std::panic::catch_unwind(|| {
        simulate_signals_with_trace_quiet(source, 1_000, 100)
    });

    match trace_result {
        Ok(Ok((sigs_trace, trace))) => {
            if trace.is_empty() {
                return mk(
                    Target::Simulator,
                    Oracle::O1NoCrash,
                    Category::Ok,
                    &format!(
                        "sim ok, no trace ({} signals)",
                        sigs_trace.len()
                    ),
                    source,
                );
            }

            // Trace ada — cek kualitas: minimal ada 1 trace entry yang bukan empty string
            let meaningful_traces: Vec<&String> = trace
                .iter()
                .filter(|t| !t.is_empty() && !t.trim().is_empty())
                .collect();
            if meaningful_traces.is_empty() {
                return mk(
                    Target::Simulator,
                    Oracle::O1NoCrash,
                    Category::Ok,
                    &format!(
                        "sim ok, trace kosong ({} entries, {} signals)",
                        trace.len(),
                        sigs_trace.len()
                    ),
                    source,
                );
            }

            if sigs_trace != sigs_default {
                return mk(
                    Target::Simulator,
                    Oracle::O5Differential,
                    Category::Differential,
                    &format!(
                        "trace final state berbeda: {} != {}",
                        signal_summary(&sigs_default),
                        signal_summary(&sigs_trace)
                    ),
                    source,
                );
            }

            let ev = std::panic::catch_unwind(|| crate::validate::evidence_only(source, 1_000))
                .unwrap_or_default();

            // ── 7. External reference (Icarus) — bukti KEBENARAN sim ──
            // Opsional via env MARIA_FUZZ_ICARUS=1: jalankan source juga di
            // iverilog+vvp (reference eksternal independen). Mismatch marker
            // = semantic divergence NYATA (hasil maria != reference).
            if std::env::var("MARIA_FUZZ_ICARUS").map(|v| v == "1").unwrap_or(false) {
                let ic = crate::oracle_icarus::evaluate_icarus(source, timeout_ms);
                match ic.verdict {
                    crate::oracle_icarus::Verdict::Mismatch => {
                        return mk(
                            Target::Simulator,
                            Oracle::O5Differential,
                            Category::Differential,
                            &ic.detail,
                            source,
                        );
                    }
                    crate::oracle_icarus::Verdict::MariaBug => {
                        return mk(
                            Target::Simulator,
                            Oracle::O1NoCrash,
                            Category::Panic,
                            &ic.detail,
                            source,
                        );
                    }
                    crate::oracle_icarus::Verdict::Match => {
                        return mk(
                            Target::Simulator,
                            Oracle::O5Differential,
                            Category::Ok,
                            &format!(
                                "sim VERIFIED vs Icarus reference ({}) — hasil identik",
                                ic.detail
                            ),
                            source,
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
            let x_heavy =
                ev.signal_count > 0 && (ev.x_remain + ev.z_remain) * 100 >= ev.signal_count.max(1) * 60;

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
                return mk(
                    Target::Simulator,
                    Oracle::O1NoCrash,
                    Category::Ok,
                    &format!("{detail} — design pasif (0 proses), X/Z wajar"),
                    source,
                );
            }
            if x_heavy {
                // Stimulus ada tapi signal dominan X → butuh perhatian:
                // bisa wajar (X-latch) atau bukti state-propagation bug.
                return mk(
                    Target::Simulator,
                    Oracle::O1NoCrash,
                    Category::Suspicious,
                    &format!("{detail} — stimulus ada ({}) namun X/Z dominan ({}/{})", ev.process_count, ev.x_remain + ev.z_remain, ev.signal_count),
                    source,
                );
            }

            mk(
                Target::Simulator,
                Oracle::O1NoCrash,
                Category::Ok,
                &detail,
                source,
            )
        }
        Ok(Err(e)) => {
            return mk(
                Target::Simulator,
                Oracle::O5Differential,
                Category::Differential,
                &format!("trace gagal setelah sim default sukses: {e}"),
                source,
            );
        }
        Err(_) => {
            return mk(
                Target::Simulator,
                Oracle::O5Differential,
                Category::Differential,
                "trace panic setelah sim default sukses",
                source,
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
        format!("{} signals ({} nonzero: {})", sigs.len(), nonzero.len(), nonzero.join(", "))
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
    // Fmt target dijalankan in-process — round-trip check
    let caught = std::panic::catch_unwind(|| fmt_roundtrip(source));
    match caught {
        Ok(Ok(())) => mk(Target::Fmt, Oracle::O3Roundtrip, Category::Ok, "fmt ok", source),
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
            _ => mk(Target::Fmt, Oracle::O1NoCrash, Category::Panic, &e.detail, source),
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

    let input_parses = std::panic::catch_unwind(|| {
        maria_api::compile_str_quiet(source).is_ok()
    })
    .unwrap_or(false);

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
    let once_parses = std::panic::catch_unwind(|| {
        maria_api::compile_str_quiet(&once).is_ok()
    })
    .unwrap_or(false);
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
            detail: format!(
                "fmt(fmt(s)) != fmt(s):\n--- once ---\n{once}\n--- twice ---\n{twice}"
            ),
        });
    }
    Ok(())
}

/// Target CLI: jalankan maria binary dengan arg random di cwd temp.
fn evaluate_cli(source: &str, timeout_ms: u64) -> CaseResult {
    let _ = source;
    let args = gen_cli_args();
    let outcome = crate::runner::run_args(&args, timeout_ms);
    match outcome.kind {
        crate::runner::Kind::Ok => mk(
            Target::Cli,
            Oracle::O1NoCrash,
            Category::Ok,
            &format!("cli ok: {}", args.join(" ")),
            "",
        ),
        crate::runner::Kind::CleanError => mk(
            Target::Cli,
            Oracle::O1NoCrash,
            Category::CleanError,
            &outcome.stderr,
            "",
        ),
        crate::runner::Kind::Panic => mk(
            Target::Cli,
            Oracle::O1NoCrash,
            Category::Panic,
            &outcome.stderr,
            "",
        ),
        crate::runner::Kind::Abort => mk(
            Target::Cli,
            Oracle::O1NoCrash,
            Category::Abort,
            &outcome.stderr,
            "",
        ),
        crate::runner::Kind::Crash(code) => mk(
            Target::Cli,
            Oracle::O1NoCrash,
            Category::Panic,
            &format!("crash code {code}"),
            "",
        ),
        crate::runner::Kind::Hang => mk(
            Target::Cli,
            Oracle::O1NoCrash,
            Category::Hang,
            &format!("hang > {} ms", timeout_ms),
            "",
        ),
    }
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
            mk(Target::Preproc, Oracle::O1NoCrash, Category::Ok, "preproc deterministik", source)
        }
        (Ok(Err(e1)), Ok(Err(e2))) => {
            if e1 == e2 {
                let has_loc = extract_loc(&e1);
                if has_loc {
                    mk(Target::Preproc, Oracle::O1NoCrash, Category::CleanError, &e1, source)
                } else {
                    mk(Target::Preproc, Oracle::O2DiagLocation, Category::DiagMissing, &format!("preproc tanpa lokasi: {e1}"), source)
                }
            } else {
                mk(Target::Preproc, Oracle::O4Determinism, Category::NonDeterministic, &format!("error preproc beda: {e1} vs {e2}"), source)
            }
        }
        (Ok(Err(e)), _) | (_, Ok(Err(e))) => {
            mk(Target::Preproc, Oracle::O4Determinism, Category::NonDeterministic, &format!("satu run error satu ok: {e}"), source)
        }
        _ => mk(Target::Preproc, Oracle::O1NoCrash, Category::Panic, "panic saat preprocess", source),
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
            let parses = std::panic::catch_unwind(|| maria_api::compile_str_quiet(&combined).is_ok())
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
            mk(Target::Mv, Oracle::O1NoCrash, Category::Ok, "mv transpile deterministik + output parseable", source)
        }
        (Ok(Err(_e1)), Ok(Err(_e2))) => {
            // Error deterministik — MV tak valid, bukan bug.
            mk(Target::Mv, Oracle::O1NoCrash, Category::CleanError, "mv transpile error deterministik", source)
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

/// Argumen CLI random untuk target Cli.
fn gen_cli_args() -> Vec<String> {
    let mut rng = crate::Rng::new(0xDEADBEEF);
    let cmds = [
        "compile", "simulate", "run", "run_fast", "check", "lint",
        "elaborate", "wave", "bench", "prof", "synth",
    ];
    let flags = [
        "--ast", "--tokens", "--tree", "--print-state", "--debug",
        "--deep-debug", "--fast", "--recompile", "--coverage",
    ];
    let paths = [
        "fz_top.sv", "tb_top.sv", "test.sv", "a.sv", "b.sv",
        ".", "..", "/tmp/fz_input.sv",
    ];

    let n = rng.below(6);
    let mut args = Vec::new();
    if rng.chance(70) {
        args.push(cmds[rng.below(cmds.len())].to_string());
    }
    for _ in 0..n {
        if rng.chance(50) {
            args.push(flags[rng.below(flags.len())].to_string());
        } else {
            args.push(paths[rng.below(paths.len())].to_string());
        }
    }
    args
}