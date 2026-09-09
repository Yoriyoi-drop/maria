//! Project-wide seed sweep — korpus nyata sebagai TARGET pencarian error,
//! bukan sekadar sumber fragment mutasi.
//!
//! Paper #12 (Code Fragments) & #18 (FSM-aware): seed SV nyata (opentitan,
//! cva6, test/, …) membawa error yang HANYA muncul ketika SELURUH proyek
//! di-compile sebagai satu design (dependensi lintas file, macro, package,
//! parameter) — compile per-file standalone melewatkannya.
//!
//! Tanggung jawab modul ini (1 file = 1 tanggung jawab): enumerate file SV
//! proyek → compile sebagai SATU design via `maria_api::compile_collect_errors`
//! (SEMUA error, bukan pertama) → klasifikasi per kategori → laporan sweep.

use std::path::{Path, PathBuf};

use maria_api::ProjectError;

/// Kategori error proyek — sejajar laporan "Kesiapan Simulasi" main.rs
/// (Parse/Semantik/Hierarki/Resolusi Top/Penghubung DPI) + Runtime/Elab/Lain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCat {
    Parse,
    Semantik,
    Hierarki,
    Top,
    Dpi,
    Runtime,
    Elab,
    Lain,
}

impl ErrorCat {
    pub fn label(&self) -> &'static str {
        match self {
            ErrorCat::Parse => "Parse",
            ErrorCat::Semantik => "Semantik",
            ErrorCat::Hierarki => "Hierarki",
            ErrorCat::Top => "Resolusi Top",
            ErrorCat::Dpi => "Penghubung DPI",
            ErrorCat::Runtime => "Runtime",
            ErrorCat::Elab => "Elaborasi",
            ErrorCat::Lain => "Lain",
        }
    }

    /// Klasifikasi kode diagnostic maria (E####/EL####/RT####).
    pub fn classify(code: &str) -> ErrorCat {
        match code {
            // Semantik (readiness): undefined signal / type / width / variable.
            "E2001" | "E2002" | "E2003" | "E2004" => ErrorCat::Semantik,
            // Hierarki (readiness): module / instance / circular / unresolved.
            "E3001" | "E3002" | "E3004" | "E3008" | "E3009" => ErrorCat::Hierarki,
            // Resolusi Top (readiness).
            "EL3001" | "E3006" | "E3007" => ErrorCat::Top,
            // DPI (readiness).
            "RT8001" | "RT8002" | "RT8003" => ErrorCat::Dpi,
            _ if code.starts_with("E1") => ErrorCat::Parse,
            _ if code.starts_with("RT") => ErrorCat::Runtime,
            _ if code.starts_with("EL") => ErrorCat::Elab,
            _ => ErrorCat::Lain,
        }
    }
}

/// Statistik satu kategori error.
#[derive(Debug, Clone, Default)]
pub struct CatStat {
    pub count: u64,
    /// Kode error unik dalam kategori (dedup, urut kemunculan).
    pub codes: Vec<String>,
    /// Contoh error (file:line:col) — dedup per lokasi, cap 10.
    pub samples: Vec<ProjectError>,
}

/// Hasil sweep project-wide.
#[derive(Debug, Clone, Default)]
pub struct ProjectSweep {
    pub files_total: u64,
    pub errors_total: u64,
    /// Kode error unik SELURUH proyek (dedup) — peta fitur LRM belum didukung.
    pub all_codes: Vec<String>,
    pub cats: Vec<(ErrorCat, CatStat)>,
}

impl ProjectSweep {
    pub fn cat(&self, c: ErrorCat) -> Option<&CatStat> {
        self.cats.iter().find(|(k, _)| *k == c).map(|(_, s)| s)
    }
}

/// Expand direktori → daftar file `.sv`/`.v` (rekursif). Include-hidden off.
/// Path relatif dipertahankan (sesuai input).
pub fn enumerate_sv(dirs: &[PathBuf]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for dir in dirs {
        collect_files(dir, &mut out);
    }
    out.sort();
    out
}

