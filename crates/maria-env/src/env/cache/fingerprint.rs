use maria_compiler::cache::compute_checksum;
use std::path::Path;

/// Fingerprint — identitas konten (xxh3) + meta (ukuran) untuk cache key.
#[derive(Debug, Clone)]
pub struct Fingerprint {
    pub content_hash: u64,
    pub size: u64,
}

impl Fingerprint {
    /// Hitung fingerprint dari konten byte.
    pub fn of(content: &[u8]) -> Self {
        Fingerprint {
            content_hash: compute_checksum(content),
            size: content.len() as u64,
        }
    }

    /// Baca file + hitung fingerprint-nya (0 bila tidak terbaca).
    pub fn of_file(path: &Path) -> Self {
        let content = std::fs::read(path).unwrap_or_default();
        Self::of(&content)
    }

    /// Fingerprint dari string.
    pub fn of_str(s: &str) -> Self {
        Self::of(s.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fingerprint_deterministic() {
        let a = Fingerprint::of_str("module m; endmodule");
        let b = Fingerprint::of_str("module m; endmodule");
        assert_eq!(a.content_hash, b.content_hash);
        assert_eq!(a.size, b.size);
    }

    #[test]
    fn test_fingerprint_differs() {
        let a = Fingerprint::of_str("a");
        let b = Fingerprint::of_str("b");
        assert_ne!(a.content_hash, b.content_hash);
    }

    #[test]
    fn test_fingerprint_of_file() {
        let dir = std::env::temp_dir().join("maria_fp_test");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("f.sv");
        std::fs::write(&p, "module f; endmodule").unwrap();
        let fp = Fingerprint::of_file(&p);
        assert!(fp.content_hash != 0);
        assert!(fp.size > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
