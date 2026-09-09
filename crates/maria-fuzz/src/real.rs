//! Real-project error hunting — atur file nyata menjadi reproducer minimal.
//!
//! Problem (audit GAP-11): fuzzer permukaan memutasi snippet kecil; proyek
//! nyata (opentitan 3920 file) punya 1679 error parse yang fuzzer tidak pernah
//! lihat. CLI `--filelist ... --recompile` melaporkannya tapi tidak pernah
//! mereduksi — 1679 error = mungkin HANYA 1 fitur LRM yang tidak didukung
//! ditambah recovery cascade yang memproduksi ratusan error lanjutan.
//!
//! Modul ini = 1 tanggung jawab: per-file compile → klasifikasi tiap error →
//! minimasi ke reproducer minimal → laporan kandidat BUG (bukan feature-gap).
//!
//! Klasifikasi (penting — feature gap BUKAN bug engine):
//! - `E9xxx` = InternalError → REAL BUG engine (harus 0).
//! - Panic/Hang → REAL BUG.
//! - `E1xxx` parse: bisa feature-gap (konstruk LRM belum didukung) ATAU
//!   recovery bug (parser jatuh ke EOF => cascading ribuan error). Yang
//!   selisih banyak (cascade) dicurigai recovery defect.
//! - Cascade detection: error di baris AKHIR file ("unexpected EOF") yang
//!   banyak = parser kehilangan recovery, bukan error fitur sejati.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// HASIL per-file.
#[derive(Debug, Clone)]
pub struct FileProbe {
    /// Path asli file (untuk konteks).
    pub path: PathBuf,
    /// Error code pertama (mis. "E1002", "E9001") — kosong bila ok.
    pub code: String,
    /// Pesan error pertama.
    pub message: String,
    /// Jumlah error jika di-compile dgn koleksi penuh (0 bila ok).
    pub total_errors: u64,
    /// Reproducer terminimalkan (bila error) — ukuran kecil yang memicu
    /// error CODE YANG SAMA. Kosong bila tidak bisa terminimalkan.
    pub minimized: String,
    /// Klasifikasi.
    pub kind: ProbeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeKind {
    Ok,
    /// Feature gap LRM — parser tolak konstruk valid (scorecard, bukan bug).
    FeatureGap,
    /// Error internal engine (E9xxx) — REAL BUG.
    InternalBug,
    /// Compile error tapi bukan internal — bisa feature gap atau recovery.
    CompileErr,
    /// Source berisi artefak fuzz (`fz_`) — di-skip.
    FuzzArtifact,
}

impl ProbeKind {
    pub fn label(&self) -> &'static str {
        match self {
            ProbeKind::Ok => "ok",
            ProbeKind::FeatureGap => "feature-gap",
            ProbeKind::InternalBug => "INTERNAL-BUG",
            ProbeKind::CompileErr => "compile-err",
            ProbeKind::FuzzArtifact => "fuzz-artifact",
        }
    }
}

/// Ringkasan seluruh probe.
#[derive(Debug, Clone, Default)]
pub struct RealHuntReport {
    pub files_scanned: u64,
    pub ok: u64,
    pub feature_gaps: u64,
    pub internal_bugs: u64,
    pub compile_errs: u64,
    /// Kode error unik (dedup) — peta fitur LRM belum didukung / bug.
    pub all_codes: Vec<String>,
    /// Kandidat bug (internal + cascade) dengan reproducer.
    pub bug_candidates: Vec<FileProbe>,
    /// Feature-gap samples (cap 5) utk laporan.
    pub gap_samples: Vec<String>,
}

/// Probe SATU file: compile → klasifikasi → minimasi.
///
/// `keep_errcode`: minimasi mempertahankan error CODE yang sama (bukan sekadar
/// "masih error" — error berbeda = konstruk berbeda terlibat).
pub fn probe_file(path: &Path) -> Option<FileProbe> {
    let content = std::fs::read_to_string(path).ok()?;
    // Artefak fuzz di-skip.
    if content.contains("fz_") && content.contains("_fuzz") {
        return Some(FileProbe {
            path: path.to_path_buf(),
            code: String::new(),
            message: String::new(),
            total_errors: 0,
            minimized: String::new(),
            kind: ProbeKind::FuzzArtifact,
        });
    }
    // Compile pertama: ambil error code.
    let (code, message) = match maria_api::compile_str_quiet(&content) {
        Ok(_) => (String::new(), String::new()),
        Err(e) => (e.error_code().to_string(), e.to_string()),
    };
    if code.is_empty() {
        return Some(FileProbe {
            path: path.to_path_buf(),
            code,
            message,
            total_errors: 0,
            minimized: String::new(),
            kind: ProbeKind::Ok,
        });
    }
    // Jumlah error penuh (SEMUA error, bukan pertama) — ukur cascade.
    let total_errors = maria_api::compile_diag_counts(&content).0 as u64;

    // Klasifikasi.
    let is_internal = code.starts_with("E9") || code.starts_with("EL9");
    let kind = if is_internal {
        ProbeKind::InternalBug
    } else if code.starts_with("E1") || code.starts_with("EL") {
        ProbeKind::CompileErr
    } else {
        ProbeKind::FeatureGap
    };

    // Minimasi: hapus baris selama error code SAMA (reproducer minimal).
    // Batch predicate ke maria-api via closure (bukan harness — compile cepat,
    // file real bisa besar; hang di-proteksi oleh parser internal).
    let src = &content;
    let minimized = crate::corpus::Corpus::minimize(src, &mut |cand| {
        if cand.trim().is_empty() {
            return false;
        }
        match maria_api::compile_str_quiet(cand) {
            Err(e) => e.error_code().to_string() == code,
            Ok(_) => false,
        }
    });

    Some(FileProbe {
        path: path.to_path_buf(),
        code,
        message,
        total_errors,
        minimized,
        kind,
    })
}

