use std::path::PathBuf;

/// Pengelola direktori include (`+incdir+`): urutan dijaga, tanpa duplikasi.
#[derive(Debug, Clone, Default)]
pub struct IncludeDirs {
    dirs: Vec<PathBuf>,
}

impl IncludeDirs {
    pub fn new() -> Self {
        IncludeDirs { dirs: Vec::new() }
    }

    pub fn add(&mut self, dir: impl Into<PathBuf>) {
        let d = dir.into();
        if !self.dirs.contains(&d) {
            self.dirs.push(d);
        }
    }

    pub fn extend<I: IntoIterator<Item = PathBuf>>(&mut self, iter: I) {
        for d in iter {
            self.add(d);
        }
    }

    pub fn dedup(&mut self) {
        let mut seen: Vec<PathBuf> = Vec::new();
        self.dirs.retain(|d| {
            if seen.contains(d) {
                false
            } else {
                seen.push(d.clone());
                true
            }
        });
    }

    pub fn dirs(&self) -> &[PathBuf] {
        &self.dirs
    }

    pub fn len(&self) -> usize {
        self.dirs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dirs.is_empty()
    }

    /// Representasi string untuk `Preprocessor::add_search_path`.
    pub fn to_search_paths(&self) -> Vec<String> {
        self.dirs
            .iter()
            .filter_map(|d| d.to_str().map(String::from))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_dedup_keeps_order() {
        let mut inc = IncludeDirs::new();
        inc.add("rtl");
        inc.add("tb");
        inc.add("rtl");
        assert_eq!(inc.len(), 2);
        assert_eq!(inc.dirs()[0], PathBuf::from("rtl"));
        assert_eq!(inc.dirs()[1], PathBuf::from("tb"));
    }

    #[test]
    fn test_extend_and_dedup() {
        let mut inc = IncludeDirs::new();
        inc.extend(vec![PathBuf::from("a"), PathBuf::from("b"), PathBuf::from("a")]);
        assert_eq!(inc.len(), 2);
        inc.dedup();
        assert_eq!(inc.len(), 2);
    }

    #[test]
    fn test_to_search_paths() {
        let mut inc = IncludeDirs::new();
        inc.add("rtl");
        inc.add("tb");
        assert_eq!(inc.to_search_paths(), vec!["rtl".to_string(), "tb".to_string()]);
    }
}
