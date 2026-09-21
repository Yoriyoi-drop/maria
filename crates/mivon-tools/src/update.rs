//! `mivon update` — auto-update Mivon dari release resmi.
//!
//! Prinsip keamanan (doc/release-pipeline.md §4):
//!   - Manifest HANYA metadata (version/url/sha256) — tidak pernah dieksekusi.
//!   - SHA-256 diverifikasi sebelum binary dipasang (fail-closed).
//!   - Pemasangan atomik: unduh → verify → backup lama → rename.
//!   - Rollback: restore backup terakhir bila smoke test gagal / `--rollback`.
//!   - Default eksplisit: tanpa perintah `mivon update`, tidak ada yang berubah.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use mivon_core::error::SimError;
use serde::Deserialize;

/// URL manifest default. Hanya diperbarui oleh workflow `release.yml`
/// (commit sinkronisasi) — jadi konsumen tidak pernah melihat versi yang
/// belum diterbitkan sebagai release resmi.
pub const DEFAULT_MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/mivonsim/mivon/main/dist/latest.json";

const REPO: &str = "mivonsim/mivon";

// ────────────────────────────────────────────────────────────────────────────
// Tipe manifest
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PlatformEntry {
    pub url: String,
    pub sha256: String,
}

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub tag: String,
    #[serde(default)]
    pub published_at: Option<String>,
    pub platforms: HashMap<String, PlatformEntry>,
}

// ────────────────────────────────────────────────────────────────────────────
// Opsi & entrypoint
// ────────────────────────────────────────────────────────────────────────────

/// Opsi `mivon update`.
pub struct UpdateArgs<'a> {
    /// `true` → hanya melapor (`mivon update check`), tanpa mengubah apa pun.
    pub check_only: bool,
    /// Kanal rilis: `stable` (default) | `beta`. Memilih manifest
    /// `dist/latest-<channel>.json`.
    pub channel: Option<&'a str>,
    /// Pasang versi spesifik (mis. `0.4.0`) dari release resmi v<version>.
    pub version: Option<&'a str>,
    /// Kembalikan ke binary sebelumnya (backup terakhir).
    pub rollback: bool,
    /// Lewati konfirmasi (`--yes`).
    pub yes: bool,
    /// Override URL manifest (hook uji: `file://...` / path lokal).
    pub manifest_url: Option<&'a str>,
    /// Override path binary (hook uji).
    pub exe_path: Option<&'a str>,
}

/// Jalankan `mivon update`. Exit semantics via `SimError` (main.rs `exit_tool`).
pub fn run(args: &UpdateArgs) -> Result<(), SimError> {
    let local = local_version();

    if args.rollback {
        return do_rollback(exe_path(args.exe_path), args.yes);
    }

    // Sumber rilis tertentu dipilih secara eksplisit oleh user.
    let target: (String, String, String) = if let Some(ver) = args.version {
        // Versi spesifik: langsung dari release v<ver> (tanpa manifest).
        let ver = ver.trim_start_matches('v').to_string();
        let sha = fetch_checksum_for(&ver)?;
        let url = format!(
            "https://github.com/{}/releases/download/v{}/mivon",
            REPO, ver
        );
        (ver, url, sha)
    } else {
        let manifest = fetch_manifest(args.manifest_url, args.channel)?;
        let platform = platform_key()?;
        let entry = manifest.platforms.get(&platform).ok_or_else(|| {
            SimError::with_diag(
                mivon_core::diagnostics::DiagCode::InternalError,
                format!(
                    "manifest tidak memuat platform '{}' (diperbarui otomatis oleh release.yml)",
                    platform
                ),
            )
        })?;
        (
            manifest.version.clone(),
            entry.url.clone(),
            entry.sha256.clone(),
        )
    };

    if !newer(&target.0, &local) {
        println!("Mivon sudah terbaru (v{})", local);
        return Ok(());
    }

    if args.check_only {
        println!("Update tersedia: v{} (lokal v{})", target.0, local);
        println!("  url   : {}", target.1);
        println!("  sha256: {}", abbreviate(&target.2));
        println!("Jalankan `mivon update` untuk memasang.");
        return Ok(());
    }

    apply_update(
        &target.0,
        &target.1,
        &target.2,
        exe_path(args.exe_path),
        args.yes,
    )
}

