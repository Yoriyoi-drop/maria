//! maria-fuzz CLI (dev-only, butuh --features dev).
//!
//! Subcommand: run, replay, triage, minimize, report, help.

use maria_fuzz::{
    bugs_dir, default_corpus_dir, minimize, run, triage, CaseResult, Category, FuzzConfig, Target,
};

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    let code = match cmd {
        "run" => cmd_run(&args[2..]),
        "replay" => cmd_replay(&args[2..]),
        "triage" => cmd_triage(&args[2..]),
        "minimize" => cmd_minimize(&args[2..]),
        "report" => cmd_report(&args[2..]),
        "help" | "-h" | "--help" => {
            print_help();
            0
        }
        other => {
            eprintln!("subcommand tidak dikenal: {other}\n");
            print_help();
            1
        }
    };
    std::process::exit(code);
}

fn print_help() {
    println!(
        r#"maria-fuzz {version} — fuzzer berbasis corpus project nyata (cva6/openc910/opentitan)

USAGE:
  maria-fuzz run     [--target <t>] [-c <n>] [--seed <n>] [--timeout <ms>] [--corpus <dir>] [--no-save]
  maria-fuzz replay  <file.sv> [--target <t>] [--timeout <ms>]
  maria-fuzz triage  <dir>
  maria-fuzz minimize <file.sv> [--target <t>] [--timeout <ms>]
  maria-fuzz report  [<dir>]
  maria-fuzz help

TARGETS: all | lexer | parser | elab | sim | fmt | cli
Default: all (2000 cases/target). Corpus default: {corpus}
Bug output: {bugs}
"#,
        version = maria_fuzz::VERSION,
        corpus = default_corpus_dir().display(),
        bugs = bugs_dir().display(),
    );
}

fn parse_target(s: &str) -> Option<Target> {
    Target::from_str(s)
}

/// Cari nilai flag di args (mis. "--seed" → value), hapus dari slice.
fn take_flag(args: &[String], flag: &str) -> (Option<String>, Vec<String>) {
    let mut val = None;
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == flag && i + 1 < args.len() {
            val = Some(args[i + 1].clone());
            i += 2;
        } else {
            rest.push(args[i].clone());
            i += 1;
        }
    }
    (val, rest)
}

fn cmd_run(args: &[String]) -> i32 {
    let (target_s, rest) = take_flag(args, "--target");
    let (cases_s, rest) = take_flag(&rest, "-c");
    let (seed_s, rest) = take_flag(&rest, "--seed");
    let (timeout_s, rest) = take_flag(&rest, "--timeout");
    let (corpus_s, rest) = take_flag(&rest, "--corpus");
    let no_save = rest.iter().any(|a| a == "--no-save");

    let target = target_s
        .as_deref()
        .and_then(parse_target)
        .unwrap_or(Target::All);
    let cases = cases_s.and_then(|s| s.parse().ok()).unwrap_or(2000);
    let seed = seed_s.and_then(|s| s.parse().ok()).unwrap_or(0xC0FFEE);
    let timeout = timeout_s.and_then(|s| s.parse().ok()).unwrap_or(2000);
    let corpus_dir = corpus_s.map(PathBuf::from);

    let cfg = FuzzConfig {
        target,
        cases,
        seed,
        timeout_ms: timeout,
        corpus_dir,
        save_bugs: !no_save,
    };

    eprintln!(
        "KAMPANYE: target={} cases={} seed={:#x} timeout={}ms",
        target.as_str(),
        cases,
        seed,
        timeout
    );
    let report = run(cfg);
    let text = triage::render_report(&report.bugs);
    println!("{text}");

    // Ringkasan per target
    for s in &report.summaries {
        let mut parts: Vec<String> = s
            .categories
            .iter()
            .map(|(c, n)| format!("{}={}", c.label(), n))
            .collect();
        parts.sort();
        println!(
            "[{}] total={} bugs={} :: {}",
            s.target.as_str(),
            s.total,
            s.bugs,
            parts.join(" ")
        );
    }
    if !report.bugs.is_empty() {
        let p = triage::save_report(&report.bugs);
        if let Ok(p) = p {
            eprintln!("laporan: {}", p.display());
        }
    }

    if report.bugs.is_empty() {
        0
    } else {
        1
    }
}

