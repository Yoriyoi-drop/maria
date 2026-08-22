//! MDME Binary — Jalankan Maria Directed Mutation Engine untuk menemukan bug di Maria.
//!
//! Usage:
//!   cargo run -p maria-tests --example mdme_run -- [OPTIONS]
//!
//! Options:
//!   --iterations N       Max iterations (default: 10000)
//!   --timeout N          Timeout per testcase in seconds (default: 30)
//!   --memory N           Memory limit in bytes (default: 2GB)
//!   --population N       Population size (default: 200)
//!   --offspring N        Offspring per generation (default: 100)
//!   --mode MODE          Mode: parser, semantic, type, elaboration, hierarchy, generate, simulator, differential, resource, full (default: full)
//!   --corpus-dir PATH    Corpus directory (default: .mdme_corpus)
//!   --seed-dir PATH      Seed directory (default: test)
//!   --minimizer          Enable minimizer (default: true)
//!   --min-timeout N      Minimizer timeout in seconds (default: 60)
//!   --exploration N      Bandit exploration parameter (default: 1.0)
//!   --report PATH        Output report JSON path (default: mdme_report.json)
//!   --runs N             Number of parallel runs (default: 1, multi-run uses thread pool)
//!   --merge-corpus       Merge corpus from all runs (default: true)

use maria_tests::mdme::{MdmeEngine, MdmeConfig, MdmeMode, MdmeReport};
use std::path::PathBuf;
use std::fs;
use std::time::Instant;
use std::thread;
use std::sync::{Arc, Mutex};

fn parse_mode(s: &str) -> MdmeMode {
    match s.to_lowercase().as_str() {
        "parser" => MdmeMode::Parser,
        "semantic" => MdmeMode::Semantic,
        "type" => MdmeMode::Type,
        "elaboration" => MdmeMode::Elaboration,
        "hierarchy" => MdmeMode::Hierarchy,
        "generate" => MdmeMode::Generate,
        "simulator" => MdmeMode::Simulator,
        "differential" => MdmeMode::Differential,
        "resource" => MdmeMode::Resource,
        "full" => MdmeMode::Full,
        _ => {
            eprintln!("Unknown mode: {}, using Full", s);
            MdmeMode::Full
        }
    }
}

