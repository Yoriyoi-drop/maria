//! Foreign Library Loader (arsitektur masukan user poin 2, 9, 10).
//!
//! Penemuan + pemuatan library native untuk VHPI/PLI. Maria harus bisa
//! menemukan:
//!
//! ```text
//! libfoo.so       (Linux)
//! foo.so
//! libfoo.dylib    (macOS)
//! foo.dll         (Windows)
//! ```
//!
//! lalu: discover → load → ABI validation → symbol resolution → registration.
//! Loader memakai `libloading` (feature "dpi" — dependency opsional yang sama
//! dengan DPI-C engine). Tanpa feature, loader menjadi stub (compile-time).

use std::path::{Path, PathBuf};

/// Kandidat nama file untuk satu library name, per-platform.
/// `foo` → `libfoo.so`, `foo.so`, `libfoo.dylib`, `foo.dll`.
pub fn candidate_paths(name: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return out;
    }
    // Bila sudah punya ekstensi, coba langsung dulu.
    out.push(PathBuf::from(trimmed));
    let stem = trimmed
        .strip_prefix("lib")
        .map(|s| s.trim_end_matches(".so").trim_end_matches(".dylib").trim_end_matches(".dll"))
        .unwrap_or(trimmed.trim_end_matches(".so").trim_end_matches(".dylib").trim_end_matches(".dll"));
    #[cfg(target_os = "linux")]
    {
        out.push(PathBuf::from(format!("lib{}.so", stem)));
        out.push(PathBuf::from(format!("{}.so", stem)));
    }
    #[cfg(target_os = "macos")]
    {
        out.push(PathBuf::from(format!("lib{}.dylib", stem)));
        out.push(PathBuf::from(format!("{}.dylib", stem)));
    }
    #[cfg(target_os = "windows")]
    {
        out.push(PathBuf::from(format!("lib{}.dll", stem)));
        out.push(PathBuf::from(format!("{}.dll", stem)));
    }
    out
}

/// Direktori pencarian default: cwd, LD_LIBRARY_PATH/DYLD_LIBRARY_PATH,
/// /usr/local/lib, /usr/lib, /lib.
pub fn default_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd);
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if let Ok(ld) = std::env::var("LD_LIBRARY_PATH") {
        for p in ld.split(':') {
            if !p.is_empty() {
                paths.push(PathBuf::from(p));
            }
        }
    }
    paths.push(PathBuf::from("/usr/local/lib"));
    paths.push(PathBuf::from("/usr/lib"));
    paths.push(PathBuf::from("/lib"));
    paths
}

/// Hasil ABI validation (arsitektur poin 10): arsitektur/pointer-width harus
/// cocok dengan proses Maria. Validasi nyata dilakukan saat `Library::new`
/// gagal memuat (kernel menolak binary arsitektur berbeda) — di sini kita
/// mencatat target proses untuk laporan yang jelas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiInfo {
    pub arch: &'static str,
    pub os: &'static str,
    pub pointer_width: usize,
}

pub fn current_abi() -> AbiInfo {
    AbiInfo {
        arch: std::env::consts::ARCH,
        os: std::env::consts::OS,
        pointer_width: std::mem::size_of::<usize>() * 8,
    }
}

/// Cari path library yang benar-benar ada di search paths.
/// Mengembalikan path pertama yang `exists()`.
pub fn find_library(name: &str, search_paths: &[PathBuf]) -> Option<PathBuf> {
    for cand in candidate_paths(name) {
        if cand.exists() {
            return Some(cand);
        }
        for sp in search_paths {
            let fp = sp.join(&cand);
            if fp.exists() {
                return Some(fp);
            }
        }
    }
    None
}

// ─── Pemuatan (feature "dpi" → libloading) ───

/// Muat library native. `None` bila feature "dpi" off (stub) atau load gagal.
#[cfg(feature = "dpi")]
pub fn load_library(path: &Path) -> Result<std::sync::Arc<libloading::Library>, String> {
    match unsafe { libloading::Library::new(path) } {
        Ok(lib) => Ok(std::sync::Arc::new(lib)),
        Err(e) => Err(format!("cannot load '{}': {}", path.display(), e)),
    }
}

/// Resolve symbol C dari library. `None` bila symbol tidak ada.
/// T harus `Copy` (fn pointer / data pointer) — pola sama dgn DPI engine.
#[cfg(feature = "dpi")]
pub fn resolve_symbol<T: Copy>(lib: &libloading::Library, name: &str) -> Option<T> {
    unsafe { lib.get::<T>(name.as_bytes()) }.ok().map(|s| *s)
}

/// Stub tanpa feature "dpi" — kompilasi tetap jalan, runtime mengembalikan
/// error jelas (bukan panic).
#[cfg(not(feature = "dpi"))]
pub fn load_library(_path: &Path) -> Result<std::sync::Arc<()>, String> {
    Err("foreign library loading requires feature \"dpi\" (libloading)".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_candidate_paths_linux() {
        let paths = candidate_paths("foo");
        assert!(!paths.is_empty(), "harus ada kandidat");
        let names: Vec<String> = paths.iter().map(|p| p.to_string_lossy().to_string()).collect();
        // Nama polos selalu kandidat pertama.
        assert_eq!(names[0], "foo");
        #[cfg(target_os = "linux")]
        {
            assert!(names.contains(&"libfoo.so".to_string()), "libfoo.so harus ada: {:?}", names);
            assert!(names.contains(&"foo.so".to_string()), "foo.so harus ada: {:?}", names);
        }
    }

    #[test]
    fn test_candidate_paths_with_extension() {
        let paths = candidate_paths("libfoo.so");
        let names: Vec<String> = paths.iter().map(|p| p.to_string_lossy().to_string()).collect();
        assert_eq!(names[0], "libfoo.so", "path polos pertama");
        // stem dihitung dari libfoo.so → foo → libfoo.so tetap kandidat.
        assert!(names.contains(&"libfoo.so".to_string()));
    }

    #[test]
    fn test_find_library_missing_returns_none() {
        let paths = default_search_paths();
        let found = find_library("definitely_not_existing_lib_xyz_12345", &paths);
        assert!(found.is_none(), "library imajiner tidak boleh ditemukan");
    }

    #[test]
    fn test_current_abi_reports_target() {
        let abi = current_abi();
        assert_eq!(abi.pointer_width, std::mem::size_of::<usize>() * 8);
        assert!(!abi.arch.is_empty());
        assert!(!abi.os.is_empty());
    }
}