fn collect_files(dir: &Path, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_files(&p, out);
        } else if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
            if ext == "sv" || ext == "v" {
                out.push(p.to_string_lossy().to_string());
            }
        }
    }
}

/// Sample deterministik dari daftar file (stride seragam, urutan terurut) —
/// mencegah sweep penuh 4k+ file membakar budget kampanye. `cap`=maks file.
pub fn sample_stride(files: &[String], cap: usize) -> Vec<String> {
    if files.len() <= cap {
        return files.to_vec();
    }
    let step = files.len() / cap;
    let mut out = Vec::with_capacity(cap);
    let mut idx = 0usize;
    for _ in 0..cap {
        if idx < files.len() {
            out.push(files[idx].clone());
        }
        idx += step;
    }
    out
}

/// Jalankan sweep: compile seluruh daftar file sebagai satu design, kumpulkan
/// SEMUA error (parse + elab), klasifikasikan per kategori.
pub fn sweep_files(files: &[String]) -> ProjectSweep {
    let errs = maria_api::compile_collect_errors(&files.iter().cloned().collect::<Vec<_>>());
    let mut sweep = ProjectSweep {
        files_total: files.len() as u64,
        errors_total: errs.len() as u64,
        ..ProjectSweep::default()
    };
    // Seed kategori dengan urutan tetap (stabil utk laporan).
    let order = [
        ErrorCat::Parse,
        ErrorCat::Semantik,
        ErrorCat::Hierarki,
        ErrorCat::Top,
        ErrorCat::Dpi,
        ErrorCat::Runtime,
        ErrorCat::Elab,
        ErrorCat::Lain,
    ];
    sweep.cats = order.iter().map(|c| (*c, CatStat::default())).collect();
    for e in &errs {
        let cat = ErrorCat::classify(&e.code);
        let stat = sweep
            .cats
            .iter_mut()
            .find(|(k, _)| *k == cat)
            .map(|(_, s)| s)
            .expect("kategori dari order");
        stat.count += 1;
        if !stat.codes.contains(&e.code) {
            stat.codes.push(e.code.clone());
        }
        if stat.samples.len() < 10 {
            let loc = format!("{}:{}:{}", e.file, e.line, e.col);
            if !stat.samples.iter().any(|s| format!("{}:{}:{}", s.file, s.line, s.col) == loc) {
                stat.samples.push(e.clone());
            }
        }
        if !sweep.all_codes.contains(&e.code) {
            sweep.all_codes.push(e.code.clone());
        }
    }
    sweep
}

/// Sweep korpus direktori (sample deterministik `cap` file per pemanggilan) —
/// dipakai run_fuzz satu kali per kampanye. `full_cap`=0 → semua file.
pub fn sweep_corpus(dirs: &[PathBuf], cap: usize) -> ProjectSweep {
    let files = enumerate_sv(dirs);
    let picked = if cap == 0 {
        files
    } else {
        sample_stride(&files, cap)
    };
    sweep_files(&picked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_codes() {
        assert_eq!(ErrorCat::classify("E1005"), ErrorCat::Parse);
        assert_eq!(ErrorCat::classify("E2002"), ErrorCat::Semantik);
        assert_eq!(ErrorCat::classify("E3001"), ErrorCat::Hierarki);
        assert_eq!(ErrorCat::classify("EL3001"), ErrorCat::Top);
        assert_eq!(ErrorCat::classify("RT8002"), ErrorCat::Dpi);
        assert_eq!(ErrorCat::classify("RT0001"), ErrorCat::Runtime);
        assert_eq!(ErrorCat::classify("EL9999"), ErrorCat::Elab);
        assert_eq!(ErrorCat::classify("XX"), ErrorCat::Lain);
    }

    #[test]
    fn sample_stride_reduces() {
        let files: Vec<String> = (0..100).map(|i| format!("f{}.sv", i)).collect();
        let s = sample_stride(&files, 10);
        assert_eq!(s.len(), 10);
        assert!(s.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn sweep_empty_dirs_no_panic() {
        let s = sweep_corpus(&[], 100);
        assert_eq!(s.files_total, 0);
        assert_eq!(s.errors_total, 0);
    }
}