fn cmd_replay(args: &[String]) -> i32 {
    let (target_s, rest) = take_flag(args, "--target");
    let (timeout_s, rest) = take_flag(&rest, "--timeout");
    let file = rest.first().cloned().unwrap_or_default();
    if file.is_empty() {
        eprintln!("usage: maria-fuzz replay <file.sv> [--target <t>] [--timeout <ms>]");
        return 1;
    }
    let source = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("gagal baca {file}: {e}");
            return 1;
        }
    };
    let target = target_s
        .as_deref()
        .and_then(parse_target)
        .unwrap_or(Target::Simulator);
    let timeout = timeout_s.and_then(|s| s.parse().ok()).unwrap_or(2000);

    eprintln!("REPLAY: {file} -> target={}", target.as_str());
    let result = maria_fuzz::oracle::evaluate(target, &source, timeout);
    println!("category: {}", result.category.label());
    println!("oracle:   {}", result.oracle);
    println!("detail:   {}", result.detail.lines().next().unwrap_or(""));
    if result.category.is_bug() {
        1
    } else {
        0
    }
}

fn cmd_triage(args: &[String]) -> i32 {
    let dir = args.first().cloned().unwrap_or_else(|| bugs_dir().to_string_lossy().to_string());

    let mut results = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("sv") {
                continue;
            }
            // butuh pasangan .txt metadata
            let txt = p.with_extension("txt");
            let meta = std::fs::read_to_string(&txt).unwrap_or_default();
            let category = meta
                .lines()
                .find(|l| l.starts_with("category: "))
                .and_then(|l| l.split(": ").nth(1))
                .and_then(Category::from_label)
                .unwrap_or(Category::Panic);

            let source = std::fs::read_to_string(&p).unwrap_or_default();
            results.push(CaseResult {
                target: Target::Parser,
                category,
                oracle: "manual",
                detail: meta.lines().find(|l| l.starts_with("detail: ")).unwrap_or("").to_string(),
                source,
            });
        }
    } else {
        eprintln!("dir tidak ada: {dir}");
        return 1;
    }

    println!("{}", triage::render_report(&results));
    0
}

fn cmd_minimize(args: &[String]) -> i32 {
    let (target_s, rest) = take_flag(args, "--target");
    let (timeout_s, rest) = take_flag(&rest, "--timeout");
    let file = rest.first().cloned().unwrap_or_default();
    if file.is_empty() {
        eprintln!("usage: maria-fuzz minimize <file.sv> [--target <t>] [--timeout <ms>]");
        return 1;
    }
    let source = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("gagal baca {file}: {e}");
            return 1;
        }
    };
    let target = target_s
        .as_deref()
        .and_then(parse_target)
        .unwrap_or(Target::Simulator);
    let timeout = timeout_s.and_then(|s| s.parse().ok()).unwrap_or(2000);

    // Predikat: bug ter-reproduksi (bukan Ok/CleanError)
    let predicate = |s: &str| {
        let r = maria_fuzz::oracle::evaluate(target, s, timeout);
        r.category.is_bug()
    };

    eprintln!(
        "MINIMIZE: {file} ({} bytes) target={}",
        source.len(),
        target.as_str()
    );
    if !predicate(&source) {
        eprintln!("source asli TIDAK reproducible — tidak bisa minimize");
        return 1;
    }

    let min = minimize::ddmin(&source, &predicate);
    eprintln!("minimized: {} -> {} bytes", source.len(), min.len());
    println!("{min}");

    let out = file.replace(".sv", "_min.sv");
    let _ = std::fs::write(&out, &min);
    eprintln!("tersimpan: {out}");
    0
}

fn cmd_report(args: &[String]) -> i32 {
    // Baca semua bug dari bugs dir (pakai metadata .txt), lalu render.
    let mut results: Vec<CaseResult> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(bugs_dir()) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("sv") {
                continue;
            }
            let txt = p.with_extension("txt");
            let meta = std::fs::read_to_string(&txt).unwrap_or_default();
            let category = meta
                .lines()
                .find(|l| l.starts_with("category: "))
                .and_then(|l| l.split(": ").nth(1))
                .and_then(Category::from_label)
                .unwrap_or(Category::Panic);
            let source = std::fs::read_to_string(&p).unwrap_or_default();
            results.push(CaseResult {
                target: Target::Parser,
                category,
                oracle: "manual",
                detail: String::new(),
                source,
            });
        }
    }
    println!("{}", triage::render_report(&results));
    let _ = args;
    0
}