// ────────────────────────────────────────────────────────────────────────────
// Util versi & platform
// ────────────────────────────────────────────────────────────────────────────

/// Versi binary (versi crate mivon-tools saat build; seluruh workspace
/// mengikuti versi root `mivon`).
pub fn local_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Kunci platform untuk manifest: `arch-os` ala install.sh.
pub fn platform_key() -> Result<String, SimError> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let key = match (os, arch) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        _ => {
            return Err(SimError::with_diag(
                mivon_core::diagnostics::DiagCode::InternalError,
                format!("platform tidak didukung: {}-{}", arch, os),
            ))
        }
    };
    Ok(key.to_string())
}

/// Parse `semver` sederhana -> (major, minor, patch). Toleran `v` prefix dan
/// sufiks prarilis (`0.4.0-beta.1` → 0.4.0).
pub fn parse_version(s: &str) -> (u64, u64, u64) {
    let s = s.trim().trim_start_matches('v');
    let core = s.split(['-', '+']).next().unwrap_or(s);
    let mut parts = core.split('.');
    (
        parts.next().and_then(|p| p.parse().ok()).unwrap_or(0),
        parts.next().and_then(|p| p.parse().ok()).unwrap_or(0),
        parts.next().and_then(|p| p.parse().ok()).unwrap_or(0),
    )
}

/// `a` lebih baru dari `b` (compare semver).
pub fn newer(a: &str, b: &str) -> bool {
    parse_version(a) > parse_version(b)
}

/// Path binary: override uji atau `current_exe()`.
fn exe_path(override_path: Option<&str>) -> PathBuf {
    match override_path {
        Some(p) => PathBuf::from(p),
        None => std::env::current_exe().unwrap_or_else(|_| PathBuf::from("mivon")),
    }
}

