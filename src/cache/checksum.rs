//! Fast content hashing using xxhash3.
//!
//! xxhash3 dipilih karena: non-cryptographic, sangat cepat (~50GB/s),
//! cukup untuk content-based cache invalidation.

use xxhash_rust::xxh3::xxh3_64;

/// Compute xxhash3-64 checksum of byte slice.
pub fn compute_checksum(data: &[u8]) -> u64 {
    xxh3_64(data)
}

/// Compute checksum of a file's content.
pub fn compute_file_checksum(path: &std::path::Path) -> std::io::Result<u64> {
    let data = std::fs::read(path)?;
    Ok(compute_checksum(&data))
}

/// Compute checksum of a string.
pub fn compute_str_checksum(s: &str) -> u64 {
    xxh3_64(s.as_bytes())
}

/// Combine two checksums (for dependency hashing).
pub fn combine_checksum(a: u64, b: u64) -> u64 {
    // Simple combination: xorshift
    let mixed = a.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(b);
    mixed ^ (mixed >> 31)
}

/// Compute checksum of multiple values (fold).
pub fn checksum_fold(checksums: &[u64]) -> u64 {
    checksums
        .iter()
        .copied()
        .fold(0u64, |acc, h| combine_checksum(acc, h))
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_checksum_basic() {
        let h1 = compute_checksum(b"hello");
        let h2 = compute_checksum(b"hello");
        let h3 = compute_checksum(b"world");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_compute_str_checksum() {
        let h = compute_str_checksum("module test");
        assert_ne!(h, 0);
    }

    #[test]
    fn test_combine_checksum() {
        let c1 = combine_checksum(100, 200);
        let c2 = combine_checksum(100, 200);
        let c3 = combine_checksum(200, 100);
        assert_eq!(c1, c2);
        assert_ne!(c1, c3); // commutativity broken (expected)
    }

    #[test]
    fn test_checksum_fold() {
        let empty = checksum_fold(&[]);
        let single = checksum_fold(&[42]);
        assert_eq!(empty, 0);
        assert_ne!(single, 0);
    }

    #[test]
    fn test_file_checksum() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        if path.exists() {
            let h = compute_file_checksum(&path).unwrap();
            assert_ne!(h, 0);
        }
    }
}
