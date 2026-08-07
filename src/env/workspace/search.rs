use crate::env::workspace::IncludeDirs;
use std::path::PathBuf;

/// Cari file `name` di daftar direktori. Urutan deklarasi dipakai — yang
/// pertama ketemu menang (semantik `+incdir+`).
pub fn find_in_dirs(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    for d in dirs {
        let p = d.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Resolve header `` `include "..." `` lewat IncludeDirs workspace.
pub fn resolve_header(name: &str, include: &IncludeDirs) -> Option<PathBuf> {
    find_in_dirs(name, include.dirs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_in_dirs() {
        let dir = std::env::temp_dir().join("maria_search_test");
        let _ = std::fs::create_dir_all(dir.join("inc"));
        std::fs::write(dir.join("inc/defs.svh"), "`define X\n").unwrap();

        let dirs = vec![dir.join("rtl"), dir.join("inc")];
        let found = find_in_dirs("defs.svh", &dirs).expect("harus ditemukan");
        assert_eq!(found, dir.join("inc/defs.svh"));

        assert!(find_in_dirs("nope.svh", &dirs).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_header() {
        let dir = std::env::temp_dir().join("maria_search_hdr");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("pkg.svh"), "package p; endpackage\n").unwrap();

        let mut inc = IncludeDirs::new();
        inc.add(dir.clone());
        assert_eq!(resolve_header("pkg.svh", &inc), Some(dir.join("pkg.svh")));
        assert!(resolve_header("absent.svh", &inc).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
