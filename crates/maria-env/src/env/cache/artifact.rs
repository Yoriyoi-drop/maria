use std::path::{Path, PathBuf};

/// ArtifactPaths — resolver lokasi output artifact (VCD, FST, HTML, JSON).
#[derive(Debug, Clone)]
pub struct ArtifactPaths {
    pub output_dir: PathBuf,
}

impl ArtifactPaths {
    pub fn new(output_dir: impl Into<PathBuf>) -> Self {
        ArtifactPaths { output_dir: output_dir.into() }
    }

    /// Path artifact di dalam output_dir (mis. "top.vcd").
    pub fn path(&self, name: &str) -> PathBuf {
        self.output_dir.join(name)
    }

    /// Buat direktori output bila belum ada. Error bila gagal.
    pub fn ensure_output_dir(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.output_dir)
            .map_err(|e| format!("cannot create output dir '{}': {}", self.output_dir.display(), e))
    }

    /// Jalur relative terhadap output dir (untuk report).
    pub fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.output_dir)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| path.to_string_lossy().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_artifact_paths() {
        let a = ArtifactPaths::new("/tmp/maria-art-test");
        assert_eq!(a.path("top.vcd"), PathBuf::from("/tmp/maria-art-test/top.vcd"));
        a.ensure_output_dir().unwrap();
        assert!(a.output_dir.is_dir());
        let _ = std::fs::remove_dir_all(&a.output_dir);
    }
}