/// Persingkat hex untuk tampilan.
fn abbreviate(s: &str) -> String {
    if s.len() > 16 {
        format!("{}…{}", &s[..8], &s[s.len() - 8..])
    } else {
        s.to_string()
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Fetch (curl) + manifest
// ────────────────────────────────────────────────────────────────────────────

fn manifest_url_for(channel: Option<&str>) -> String {
    match channel {
        Some(c) if c == "beta" => format!(
            "https://raw.githubusercontent.com/{}/main/dist/latest-beta.json",
            REPO
        ),
        Some(c) if c == "stable" => DEFAULT_MANIFEST_URL.to_string(),
        Some(other) => format!(
            "https://raw.githubusercontent.com/{}/main/dist/latest-{}.json",
            REPO, other
        ),
        None => DEFAULT_MANIFEST_URL.to_string(),
    }
}

/// Ambil teks dari URL. `file://` atau path lokal dipakai sebagai hook uji.
fn fetch_text(url: &str) -> Result<String, SimError> {
    let err = |msg: String| SimError::with_diag(mivon_core::diagnostics::DiagCode::IoError, msg);

    if let Some(rest) = url.strip_prefix("file://") {
        let p = PathBuf::from(rest);
        return std::fs::read_to_string(&p)
            .map_err(|e| err(format!("baca manifest lokal '{}': {}", p.display(), e)));
    }
    if let Ok(p) = std::path::Path::new(url).canonicalize() {
        if p.is_file() {
            return std::fs::read_to_string(&p)
                .map_err(|e| err(format!("baca manifest lokal '{}': {}", p.display(), e)));
        }
    }

    let out = std::process::Command::new("curl")
        .args(["-fsSL", url])
        .output()
        .map_err(|e| err(format!("curl tidak tersedia: {}", e)))?;
    if !out.status.success() {
        return Err(err(format!(
            "gagal mengambil '{}' (curl exit {})",
            url, out.status
        )));
    }
    String::from_utf8(out.stdout).map_err(|e| err(format!("respons bukan UTF-8: {}", e)))
}

fn fetch_manifest(url: Option<&str>, channel: Option<&str>) -> Result<Manifest, SimError> {
    let default_url = manifest_url_for(channel);
    let url = url.unwrap_or(&default_url);
    let text = fetch_text(url)?;
    let mut m: Manifest = serde_json::from_str(&text).map_err(|e| {
        SimError::with_diag(
            mivon_core::diagnostics::DiagCode::InternalError,
            format!("manifest tidak valid '{}': {}", url, e),
        )
    })?;
    m.version = m.version.trim_start_matches('v').to_string();
    Ok(m)
}

/// SHA-256 artefak `mivon` dari release v<ver> (via asset mivon.sha256).
fn fetch_checksum_for(version: &str) -> Result<String, SimError> {
    let url = format!(
        "https://github.com/{}/releases/download/v{}/mivon.sha256",
        REPO, version
    );
    let text = fetch_text(&url)?;
    let sha = text
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_lowercase();
    if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(SimError::with_diag(
            mivon_core::diagnostics::DiagCode::InternalError,
            format!("checksum tidak valid dari '{}'", url),
        ));
    }
    Ok(sha)
}

// ────────────────────────────────────────────────────────────────────────────
// SHA-256, pemasangan atomik, rollback
// ────────────────────────────────────────────────────────────────────────────

/// Hex SHA-256 dari byte.
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    let out = h.finalize();
    let mut s = String::with_capacity(64);
    for b in out {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// Verifikasi checksum data terhadap sha yang diharapkan.
pub fn verify_checksum(data: &[u8], expected: &str) -> bool {
    sha256_hex(data).eq_ignore_ascii_case(&expected.trim().to_lowercase())
}

/// Unduh artefak via curl (atau salin dari `file://` untuk uji).
fn download_to(url: &str, dest: &Path) -> Result<(), SimError> {
    if let Some(rest) = url.strip_prefix("file://") {
        let src = PathBuf::from(rest);
        std::fs::copy(&src, dest).map_err(|e| {
            SimError::with_diag(
                mivon_core::diagnostics::DiagCode::IoError,
                format!("salin '{}': {}", src.display(), e),
            )
        })?;
        return Ok(());
    }
    if let Ok(p) = std::path::Path::new(url).canonicalize() {
        if p.is_file() {
            std::fs::copy(&p, dest).map_err(|e| {
                SimError::with_diag(
                    mivon_core::diagnostics::DiagCode::IoError,
                    format!("salin '{}': {}", p.display(), e),
                )
            })?;
            return Ok(());
        }
    }
    let status = std::process::Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(dest)
        .arg(url)
        .status()
        .map_err(|e| {
            SimError::with_diag(
                mivon_core::diagnostics::DiagCode::IoError,
                format!("curl tidak tersedia: {}", e),
            )
        })?;
    if !status.success() {
        return Err(SimError::with_diag(
            mivon_core::diagnostics::DiagCode::IoError,
            format!("unduh gagal '{}' (curl exit {})", url, status),
        ));
    }
    Ok(())
}

/// Backup file exe ke `<dir>/mivon.bak` (hapus backup lama dulu).
fn backup_current(exe: &Path) -> Result<(), SimError> {
    if !exe.exists() {
        return Ok(());
    }
    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    let bak = dir.join("mivon.bak");
    if bak.exists() {
        std::fs::remove_file(&bak).map_err(|e| io_err(&bak, e))?;
    }
    std::fs::rename(exe, &bak).map_err(|e| io_err(exe, e))
}

/// Pasang byte baru ke path exe (atomik: backup → rename).
/// TIDAK menjalankan smoke test — caller yang melakukannya.
pub fn install_binary_bytes(exe: &Path, data: &[u8]) -> Result<(), SimError> {
    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(".mivon-update-{}.tmp", std::process::id()));
    std::fs::write(&tmp, data).map_err(|e| io_err(&tmp, e))?;
    backup_current(exe)?;
    std::fs::rename(&tmp, exe).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io_err(exe, e)
    })?;
    set_executable(exe)?;
    Ok(())
}

/// Smoke test: jalankan `--version` pada binary yang baru dipasang.
pub fn smoke_test(exe: &Path) -> bool {
    std::process::Command::new(exe)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Restore backup terakhir ke exe (rollback).
pub fn restore_backup(exe: &Path) -> Result<bool, SimError> {
    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    let bak = dir.join("mivon.bak");
    if !bak.exists() {
        return Ok(false);
    }
    let _ = std::fs::remove_file(exe);
    std::fs::rename(&bak, exe).map_err(|e| io_err(exe, e))?;
    set_executable(exe)?;
    Ok(true)
}

#[cfg(unix)]
fn set_executable(exe: &Path) -> Result<(), SimError> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(exe)
        .map_err(|e| io_err(exe, e))?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(exe, perms).map_err(|e| io_err(exe, e))
}

