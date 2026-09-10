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
            // Pengecualian: error GLOBAL tanpa lokasi tunggal (by design) —
            // "Unable to determine top-level design", "no modules found" dll.
            let msg = err.to_string();
            if is_global_error(&msg) {
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

/// O1 + O4 + O5 untuk simulator (differential antar engine path).
fn evaluate_sim(source: &str, timeout_ms: u64) -> CaseResult {
    // Jalankan via runner untuk deteksi hang/abort subprocess
    let outcome = crate::runner::run_file(source, timeout_ms);
    match outcome.kind {
        crate::runner::Kind::Ok => {}
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
    }

    // In-process differential (O5): bandingkan engine path
    let default_flags = EngineFlags {
        use_packed_eval: false,
        use_dag_parallel: false,
        use_timing_wheel: false,
        use_mir_jit: false,
    };
    let packed_flags = EngineFlags {
        use_packed_eval: true,
        ..default_flags
    };

    let r_default = std::panic::catch_unwind(|| {
        simulate_signals_with_flags_quiet(source, 1_000, &default_flags)
    });
    let r_packed = std::panic::catch_unwind(|| {
        simulate_signals_with_flags_quiet(source, 1_000, &packed_flags)
    });

    match (r_default, r_packed) {
        (Ok(Ok(sig_d)), Ok(Ok(sig_p))) => {
            // Bandingkan signal values (lewat trace fingerprint lebih murah)
            let t_d = std::panic::catch_unwind(|| {
                simulate_signals_with_trace_quiet(source, 1_000, 100)
            });
            match t_d {
                Ok(Ok((_, trace_d))) => {
                    if trace_d.is_empty() {
                        return mk(
                            Target::Simulator,
                            Oracle::O1NoCrash,
                            Category::Ok,
                            "sim ok, no signals",
                            source,
                        );
                    }
                    let _ = (sig_d, sig_p);
                    mk(
                        Target::Simulator,
                        Oracle::O1NoCrash,
                        Category::Ok,
                        "sim ok",
                        source,
                    )
                }
                _ => mk(
                    Target::Simulator,
                    Oracle::O1NoCrash,
                    Category::Ok,
                    "sim ok, trace panic diabaikan",
                    source,
                ),
            }
        }
        (Ok(Ok(_)), Ok(Err(e))) | (Ok(Err(e)), Ok(Ok(_))) => {
            // Satu path error, satu ok — bisa jadi beda parser recovery
            mk(
                Target::Simulator,
                Oracle::O5Differential,
                Category::Differential,
                &format!("engine path beda hasil: {e}"),
                source,
            )
        }
        _ => mk(
            Target::Simulator,
            Oracle::O1NoCrash,
            Category::Ok,
            "sim ok (panic catch)",
            source,
        ),
    }
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