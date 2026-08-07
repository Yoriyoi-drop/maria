use std::path::{Path, PathBuf};

/// File proyek `.maria` — daftar file `.sv` (satu per baris, `#` komentar),
/// plus `+incdir+` / `-f` bila ada. Path relatif terhadap direktori file.
#[derive(Debug, Clone)]
pub struct ProjectFile {
    pub path: PathBuf,
    pub files: Vec<PathBuf>,
    pub incdirs: Vec<PathBuf>,
}

impl ProjectFile {
    pub fn load(path: &Path) -> Result<Self, String> {
        let list = crate::env::workspace::Filelist::parse(path)?;
        if list.files.is_empty() {
            return Err(format!("no .sv files listed in '{}'", path.display()));
        }
        Ok(ProjectFile {
            path: path.to_path_buf(),
            files: list.files,
            incdirs: list.incdirs,
        })
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn files(&self) -> &[PathBuf] {
        &self.files
    }

    pub fn incdirs(&self) -> &[PathBuf] {
        &self.incdirs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_file_load() {
        let dir = std::env::temp_dir().join("maria_project_test");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("proj.maria");
        std::fs::write(&p, "# project\nrtl/a.sv\n+incdir+inc\ntb/b.sv\n").unwrap();
        let proj = ProjectFile::load(&p).unwrap();
        assert_eq!(proj.files.len(), 2);
        assert_eq!(proj.files[0], dir.join("rtl/a.sv"));
        assert_eq!(proj.incdirs, vec![dir.join("inc")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_project_file_empty_is_err() {
        let dir = std::env::temp_dir().join("maria_project_empty");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("empty.maria");
        std::fs::write(&p, "# tidak ada file\n").unwrap();
        assert!(ProjectFile::load(&p).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