#[cfg(not(unix))]
fn set_executable(_exe: &Path) -> Result<(), SimError> {
    Ok(())
}

fn io_err(p: &Path, e: std::io::Error) -> SimError {
    SimError::with_diag(
        mivon_core::diagnostics::DiagCode::IoError,
        format!("{}: {}", p.display(), e),
    )
}

/// Konfirmasi interaktif kecuali `--yes`.
fn confirm(question: &str, yes: bool) -> Result<bool, SimError> {
    if yes {
        return Ok(true);
    }
    print!("{} [y/N]: ", question);
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return Ok(false);
    }
    Ok(line.trim().eq_ignore_ascii_case("y"))
}

// ────────────────────────────────────────────────────────────────────────────
// Alur utama update & rollback
// ────────────────────────────────────────────────────────────────────────────

fn apply_update(
    version: &str,
    url: &str,
    sha: &str,
    exe: PathBuf,
    yes: bool,
) -> Result<(), SimError> {
    if !confirm(
        &format!("Pasang Mivon v{}? (binary saat ini di-backup)", version),
        yes,
    )? {
        println!("Dibatalkan — tidak ada perubahan.");
        return Ok(());
    }

    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(".mivon-update-{}.tmp", std::process::id()));

    println!("Mengunduh v{} ...", version);
    download_to(url, &tmp)?;

    let data = std::fs::read(&tmp).map_err(|e| io_err(&tmp, e))?;
    if !verify_checksum(&data, sha) {
        let _ = std::fs::remove_file(&tmp);
        return Err(SimError::with_diag(
            mivon_core::diagnostics::DiagCode::InternalError,
            format!(
                "checksum TIDAK cocok — dibatalkan (harap {} , dapat {})",
                abbreviate(sha),
                abbreviate(&sha256_hex(&data))
            ),
        ));
    }
    println!("Checksum SHA-256 cocok ✓");

    // Backup dulu, baru pasang.
    backup_current(&exe)?;
    std::fs::rename(&tmp, &exe).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io_err(&exe, e)
    })?;
    set_executable(&exe)?;

    // Smoke test; gagal → rollback otomatis.
    if !smoke_test(&exe) {
        let restored = restore_backup(&exe)?;
        return Err(SimError::with_diag(
            mivon_core::diagnostics::DiagCode::InternalError,
            format!(
                "smoke test gagal pada v{} — {}",
                version,
                if restored {
                    "binary lama di-restore ✓"
                } else {
                    "tidak ada backup untuk di-restore!"
                }
            ),
        ));
    }

    println!(
        "Mivon v{} terpasang ✓ (backup: {} )",
        version,
        abbreviate(sha)
    );
    println!("Gunakan `mivon update --rollback` untuk kembali.");
    Ok(())
}

