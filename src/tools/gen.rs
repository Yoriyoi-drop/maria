//! `mgen` — Generator SystemVerilog dari Maria HDL (.mv).
//!
//! Membaca file `.mv` (atau direktori berisi `.mv`), me-transpile ke
//! `.sv` + `.svh` (MARIA-HDL.md). Output deterministik — bisa di-commit
//! ke repo dan di-`--check` di CI.
//!
//! Mode:
//! - default : tulis `<base>.sv` + `<base>.svh` di direktori input (atau `-o`)
//! - `--stdout` : print `.sv` ke stdout (debug)
//! - `--check`  : verifikasi output up-to-date — exit 1 bila beda (CI)
//! - `--svh-only` / `--sv-only` : hanya satu file output

use std::path::{Path, PathBuf};

use crate::error::SimError;
use crate::mv;

/// Opsi mgen.
pub struct GenArgs<'a> {
    pub targets: &'a [String],
    pub output: Option<String>,
    pub stdout: bool,
    pub check: bool,
    pub svh_only: bool,
    pub sv_only: bool,
    pub no_check: bool,
    pub verbose: bool,
}

fn diag(msg: impl Into<String>) -> SimError {
    SimError::with_diag(crate::diagnostics::DiagCode::InvalidSyntax, msg)
}

/// Jalankan mgen.
pub fn run(args: &GenArgs) -> Result<(), SimError> {
    if args.stdout && args.targets.len() != 1 {
        return Err(diag("--stdout hanya untuk satu file .mv"));
    }
    if args.stdout && args.check {
        return Err(diag("--stdout tidak bisa digabung dengan --check"));
    }

    let files = collect_mv_files(args.targets)?;
    if files.is_empty() {
        return Err(diag("tidak ada file .mv ditemukan"));
    }

    let out_dir: Option<PathBuf> = args.output.as_ref().map(PathBuf::from);
    if let Some(d) = &out_dir {
        if !d.exists() {
            std::fs::create_dir_all(d).map_err(|e| diag(format!("tidak bisa membuat direktori '{}': {}", d.display(), e)))?;
        }
    }

    // ── F9: transpile batch (konteks gabungan lintas file) ──
    // Semua file dibaca dulu, lalu di-transpile BERSAMA — tipe/package dari
    // satu file terlihat oleh file lain (`types.mv` → `counter.mv`).
    let mut items: Vec<(String, String)> = Vec::with_capacity(files.len());
    for path in &files {
        let base = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| diag(format!("nama file tidak valid: '{}'", path.display())))?
            .to_string();
        let src = std::fs::read_to_string(path)
            .map_err(|e| diag(format!("{}: {}", path.display(), e)))?;
        items.push((src, base));
    }
    let results = if args.no_check {
        mv::transpile_many_no_check(&items)
    } else {
        mv::transpile_many(&items)
    }
    .map_err(|(i, e)| diag(mv::format_error(&files[i].display().to_string(), &items[i].0, &e)))?;
    // Defensif: hasil batch harus sejajar dengan input (jangan zip-truncate).
    assert_eq!(
        results.len(),
        files.len(),
        "transpile_many harus mengembalikan hasil sejajar dengan input"
    );

    let mut changed_any = false;
    for (path, result) in files.iter().zip(results.iter()) {
        let base = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| diag(format!("nama file tidak valid: '{}'", path.display())))?;

        // ── --stdout: print .sv ──
        if args.stdout {
            print!("{}", result.sv);
            continue;
        }

        let svh_path = target_path(&out_dir, path, base, "svh");
        let sv_path = target_path(&out_dir, path, base, "sv");

        // ── --check: bandingkan dengan file existing ──
        // `.svh` kosong = file tidak membutuhkan definisi bersama → tidak ada
        // file yang diharapkan (konsisten dengan generate_svh yang skip).
        if args.check {
            let svh_ok = result.svh.is_empty() || file_matches(&svh_path, &result.svh);
            let sv_ok = file_matches(&sv_path, &result.sv);
            let ok = if args.svh_only {
                svh_ok
            } else if args.sv_only {
                sv_ok
            } else {
                svh_ok && sv_ok
            };
            if ok {
                if args.verbose {
                    println!("  ✓ {} — up-to-date", path.display());
                }
            } else {
                println!("  ! {} — perlu regenerate", path.display());
                changed_any = true;
            }
            continue;
        }

        // ── tulis file ──
        // `.svh` kosong (tanpa package/typedef) → jangan tulis file sama sekali.
        if !args.sv_only && !result.svh.is_empty() {
            let changed = write_if_changed(&svh_path, &result.svh)?;
            if changed || args.verbose {
                println!("  generated {}", svh_path.display());
            }
            changed_any |= changed;
        } else if args.svh_only && result.svh.is_empty() && !args.stdout && !args.check {
            println!("  (skip .svh — file tidak punya package/typedef)");
        }
        if !args.svh_only {
            let changed = write_if_changed(&sv_path, &result.sv)?;
            if changed || args.verbose {
                println!("  generated {}", sv_path.display());
            }
            changed_any |= changed;
        }
    }

    if args.check && changed_any {
        return Err(diag("mgen --check: ada file .mv yang belum di-generate — jalankan `maria mgen <file.mv>`"));
    }
    if !args.stdout && !args.check && !args.verbose {
        println!("mgen: {} file .mv diproses", files.len());
    }
    Ok(())
}

/// Kumpulkan file `.mv` dari target (file atau direktori recursive).
fn collect_mv_files(targets: &[String]) -> Result<Vec<PathBuf>, SimError> {
    let mut out: Vec<PathBuf> = Vec::new();
    for t in targets {
        let p = Path::new(t);
        if !p.exists() {
            return Err(diag(format!("path tidak ditemukan: '{}'", t)));
        }
        if p.is_dir() {
            collect_dir(p, &mut out)?;
        } else {
            out.push(p.to_path_buf());
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn collect_dir(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), SimError> {
    for entry in std::fs::read_dir(dir).map_err(|e| diag(format!("{}: {}", dir.display(), e)))? {
        let path = entry.map_err(|e| diag(e.to_string()))?.path();
        if path.is_dir() {
            collect_dir(&path, out)?;
        } else if path
            .extension()
            .map(|e| e == "mv")
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
    Ok(())
}

/// Path output: `-o dir` → dir, selain itu di samping file input.
fn target_path(out_dir: &Option<PathBuf>, input: &Path, base: &str, ext: &str) -> PathBuf {
    match out_dir {
        Some(d) => d.join(format!("{base}.{ext}")),
        None => input.with_file_name(format!("{base}.{ext}")),
    }
}

fn file_matches(path: &Path, content: &str) -> bool {
    std::fs::read_to_string(path).map(|s| s == content).unwrap_or(false)
}

/// Tulis hanya bila konten berubah (deterministik + tidak sentuh mtime bila sama).
fn write_if_changed(path: &Path, content: &str) -> Result<bool, SimError> {
    if file_matches(path, content) {
        return Ok(false);
    }
    std::fs::write(path, content).map_err(|e| diag(format!("{}: {}", path.display(), e)))?;
    Ok(true)
}
