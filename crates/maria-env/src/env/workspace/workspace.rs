use crate::env::config::ConfigContext;
use crate::env::workspace::{IncludeDirs, ProjectFile};
use maria_compiler::frontend::discovery::{DiscoveryOptions, FileDiscovery};
use std::path::{Path, PathBuf};

/// Direktori source yang dikenali otomatis bila ada di root workspace.
const DEFAULT_SOURCE_DIRS: [&str; 5] = ["rtl", "tb", "ip", "src", "third_party"];

/// WorkspaceContext — mengelola layout project: root path, direktori source,
/// filelist, incdir, library, output path.
///
/// Compiler tidak perlu tahu di mana file berada; ia cukup bertanya
/// `workspace.discover_sources()`.
#[derive(Debug, Clone)]
pub struct WorkspaceContext {
    pub root: PathBuf,
    pub output_dir: PathBuf,
    source_dirs: Vec<PathBuf>,
    filelists: Vec<PathBuf>,
    libdirs: Vec<PathBuf>,
    libfiles: Vec<PathBuf>,
    include: IncludeDirs,
    defines: Vec<(String, String)>,
    /// Sumber eksplisit (dari CLI/filelist) — bila set, `discover_sources`
    /// memakainya TANPA scan direktori (menghindari scan penuh yang lambat).
    explicit_sources: Option<Vec<PathBuf>>,
}