fn print_usage() {
    eprintln!(
        r#"MDME Binary — Maria Directed Mutation Engine

Usage:
  cargo run -p maria-tests --example mdme_run -- [OPTIONS]

Options:
  --iterations N       Max iterations (default: 10000)
  --timeout N          Timeout per testcase in seconds (default: 30)
  --memory N           Memory limit in bytes (default: 2147483648 = 2GB)
  --population N       Population size (default: 200)
  --offspring N        Offspring per generation (default: 100)
  --mode MODE          Mode: parser, semantic, type, elaboration, hierarchy, generate, simulator, differential, resource, full (default: full)
  --corpus-dir PATH    Corpus directory (default: .mdme_corpus)
  --seed-dir PATH      Seed directory (default: test)
  --minimizer          Enable minimizer (default: true)
  --no-minimizer       Disable minimizer
  --min-timeout N      Minimizer timeout in seconds (default: 60)
  --exploration N      Bandit exploration parameter (default: 1.0)
  --report PATH        Output report JSON path (default: mdme_report.json)
  --help               Show this help
"#
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut config = MdmeConfig::default();
    let mut report_path = "mdme_report.json".to_string();
    let mut show_help = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--iterations" => {
                if i + 1 < args.len() {
                    config.max_iterations = args[i + 1].parse().unwrap_or(config.max_iterations);
                    i += 1;
                }
            }
            "--timeout" => {
                if i + 1 < args.len() {
                    config.timeout_per_testcase_secs = args[i + 1].parse().unwrap_or(config.timeout_per_testcase_secs);
                    i += 1;
                }
            }
            "--memory" => {
                if i + 1 < args.len() {
                    config.memory_limit_bytes = args[i + 1].parse().unwrap_or(config.memory_limit_bytes);
                    i += 1;
                }
            }
            "--population" => {
                if i + 1 < args.len() {
                    config.population_size = args[i + 1].parse().unwrap_or(config.population_size);
                    i += 1;
                }
            }
            "--offspring" => {
                if i + 1 < args.len() {
                    config.offspring_per_generation = args[i + 1].parse().unwrap_or(config.offspring_per_generation);
                    i += 1;
                }
            }
            "--mode" => {
                if i + 1 < args.len() {
                    config.mode = parse_mode(&args[i + 1]);
                    i += 1;
                }
            }
            "--corpus-dir" => {
                if i + 1 < args.len() {
                    config.corpus_dir = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
            }
            "--seed-dir" => {
                if i + 1 < args.len() {
                    config.seed_corpus_paths = vec![PathBuf::from(&args[i + 1])];
                    i += 1;
                }
            }
            "--minimizer" => config.enable_minimizer = true,
            "--no-minimizer" => config.enable_minimizer = false,
            "--min-timeout" => {
                if i + 1 < args.len() {
                    config.minimizer_timeout_secs = args[i + 1].parse().unwrap_or(config.minimizer_timeout_secs);
                    i += 1;
                }
            }
            "--exploration" => {
                if i + 1 < args.len() {
                    config.bandit_exploration = args[i + 1].parse().unwrap_or(config.bandit_exploration);
                    i += 1;
                }
            }
            "--report" => {
                if i + 1 < args.len() {
                    report_path = args[i + 1].to_string();
                    i += 1;
                }
            }
            "--runs" => {
                if i + 1 < args.len() {
                    config.num_runs = args[i + 1].parse().unwrap_or(1);
                    i += 1;
                }
            }
            "--merge-corpus" => config.merge_corpus = true,
            "--no-merge-corpus" => config.merge_corpus = false,
            "--help" | "-h" => show_help = true,
            _ => {
                eprintln!("Unknown argument: {}", args[i]);
                show_help = true;
            }
        }
        i += 1;
    }

    if show_help {
        print_usage();
        std::process::exit(0);
    }

    eprintln!("[MDME] Starting with config:");
    eprintln!("  Mode: {:?}", config.mode);
    eprintln!("  Max iterations: {}", config.max_iterations);
    eprintln!("  Timeout/testcase: {}s", config.timeout_per_testcase_secs);
    eprintln!("  Memory limit: {} MB", config.memory_limit_bytes / (1024 * 1024));
    eprintln!("  Population: {}", config.population_size);
    eprintln!("  Offspring/gen: {}", config.offspring_per_generation);
    eprintln!("  Minimizer: {}", config.enable_minimizer);
    eprintln!("  Corpus dir: {}", config.corpus_dir.display());
    eprintln!("  Seed dirs: {:?}", config.seed_corpus_paths);

    // Save corpus_dir for later use (config will be moved)
    let corpus_dir = config.corpus_dir.clone();

    // Buat engine
    let mut engine = match MdmeEngine::new(config) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[MDME] Failed to create engine: {}", e);
            std::process::exit(1);
        }
    };

    // Jalankan
    let start = Instant::now();
    eprintln!("[MDME] Running...");

    let report = match engine.run() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[MDME] Engine error: {}", e);
            std::process::exit(1);
        }
    };

    let elapsed = start.elapsed();

    // Print summary
    eprintln!("\n[MDME] ==================== REPORT ====================");
    eprintln!("Time elapsed: {:.2}s", elapsed.as_secs_f64());
    eprintln!("Total iterations: {}", report.iterations.len());
    eprintln!("Final corpus size: {}", report.final_corpus_size);
    eprintln!("Total coverage edges: {}", report.final_coverage_edges);
    eprintln!("");
    eprintln!("BUGS FOUND:");
    eprintln!("  Internal Crashes:     {}", report.total_crashes);
    eprintln!("  Hangs/Deadlocks:      {}", report.total_hangs);
    eprintln!("  Out of Memory:        {}", report.total_ooms);
    eprintln!("  Differential Mismatches: {}", report.total_differential_mismatches);
    eprintln!("  Total unique bugs:    {}", engine.bugs().len());

    if !engine.bugs().is_empty() {
        eprintln!("\n[MDME] DETAILED BUG REPORTS:");
        for (idx, bug) in engine.bugs().iter().enumerate() {
            eprintln!("\n  --- Bug #{} ---", idx + 1);
            eprintln!("  Category: {:?}", bug.category);
            eprintln!("  Generation: {}", bug.generation);
            eprintln!("  Operator: {}", bug.operator_used);
            eprintln!("  Risk score: {:.2}", bug.risk_score);
            eprintln!("  Description: {}", bug.description);
            if let Some(min) = &bug.minimal_reproducer {
                eprintln!("  Minimal reproducer ({} bytes):", min.len());
                eprintln!("{}", min);
                eprintln!("  (saved to corpus as min_...)");
            } else {
                eprintln!("  Source ({} bytes):", bug.source.len());
                eprintln!("{}", bug.source);
            }
        }
    } else {
        eprintln!("\n[MDME] No bugs found in this run.");
    }

    // Save JSON report
    let json_report = serde_json::json!({
        "summary": {
            "time_elapsed_secs": elapsed.as_secs_f64(),
            "total_iterations": report.iterations.len(),
            "final_corpus_size": report.final_corpus_size,
            "total_coverage_edges": report.final_coverage_edges,
            "total_crashes": report.total_crashes,
            "total_hangs": report.total_hangs,
            "total_ooms": report.total_ooms,
            "total_differential_mismatches": report.total_differential_mismatches,
            "total_bugs": engine.bugs().len(),
        },
        "bugs": engine.bugs().iter().map(|b| serde_json::json!({
            "category": format!("{:?}", b.category),
            "generation": b.generation,
            "operator": b.operator_used,
            "risk_score": b.risk_score,
            "description": b.description,
            "has_minimal_reproducer": b.minimal_reproducer.is_some(),
            "source_len": b.source.len(),
            "minimal_reproducer_len": b.minimal_reproducer.as_ref().map(|s| s.len()),
        })).collect::<Vec<_>>(),
        "iterations": report.iterations.iter().map(|i| serde_json::json!({
            "iteration": i.iteration,
            "offspring_count": i.offspring_count,
            "new_interesting": i.new_interesting,
            "corpus_size": i.corpus_size,
            "coverage_edges": i.coverage_edges,
        })).collect::<Vec<_>>(),
    });

    if let Err(e) = fs::write(&report_path, serde_json::to_string_pretty(&json_report).unwrap()) {
        eprintln!("[MDME] Warning: Failed to write report to {}: {}", report_path, e);
    } else {
        eprintln!("\n[MDME] Full report saved to: {}", report_path);
    }

    eprintln!("[MDME] Corpus saved to: {}", corpus_dir.display());

    // Exit code: 1 if bugs found, 0 if clean
    if engine.bugs().is_empty() {
        std::process::exit(0);
    } else {
        std::process::exit(1);
    }
}