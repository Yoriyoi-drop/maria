//! Parallel file discovery — scan ribuan file dalam hitungan detik.
//!
//! Menggunakan `walkdir` + `rayon` untuk parallel directory traversal.

use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Metadata file yang ditemukan.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    pub checksum: u64,
}

/// Hasil scan direktori.
#[derive(Debug, Default)]
pub struct DiscoveryResult {
    pub files: Vec<FileEntry>,
    pub sv_dirs: Vec<PathBuf>,
    pub scan_time_ms: u64,
}

/// Options untuk file discovery.
#[derive(Debug, Clone)]
pub struct DiscoveryOptions {
    pub extensions: Vec<String>,
    pub skip_dirs: Vec<String>,
    pub max_depth: usize,
}

impl Default for DiscoveryOptions {
    fn default() -> Self {
        DiscoveryOptions {
            extensions: vec!["sv".into(), "svh".into(), "v".into(), "vh".into()],
            skip_dirs: vec![".git".into(), "node_modules".into(), "build".into()],
            max_depth: 0,
        }
    }
}

/// Parallel file discovery engine.
pub struct FileDiscovery;

impl FileDiscovery {
    /// Scan direktori secara recursive untuk mencari file SV.
    pub fn scan_dir(root: impl AsRef<Path>, options: &DiscoveryOptions) -> DiscoveryResult {
        let start = Instant::now();
        let root = root.as_ref();

        if !root.exists() {
            return DiscoveryResult::default();
        }

        let skip_dirs: Vec<&str> = options.skip_dirs.iter().map(|s| s.as_str()).collect();
        let extensions: Vec<&str> = options.extensions.iter().map(|s| s.as_str()).collect();

        let mut file_paths: Vec<PathBuf> = Vec::new();
        let walk = walkdir::WalkDir::new(root)
            .follow_links(false)
            .same_file_system(true);

        let walk = if options.max_depth > 0 {
            walk.max_depth(options.max_depth)
        } else {
            walk
        };

        for entry in walk.into_iter().filter_entry(|e| {
            let name = e.file_name().to_str().unwrap_or("");
            !skip_dirs.contains(&name)
        }) {
            if let Ok(entry) = entry {
                if entry.file_type().is_file() {
                    if let Some(ext) = entry.path().extension() {
                        if let Some(ext_str) = ext.to_str() {
                            if extensions.contains(&ext_str) {
                                file_paths.push(entry.path().to_path_buf());
                            }
                        }
                    }
                }
            }
        }

        // Compute SV dirs (parent dirs of SV files)
        let mut sv_dirs: Vec<PathBuf> = file_paths
            .iter()
            .filter_map(|f| f.parent().map(|p| p.to_path_buf()))
            .collect();
        sv_dirs.sort();
        sv_dirs.dedup();

        // Compute checksums in parallel
        let file_entries: Vec<FileEntry> = file_paths
            .par_iter()
            .map(|path| {
                let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                let checksum = compute_file_checksum(path);
                FileEntry {
                    path: path.clone(),
                    size,
                    checksum,
                }
            })
            .collect();

        let elapsed = start.elapsed();

        DiscoveryResult {
            files: file_entries,
            sv_dirs,
            scan_time_ms: elapsed.as_millis() as u64,
        }
    }

    /// Scan beberapa direktori dan gabungkan hasilnya.
    pub fn scan_dirs(roots: &[impl AsRef<Path>], options: &DiscoveryOptions) -> DiscoveryResult {
        let start = Instant::now();
        let mut all_files = Vec::new();
        let mut all_dirs = Vec::new();

        for root in roots {
            let result = Self::scan_dir(root, options);
            all_files.extend(result.files);
            all_dirs.extend(result.sv_dirs);
        }

        all_files.sort_by(|a, b| a.path.cmp(&b.path));
        all_files.dedup_by(|a, b| a.path == b.path);
        all_dirs.sort();
        all_dirs.dedup();

        DiscoveryResult {
            files: all_files,
            sv_dirs: all_dirs,
            scan_time_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// Scan dari file list.
    pub fn scan_file_list(path: &Path) -> Result<Vec<PathBuf>, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read '{}': {}", path.display(), e))?;

        let base = path.parent().unwrap_or(Path::new("."));
        let files: Vec<PathBuf> = content
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| base.join(l))
            .collect();

        if files.is_empty() {
            return Err(format!("no files listed in '{}'", path.display()));
        }
        Ok(files)
    }
}

/// Compute xxhash3 checksum untuk file.
pub fn compute_file_checksum(path: &Path) -> u64 {
    use std::fs;
    use xxhash_rust::xxh3::xxh3_64;
    fs::read(path).map(|data| xxh3_64(&data)).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_test_dir() {
        let options = DiscoveryOptions::default();
        let result = FileDiscovery::scan_dir("test", &options);
        assert!(!result.files.is_empty(), "should find SV files in test/");
        for f in &result.files {
            let ext = f.path.extension().and_then(|e| e.to_str()).unwrap_or("");
            assert!(ext == "sv" || ext == "svh", "unexpected: {}", ext);
        }
    }

    #[test]
    fn test_scan_nonexistent() {
        let result = FileDiscovery::scan_dir("/nonexistent_path_xyz", &Default::default());
        assert!(result.files.is_empty());
        assert_eq!(result.scan_time_ms, 0);
    }

    #[test]
    fn test_scan_file_list() {
        let dir = std::env::temp_dir().join("maria_test_discovery");
        let _ = std::fs::create_dir_all(&dir);
        let list_path = dir.join("test_list.f");
        std::fs::write(&list_path, "counter.sv\ntb_counter.sv\n").unwrap();

        let result = FileDiscovery::scan_file_list(&list_path).unwrap();
        assert_eq!(result.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_checksum() {
        let dir = std::env::temp_dir().join("maria_test_checksum");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.sv");
        std::fs::write(&path, "module test; endmodule").unwrap();

        let cksum = compute_file_checksum(&path);
        assert_ne!(cksum, 0, "checksum should not be zero");

        let path2 = dir.join("test2.sv");
        std::fs::write(&path2, "module test; endmodule").unwrap();
        let cksum2 = compute_file_checksum(&path2);
        assert_eq!(cksum, cksum2, "same content → same checksum");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