/// Probe seluruh daftar file (dari filelist atau dir scan).
pub fn hunt_files(files: &[PathBuf]) -> RealHuntReport {
    let mut report = RealHuntReport::default();
    for f in files {
        report.files_scanned += 1;
        let Some(probe) = probe_file(f) else {
            continue;
        };
        match probe.kind {
            ProbeKind::Ok => report.ok += 1,
            ProbeKind::FuzzArtifact => {}
            ProbeKind::FeatureGap => {
                report.feature_gaps += 1;
                if !report.all_codes.contains(&probe.code) {
                    report.all_codes.push(probe.code.clone());
                }
                if report.gap_samples.len() < 5 && probe.code.starts_with("E1") {
                    // Sample lokasi error pertama dari minimize awal.
                    report.gap_samples.push(format!(
                        "{} | {}",
                        probe.path.display(),
                        truncate(&probe.message, 72)
                    ));
                }
            }
            ProbeKind::InternalBug => {
                report.internal_bugs += 1;
                if !report.all_codes.contains(&probe.code) {
                    report.all_codes.push(probe.code.clone());
                }
                report.bug_candidates.push(probe);
            }
            ProbeKind::CompileErr => {
                report.compile_errs += 1;
                if !report.all_codes.contains(&probe.code) {
                    report.all_codes.push(probe.code.clone());
                }
                // Cascade detection: error di suspek EOF / total error >> 1.
                // Reproducer minim punya error code sama → kandidat recovery bug.
                let cascade_suspicious = probe.total_errors >= 5
                    || probe.message.contains("unexpected EOF")
                    || probe.message.contains("expected"); // recovery kehilangan konteks
                if cascade_suspicious && probe.minimized.len() < probe_min(&probe) {
                    report.bug_candidates.push(probe);
                } else if report.gap_samples.len() < 8 {
                    report.gap_samples.push(format!(
                        "{} | {}",
                        probe.path.display(),
                        truncate(&probe.message, 72)
                    ));
                }
            }
        }
    }
    report
}

fn probe_min(p: &FileProbe) -> usize {
    // Ukuran file asli (estimasi: minimize menghasilkan ≤ ini).
    p.minimized.len().saturating_mul(4).max(1)
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() > n {
        format!("{}…", &s[..n.saturating_sub(1)])
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_artifact_skipped() {
        let path = std::env::temp_dir().join("fz_artifact_check.sv");
        std::fs::write(&path, "module fz_abc; endmodule\n").unwrap();
        let p = probe_file(&path).unwrap();
        assert!(matches!(p.kind, ProbeKind::FuzzArtifact));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn probe_ok_file() {
        let path = std::env::temp_dir().join("probe_ok_check.sv");
        std::fs::write(&path, "module ok_t; initial $display(\"x\"); endmodule\n").unwrap();
        let p = probe_file(&path).unwrap();
        assert!(matches!(p.kind, ProbeKind::Ok), "expected Ok: {:?}", p);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn probe_compile_err_minimizes() {
        // Konstruk tanpa dukungan: `parameter type` dgn class? coba constructs
        // yang ELABORASI tolak: referensi sinyal undefined (E2xxx semantik).
        let path = std::env::temp_dir().join("probe_err_check.sv");
        std::fs::write(
            &path,
            "module err_t;\n  logic a;\n  assign a = undefined_sig;\nendmodule\n",
        )
        .unwrap();
        let p = probe_file(&path).unwrap();
        assert!(
            matches!(p.kind, ProbeKind::FeatureGap | ProbeKind::CompileErr),
            "expected err: {:?}",
            p
        );
        assert!(!p.code.is_empty(), "ada error code");
        // Minimized mempertahankan error yang sama.
        match maria_api::compile_str_quiet(&p.minimized) {
            Err(e) => assert_eq!(e.error_code().to_string(), p.code),
            Ok(_) => panic!("minimized tidak reproduce error"),
        }
        let _ = std::fs::remove_file(&path);
    }
}