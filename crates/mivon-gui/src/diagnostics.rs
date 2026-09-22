//! Diagnostics & crash-log — "jangan tebak-tebak": tiap masalah dicatat dengan
//! info selengkap mungkin (fase startup, nama thread, lokasi, backtrace) ke
//! file log di direktori konfigurasi, di samping stderr.
//!
//! - `gui.log`  — fase startup (post-mortem: fase terakhir sebelum crash).
//! - `crash.log` — panic report lengkap (dari panic hook) + ditampilkan
//!   ringkasannya di Console saat startup berikutnya.
//!
//! Stack overflow (abort) tidak bisa di-`catch`, tetapi dengan memberi nama
//! pada SEMUA thread (rayon + worker GUI) pesan abort OS menjadi
//! `thread 'rayon-parse-3' has overflowed its stack` — identitas thread
//! langsung terlihat, bukan `<unknown>`.

use std::io::Write;
use std::path::PathBuf;

/// Direktori konfigurasi pengguna ($XDG_CONFIG_HOME atau ~/.config) — konsisten
/// dengan `last_workspace.json` (workspace.rs).
pub fn config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config");
    }
    std::env::temp_dir()
}

/// Path log fase startup: `<config>/mivon/gui.log`.
pub fn gui_log_path() -> PathBuf {
    config_dir().join("mivon").join("gui.log")
}

/// Path crash report: `<config>/mivon/crash.log`.
pub fn crash_log_path() -> PathBuf {
    config_dir().join("mivon").join("crash.log")
}

fn append(path: &PathBuf, text: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{}", text);
    }
}

fn timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

/// Catat satu fase startup ke `gui.log` (append, timestamp). Saat crash, fase
/// terakhir yang tercatat = lokasi crash paling mungkin.
pub fn log(msg: impl AsRef<str>) {
    let line = format!("[{}] {}", timestamp(), msg.as_ref());
    append(&gui_log_path(), &line);
}

/// Pasang panic hook: selain pesan stderr (meniru default hook), tulis crash
/// report LENGKAP — payload, lokasi, nama thread, backtrace (`force_capture`,
/// jalan juga di release) — ke `crash.log`. Ini menggantikan hook default,
/// jadi perilaku pesan stderr ditiru supaya tidak ada yang hilang.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "panic (payload non-string)".to_string()
        };
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "lokasi tidak diketahui".to_string());
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>").to_string();
        let bt = std::backtrace::Backtrace::force_capture();

        // Mimik pesan stderr default Rust + detail tambahan.
        eprintln!(
            "thread '{}' panicked at {}:\n{}",
            thread_name, location, payload
        );
        eprintln!("  → crash report lengkap: {}", crash_log_path().display());

        let report = format!(
            "[{}] PANIC\n  thread:   {}\n  location: {}\n  payload:  {}\n  ==== backtrace ====\n{}",
            timestamp(),
            thread_name,
            location,
            payload,
            bt
        );
        append(&crash_log_path(), &report);
    }));
}

/// Ringkasan crash TERAKHIR (beberapa baris teratas report terakhir di
/// `crash.log`) — ditampilkan di Console saat startup berikutnya, agar masalah
/// sebelumnya punya jejak yang bisa dibaca, bukan hilang begitu saja.
pub fn last_crash_summary() -> Option<String> {
    let text = std::fs::read_to_string(crash_log_path()).ok()?;
    if text.trim().is_empty() {
        return None;
    }
    // Ambil blok laporan TERAKHIR: mulai dari line "[...] PANIC" terakhir.
    let report_start = text
        .rfind("] PANIC")
        .map(|i| i + "] PANIC".len())
        .unwrap_or(0);
    let lines: Vec<&str> = text[report_start..].lines().collect();
    let tail: Vec<&str> = lines.iter().take(14).copied().collect();
    Some(format!(
        "{} (lihat {} untuk backtrace penuh)",
        tail.join("\n"),
        crash_log_path().display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_honors_xdg() {
        std::env::set_var("XDG_CONFIG_HOME", "/tmp/xdg-test");
        std::env::set_var("HOME", "/tmp/home-test");
        assert_eq!(config_dir(), PathBuf::from("/tmp/xdg-test"));
        std::env::remove_var("XDG_CONFIG_HOME");
        assert_eq!(
            config_dir(),
            PathBuf::from("/tmp/home-test").join(".config")
        );
    }

    #[test]
    fn log_paths_live_in_mivon_dir() {
        assert!(gui_log_path().ends_with("gui.log"));
        assert!(crash_log_path().ends_with("crash.log"));
    }
}
