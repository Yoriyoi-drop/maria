//! Runner subprocess: jalankan binary `maria` sebagai black-box.
//!
//! Miller 1990-style: timeout via kill, pipe drain via 2 reader threads
//! (cegah false-hang pipe-full 64KB).

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

static SEQ: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Ok,
    CleanError,
    Panic,
    Abort,
    Crash(i32),
    Hang,
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub kind: Kind,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub ms: u128,
}

/// Jalankan `maria <file>` dengan timeout.
pub fn run_file(source: &str, timeout_ms: u64) -> Outcome {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "mariafz_{}_{}.sv",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    if std::fs::write(&path, source).is_err() {
        return Outcome {
            kind: Kind::CleanError,
            code: None,
            stdout: String::new(),
            stderr: "gagal tulis file temp".to_string(),
            ms: 0,
        };
    }

    let bin = find_maria();
    let mut cmd = Command::new(&bin);
    cmd.arg(&path)
        .arg("-T")
        .arg("1000")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let outcome = spawn(&mut cmd, timeout_ms);
    let _ = std::fs::remove_file(&path);
    outcome
}

/// Jalankan `maria <args...>` di cwd temp (target Cli).
pub fn run_args(args: &[String], timeout_ms: u64) -> Outcome {
    let bin = find_maria();
    let mut cmd = Command::new(&bin);
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    spawn(&mut cmd, timeout_ms)
}

/// Cari binary maria: env MARIA_BIN, release (LTO, cepat), debug, atau workspace.
pub fn find_maria() -> String {
    if let Ok(bin) = std::env::var("MARIA_BIN") {
        return bin;
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // current_exe = target/{debug,release}/maria-fuzz → dir = target/{debug,release}.
            // RELEASE dulu (LTO cepat), lalu debug, lalu sibling same-dir.
            if let Some(target) = dir.parent() {
                // target/release dan target/debug adalah PASANGAN dari dir ini
                let rel = target.join("release/maria");
                let dbg = target.join("debug/maria");
                if rel.exists() {
                    return rel.to_string_lossy().to_string();
                }
                if dbg.exists() {
                    return dbg.to_string_lossy().to_string();
                }
            }
            // Fallback same-dir (binary diinstall berdampingan)
            let cand = dir.join("maria");
            if cand.exists() {
                return cand.to_string_lossy().to_string();
            }
        }
    }
    // Workspace root walk (cari Cargo.toml dengan [workspace]),
    // RELEASE dulu (LTO — startup/eksekusi jauh lebih cepat).
    let mut dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    loop {
        let release = dir.join("target/release/maria");
        let debug = dir.join("target/debug/maria");
        if release.exists() {
            return release.to_string_lossy().to_string();
        }
        if debug.exists() {
            return debug.to_string_lossy().to_string();
        }
        if !dir.pop() {
            break;
        }
    }
    "maria".to_string()
}

fn spawn(cmd: &mut Command, timeout_ms: u64) -> Outcome {
    let start = Instant::now();
    let mut child: Child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Outcome {
                kind: Kind::CleanError,
                code: None,
                stdout: String::new(),
                stderr: format!("spawn gagal: {e}"),
                ms: start.elapsed().as_millis(),
            };
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Drain pipe via reader thread — cegah pipe-full false-hang
    let out_handle = std::thread::spawn(move || {
        read_pipe(stdout)
    });
    let err_handle = std::thread::spawn(move || {
        read_pipe(stderr)
    });

    let timeout = Duration::from_millis(timeout_ms);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(_) => break None,
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            break None; // hang
        }
        std::thread::sleep(Duration::from_millis(5));
    };

    let stdout = out_handle.join().unwrap_or_default();
    let stderr = err_handle.join().unwrap_or_default();
    let ms = start.elapsed().as_millis();

    let (kind, code) = match status {
        Some(status) => {
            let code = status.code();
            classify(code, &stderr)
        }
        None => (Kind::Hang, None),
    };

    Outcome {
        kind,
        code,
        stdout,
        stderr,
        ms,
    }
}

fn read_pipe<R: std::io::Read + Send + 'static>(pipe: Option<R>) -> String {
    let mut buf = String::new();
    if let Some(mut p) = pipe {
        let _ = p.read_to_string(&mut buf);
    }
    buf
}

fn classify(code: Option<i32>, stderr: &str) -> (Kind, Option<i32>) {
    let Some(code) = code else {
        return (Kind::CleanError, None);
    };
    match code {
        0 => (Kind::Ok, Some(0)),
        // maria CLI pakai exit code untuk error bersih
        1 if !stderr.contains("panic") => (Kind::CleanError, Some(1)),
        101 => (Kind::Panic, Some(101)),
        132 | 134 => (Kind::Abort, Some(code)),
        139 => (Kind::Crash(139), Some(139)),
        c if c >= 128 => (Kind::Crash(c), Some(c)),
        _ => (Kind::CleanError, Some(code)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_exit_codes() {
        assert_eq!(classify(Some(0), "").0, Kind::Ok);
        assert_eq!(classify(Some(1), "error: syntax").0, Kind::CleanError);
        assert_eq!(classify(Some(101), "panicked at").0, Kind::Panic);
        assert_eq!(classify(Some(139), "").0, Kind::Crash(139));
        assert_eq!(classify(None, "").0, Kind::CleanError);
    }

    #[test]
    fn find_maria_returns_something() {
        assert!(!find_maria().is_empty());
    }
}