impl WorkspaceContext {
    /// Buka workspace di direktori saat ini (CWD).
    pub fn open(_config: &ConfigContext) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::open_in(&cwd)
    }

    /// Buka workspace di root eksplisit. Source dirs `rtl/`, `tb/`, `ip/`,
    /// `src/`, `third_party/` dikenali otomatis bila ada.
    pub fn open_in(root: &Path) -> Self {
        let mut source_dirs = Vec::new();
        for name in DEFAULT_SOURCE_DIRS {
            let d = root.join(name);
            if d.is_dir() {
                source_dirs.push(d);
            }
        }
        WorkspaceContext {
            root: root.to_path_buf(),
            output_dir: root.join("out"),
            source_dirs,
            filelists: Vec::new(),
            libdirs: Vec::new(),
            libfiles: Vec::new(),
            include: IncludeDirs::new(),
            defines: Vec::new(),
            explicit_sources: None,
        }
    }

    // ── Akses ──

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn output_dir(&self) -> &Path {
        &self.output_dir
    }

    pub fn source_dirs(&self) -> &[PathBuf] {
        &self.source_dirs
    }

    pub fn incdirs(&self) -> &IncludeDirs {
        &self.include
    }

    pub fn defines(&self) -> &[(String, String)] {
        &self.defines
    }

    pub fn libdirs(&self) -> &[PathBuf] {
        &self.libdirs
    }

    pub fn libfiles(&self) -> &[PathBuf] {
        &self.libfiles
    }

    /// Setel daftar sumber eksplisit (dari CLI/filelist yang sudah di-expand).
    /// Menghindari scan direktori di `discover_sources`.
    pub fn set_explicit_sources(&mut self, sources: Vec<PathBuf>) {
        self.explicit_sources = Some(sources);
    }

    // ── Mutasi ──

    pub fn set_output_dir(&mut self, dir: impl Into<PathBuf>) {
        self.output_dir = dir.into();
    }

    pub fn add_source_dir(&mut self, dir: impl Into<PathBuf>) {
        let d = dir.into();
        if !self.source_dirs.contains(&d) {
            self.source_dirs.push(d);
        }
    }

    pub fn add_incdir(&mut self, dir: impl Into<PathBuf>) {
        self.include.add(dir);
    }

    pub fn add_define(&mut self, name: &str, value: &str) {
        self.defines.push((name.to_string(), value.to_string()));
    }

    pub fn add_filelist(&mut self, path: impl Into<PathBuf>) {
        let p = path.into();
        if !self.filelists.contains(&p) {
            self.filelists.push(p);
        }
    }

    pub fn add_libdir(&mut self, dir: impl Into<PathBuf>) {
        let d = dir.into();
        if !self.libdirs.contains(&d) {
            self.libdirs.push(d);
        }
    }

    pub fn add_libfile(&mut self, f: impl Into<PathBuf>) {
        let f = f.into();
        if !self.libfiles.contains(&f) {
            self.libfiles.push(f);
        }
    }

    // ── Operasi ──

    /// Muat file proyek `.maria`/`.f` dan daftarkan sebagai filelist +
    /// incdir-nya. Error bila file list tidak punya file.
    pub fn load_project(&mut self, path: &Path) -> Result<(), String> {
        let proj = ProjectFile::load(path)?;
        self.add_filelist(path.to_path_buf());
        for d in proj.incdirs {
            self.add_incdir(d);
        }
        Ok(())
    }

    /// Kumpulkan seluruh file source. Bila `set_explicit_sources` dipanggil,
    /// daftar itu yang dipakai (tanpa scan). Selain itu: filelist + scan
    /// paralel direktori source. Di-unique dan diurutkan agar deterministik.
    pub fn discover_sources(&self) -> Vec<PathBuf> {
        if let Some(explicit) = &self.explicit_sources {
            let mut files = explicit.clone();
            files.sort();
            files.dedup();
            return files;
        }
        let mut files: Vec<PathBuf> = Vec::new();
        for fl in &self.filelists {
            if let Ok(p) = ProjectFile::load(fl) {
                files.extend(p.files);
            }
        }
        if !self.source_dirs.is_empty() {
            let result = FileDiscovery::scan_dirs(&self.source_dirs, &DiscoveryOptions::default());
            files.extend(result.files.into_iter().map(|f| f.path));
        }
        files.sort();
        files.dedup();
        files
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_in_creates_structure() {
        let dir = std::env::temp_dir().join("maria_ws_test");
        let _ = std::fs::create_dir_all(dir.join("rtl"));
        let _ = std::fs::create_dir_all(dir.join("tb"));
        std::fs::write(dir.join("rtl/counter.sv"), "module counter; endmodule\n").unwrap();

        let ws = WorkspaceContext::open_in(&dir);
        assert_eq!(ws.root(), dir.as_path());
        assert_eq!(ws.output_dir(), dir.join("out"));
        assert_eq!(ws.source_dirs().len(), 2, "rtl + tb dikenali otomatis");
        assert!(ws.source_dirs().contains(&dir.join("rtl")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_discover_sources_dedup_sorted() {
        let dir = std::env::temp_dir().join("maria_ws_discover");
        let _ = std::fs::create_dir_all(dir.join("rtl"));
        std::fs::write(dir.join("rtl/b.sv"), "module b; endmodule\n").unwrap();
        std::fs::write(dir.join("rtl/a.sv"), "module a; endmodule\n").unwrap();
        std::fs::write(dir.join("list.f"), "rtl/b.sv\n").unwrap();

        let mut ws = WorkspaceContext::open_in(&dir);
        ws.add_filelist(dir.join("list.f"));
        let files = ws.discover_sources();
        // b.sv ada di dua sumber (filelist + scan) → dedup; urut: a < b.
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].file_name().unwrap(), "a.sv");
        assert_eq!(files[1].file_name().unwrap(), "b.sv");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_project_registers_incdir() {
        let dir = std::env::temp_dir().join("maria_ws_proj");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("p.maria"), "+incdir+inc\nrtl/a.sv\n").unwrap();
        let mut ws = WorkspaceContext::open_in(&dir);
        ws.load_project(&dir.join("p.maria")).unwrap();
        assert_eq!(ws.incdirs().dirs(), &[dir.join("inc")]);
        assert_eq!(ws.discover_sources(), vec![dir.join("rtl/a.sv")]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
