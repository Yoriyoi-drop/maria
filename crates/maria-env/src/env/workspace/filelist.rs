use std::path::{Path, PathBuf};

/// Hasil parse file list (`.f` / `.maria`): file sumber + direktori include.
#[derive(Debug, Clone, Default)]
pub struct Filelist {
    pub files: Vec<PathBuf>,
    pub incdirs: Vec<PathBuf>,
}

impl Filelist {
    /// Parse file list. Mendukung:
    /// - baris file biasa (relatif ke direktori file list)
    /// - komentar `#` dan `//`
    /// - `+incdir+DIR`
    /// - `-f FILE` (nested, dengan guard cycle)
    pub fn parse(path: &Path) -> Result<Self, String> {
        let mut out = Filelist::default();
        let mut visited = std::collections::HashSet::new();
        parse_into(path, &mut out, &mut visited)?;
        Ok(out)
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}

fn parse_into(
    path: &Path,
    out: &mut Filelist,
    visited: &mut std::collections::HashSet<PathBuf>,
) -> Result<(), String> {
    let base = path.parent().unwrap_or(Path::new("."));
    if !visited.insert(path.to_path_buf()) {
        return Err(format!("circular filelist: '{}'", path.display()));
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read '{}': {}", path.display(), e))?;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("+incdir+") {
            let dir = base.join(rest.trim());
            if !out.incdirs.contains(&dir) {
                out.incdirs.push(dir);
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("-f") {
            let nested = base.join(rest.trim());
            parse_into(&nested, out, visited)?;
            continue;
        }
        let p = base.join(trimmed);
        if maria_core::template::is_template_source(&p) {
            continue;
        }
        out.files.push(p);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_list() {
        let dir = std::env::temp_dir().join("maria_filelist_test");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("list.f");
        std::fs::write(&p, "# daftar file\ncounter.sv\n\ntb_counter.sv\n").unwrap();
        let list = Filelist::parse(&p).unwrap();
        assert_eq!(list.files.len(), 2);
        assert_eq!(list.files[0], dir.join("counter.sv"));
        assert!(list.incdirs.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_incdir_and_nested() {
        let dir = std::env::temp_dir().join("maria_filelist_nested");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("nested.f"), "deep.sv\n").unwrap();
        let p = dir.join("main.f");
        std::fs::write(
            &p,
            "+incdir+inc\na.sv\n-f nested.f\n// comment\n+incdir+inc\n",
        )
        .unwrap();
        let list = Filelist::parse(&p).unwrap();
        assert_eq!(list.files.len(), 2, "a.sv + deep.sv");
        assert_eq!(list.incdirs, vec![dir.join("inc")], "incdir dedup");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_circular_filelist_detected() {
        let dir = std::env::temp_dir().join("maria_filelist_circle");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("a.f"), "-f b.f\n").unwrap();
        std::fs::write(dir.join("b.f"), "-f a.f\n").unwrap();
        assert!(Filelist::parse(&dir.join("a.f")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