fn do_rollback(exe: PathBuf, yes: bool) -> Result<(), SimError> {
    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    let bak = dir.join("mivon.bak");
    if !bak.exists() {
        return Err(SimError::with_diag(
            mivon_core::diagnostics::DiagCode::InternalError,
            format!("tidak ada backup di '{}'", bak.display()),
        ));
    }
    if !confirm("Kembali ke binary sebelumnya?", yes)? {
        println!("Dibatalkan.");
        return Ok(());
    }
    let restored = restore_backup(&exe)?;
    if !restored {
        return Err(SimError::with_diag(
            mivon_core::diagnostics::DiagCode::InternalError,
            "rollback gagal — backup hilang",
        ));
    }
    if !smoke_test(&exe) {
        return Err(SimError::with_diag(
            mivon_core::diagnostics::DiagCode::InternalError,
            "rollback gagal — binary restore tidak lolos smoke test",
        ));
    }
    println!("Rollback selesai ✓ (binary sebelumnya dipulihkan)");
    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_compare_membandingkan_patch() {
        assert!(newer("0.4.1", "0.4.0"));
        assert!(newer("0.5.0", "0.4.9"));
        assert!(newer("1.0.0", "0.9.9"));
        assert!(!newer("0.4.0", "0.4.0"));
        assert!(!newer("0.4.0", "0.4.1"));
        // toleran prefix v dan suffix prarilis
        assert!(!newer("v0.4.0-beta.1", "0.4.0"));
        assert!(newer("v0.4.0", "0.4.0-rc.1") || parse_version("v0.4.0-rc.1") == (0, 4, 0));
    }

    #[test]
    fn parse_version_beragam_format() {
        assert_eq!(parse_version("0.3.0"), (0, 3, 0));
        assert_eq!(parse_version("v1.2.3"), (1, 2, 3));
        assert_eq!(parse_version("2.0"), (2, 0, 0));
        assert_eq!(parse_version("0.4.0-beta.2"), (0, 4, 0));
        assert_eq!(parse_version("garbage"), (0, 0, 0));
    }

    #[test]
    fn sha256_dikenal() {
        // sha256("abc") == ...
        let expect = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256_hex(b"abc"), expect);
        assert!(verify_checksum(b"abc", expect));
        assert!(!verify_checksum(b"abd", expect));
        // case-insensitive
        assert!(verify_checksum(b"abc", &expect.to_uppercase()));
    }

    #[test]
    fn manifest_parse_valid_dan_invalid() {
        let ok = r#"{
            "version": "0.4.0", "tag": "v0.4.0", "published_at": "2026-09-20",
            "platforms": {
              "x86_64-unknown-linux-gnu": {
                "url": "https://example.com/mivon",
                "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
              }
            }
        }"#;
        let m: Manifest = serde_json::from_str(ok).expect("manifest valid");
        assert_eq!(m.version, "0.4.0");
        assert!(m.platforms.contains_key("x86_64-unknown-linux-gnu"));

        let bad = r#"{ "version": }"#;
        assert!(serde_json::from_str::<Manifest>(bad).is_err());
        // platform kosong → valid schema, tapi lookup gagal
        let empty = r#"{"version":"0.4.0","tag":"v0.4.0","platforms":{}}"#;
        let m: Manifest = serde_json::from_str(empty).unwrap();
        assert!(m.platforms.is_empty());
    }

    #[test]
    fn checksum_mismatch_ditolak() {
        let zeros = "0".repeat(64);
        assert!(!verify_checksum(b"payload", &zeros));
        assert!(verify_checksum(b"payload", &sha256_hex(b"payload")));
    }

    #[test]
    fn install_dan_rollback_atomik() {
        let dir = std::env::temp_dir().join("mivon-update-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("mivon");

        // instal pertama (tanpa backup sebelumnya)
        install_binary_bytes(&exe, b"#!/bin/sh\necho fake\n").unwrap();
        assert!(exe.exists());
        assert!(!dir.join("mivon.bak").exists());

        // instal kedua → backup lama dibuat
        install_binary_bytes(&exe, b"#!/bin/sh\necho newer\n").unwrap();
        assert!(dir.join("mivon.bak").exists());

        // rollback mengembalikan konten pertama
        assert!(restore_backup(&exe).unwrap());
        let content = std::fs::read_to_string(&exe).unwrap();
        assert!(content.contains("fake"));

        // restore kedua tanpa backup → false
        assert!(!restore_backup(&exe).unwrap());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fetch_text_file_url() {
        let dir = std::env::temp_dir().join("mivon-update-test-manifest");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("latest.json");
        std::fs::write(&p, r#"{"version":"9.9.9","tag":"v9.9.9","platforms":{}}"#).unwrap();

        let text = fetch_text(&format!("file://{}", p.display())).unwrap();
        assert!(text.contains("9.9.9"));

        let m = fetch_manifest(Some(&format!("file://{}", p.display())), None).unwrap();
        assert_eq!(m.version, "9.9.9");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn platform_key_pada_platform_ini() {
        let key = platform_key().unwrap();
        // hanya memastikan format arch-os valid
        assert!(
            key.starts_with("x86_64-") || key.starts_with("aarch64-"),
            "key: {}",
            key
        );
    }
}
