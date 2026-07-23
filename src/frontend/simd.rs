//! SIMD-accelerated lexer operations.
//!
//! Menyediakan fungsi untuk mempercepat lexer menggunakan CPU SIMD
//! intrinsics (AVX2, SSE4.2) dengan fallback scalar.
//!
//! # Arsitektur
//!
//! - `skip_whitespace_simd()` — fungsi dispatch utama, otomatis pilih
//!   implementasi tercepat berdasarkan CPU yang terdeteksi.
//! - AVX2: proses 32 bytes sekaligus via `_mm256_loadu_si256`
//! - SSE4.2: proses 16 bytes sekaligus via `_mm_loadu_si128`
//! - Scalar: byte-by-byte fallback
//!
//! Deteksi dilakukan sekali di `detect_simd_level()`, hasil di-cache.

/// Level SIMD yang terdeteksi pada CPU ini.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimdLevel {
    /// No SIMD support — scalar fallback
    Scalar,
    /// SSE4.2 available
    Sse42,
    /// AVX2 available
    Avx2,
}

/// Cache SIMD level (detected once).
use std::sync::OnceLock;
static SIMD_LEVEL: OnceLock<SimdLevel> = OnceLock::new();

/// Detect available SIMD level at runtime using CPUID.
///
/// Memanggil `is_x86_feature_detected!()` yang menggunakan CPUID
/// untuk mendeteksi dukungan AVX2 dan SSE4.2.
pub fn detect_simd_level() -> SimdLevel {
    *SIMD_LEVEL.get_or_init(|| {
        #[cfg(target_arch = "x86_64")]
        {
            if is_x86_feature_detected!("avx2") {
                return SimdLevel::Avx2;
            }
            if is_x86_feature_detected!("sse4.2") {
                return SimdLevel::Sse42;
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            // ARM/AARCH64/etc — no x86 SIMD
        }
        SimdLevel::Scalar
    })
}

/// Count leading whitespace bytes in buffer, return count.
///
/// Auto-selects best implementation based on detected SIMD level.
pub fn count_whitespace(buf: &[u8]) -> usize {
    match detect_simd_level() {
        SimdLevel::Avx2 => {
            #[cfg(target_arch = "x86_64")]
            unsafe {
                count_whitespace_avx2(buf)
            }
            #[cfg(not(target_arch = "x86_64"))]
            {
                let _ = buf;
                0
            }
        }
        SimdLevel::Sse42 => {
            #[cfg(target_arch = "x86_64")]
            unsafe {
                count_whitespace_sse42(buf)
            }
            #[cfg(not(target_arch = "x86_64"))]
            {
                let _ = buf;
                0
            }
        }
        SimdLevel::Scalar => count_whitespace_scalar(buf),
    }
}

/// Check if a buffer starts with `//` or `/*` (for comment detection after whitespace).
pub fn is_comment_start(buf: &[u8]) -> bool {
    buf.len() >= 2 && buf[0] == b'/' && (buf[1] == b'/' || buf[1] == b'*')
}

/// Check if buffer starts with backtick (line directive).
pub fn is_backtick(buf: &[u8]) -> bool {
    !buf.is_empty() && buf[0] == b'`'
}

/// Check if all bytes in buffer are whitespace (for SIMD result verification).
pub fn is_all_whitespace(buf: &[u8]) -> bool {
    buf.iter().all(|&b| b == b' ' || b == b'\t' || b == b'\n' || b == b'\r')
}

// ─── AVX2 Implementation ───

/// Count leading whitespace using AVX2 intrinsics.
///
/// Memproses 32 bytes per iterasi. Menggunakan `_mm256_loadu_si256` untuk
/// load unaligned, `_mm256_cmpeq_epi8` untuk comparison, dan
/// `_mm256_movemask_epi8` untuk extract bitmask.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub unsafe fn count_whitespace_avx2(buf: &[u8]) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::*;

        let len = buf.len();
        let ptr = buf.as_ptr();
        let mut count: usize = 0;

        // Process 32 bytes at a time
        let full_chunks = len / 32;
        for i in 0..full_chunks {
            let chunk = _mm256_loadu_si256(ptr.add(i * 32) as *const __m256i);

            // Compare with each whitespace char
            let spaces = _mm256_set1_epi8(b' ' as i8);
            let tabs = _mm256_set1_epi8(b'\t' as i8);
            let newlines = _mm256_set1_epi8(b'\n' as i8);
            let crs = _mm256_set1_epi8(b'\r' as i8);

            let is_space = _mm256_cmpeq_epi8(chunk, spaces);
            let is_tab = _mm256_cmpeq_epi8(chunk, tabs);
            let is_newline = _mm256_cmpeq_epi8(chunk, newlines);
            let is_cr = _mm256_cmpeq_epi8(chunk, crs);

            // Combine: any byte that matches ANY whitespace char
            let is_ws = _mm256_or_si256(
                _mm256_or_si256(is_space, is_tab),
                _mm256_or_si256(is_newline, is_cr),
            );

            let mask = _mm256_movemask_epi8(is_ws) as u32;

            if mask != 0xFFFF_FFFF {
                // Found a non-whitespace byte — count leading whitespace
                count += mask.trailing_zeros() as usize;
                return count;
            }
            count += 32;
        }

        // Handle remainder with scalar
        count += count_whitespace_scalar(&buf[full_chunks * 32..]);
        count
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = buf;
        0
    }
}

// ─── SSE4.2 Implementation ───

/// Count leading whitespace using SSE4.2 intrinsics.
///
/// Memproses 16 bytes per iterasi via `_mm_loadu_si128`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.2")]
pub unsafe fn count_whitespace_sse42(buf: &[u8]) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::*;

        let len = buf.len();
        let ptr = buf.as_ptr();
        let mut count: usize = 0;

        // Process 16 bytes at a time
        let full_chunks = len / 16;
        for i in 0..full_chunks {
            let chunk = _mm_loadu_si128(ptr.add(i * 16) as *const __m128i);

            let spaces = _mm_set1_epi8(b' ' as i8);
            let tabs = _mm_set1_epi8(b'\t' as i8);
            let newlines = _mm_set1_epi8(b'\n' as i8);
            let crs = _mm_set1_epi8(b'\r' as i8);

            let is_space = _mm_cmpeq_epi8(chunk, spaces);
            let is_tab = _mm_cmpeq_epi8(chunk, tabs);
            let is_newline = _mm_cmpeq_epi8(chunk, newlines);
            let is_cr = _mm_cmpeq_epi8(chunk, crs);

            let is_ws = _mm_or_si128(
                _mm_or_si128(is_space, is_tab),
                _mm_or_si128(is_newline, is_cr),
            );

            let mask = _mm_movemask_epi8(is_ws) as u32;

            if mask != 0xFFFF {
                // Found non-whitespace byte
                count += mask.trailing_zeros() as usize;
                return count;
            }
            count += 16;
        }

        count += count_whitespace_scalar(&buf[full_chunks * 16..]);
        count
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = buf;
        0
    }
}

// ─── Scalar Implementation ───

/// Count leading whitespace using scalar byte-by-byte comparison.
pub fn count_whitespace_scalar(buf: &[u8]) -> usize {
    let mut count = 0;
    for &b in buf {
        if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
            count += 1;
        } else {
            break;
        }
    }
    count
}

// ─── Test Utilities ───

#[cfg(test)]
pub(crate) mod test_utils {
    use super::*;

    /// Verify that all SIMD implementations produce the same result as scalar.
    pub fn verify_simd(buf: &[u8]) -> bool {
        let scalar = count_whitespace_scalar(buf);

        #[cfg(target_arch = "x86_64")]
        {
            if is_x86_feature_detected!("avx2") {
                let avx2 = unsafe { count_whitespace_avx2(buf) };
                assert_eq!(
                    scalar, avx2,
                    "AVX2 mismatch for buffer {:?}",
                    std::str::from_utf8(buf).unwrap_or("<invalid>")
                );
            }
            if is_x86_feature_detected!("sse4.2") {
                let sse = unsafe { count_whitespace_sse42(buf) };
                assert_eq!(
                    scalar, sse,
                    "SSE4.2 mismatch for buffer {:?}",
                    std::str::from_utf8(buf).unwrap_or("<invalid>")
                );
            }
        }

        true
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_simd() {
        let level = detect_simd_level();
        #[cfg(target_arch = "x86_64")]
        {
            // Should detect at least SSE4.2 on any modern x86_64 CPU
            assert!(
                level != SimdLevel::Scalar,
                "Expected at least SSE4.2 on x86_64, got Scalar"
            );
            println!("SIMD level: {:?}", level);
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            assert_eq!(level, SimdLevel::Scalar);
        }
    }

    #[test]
    fn test_count_whitespace_scalar_empty() {
        assert_eq!(count_whitespace_scalar(b""), 0);
    }

    #[test]
    fn test_count_whitespace_scalar_spaces() {
        assert_eq!(count_whitespace_scalar(b"   abc"), 3);
        assert_eq!(count_whitespace_scalar(b"\t\t\tabc"), 3);
        assert_eq!(count_whitespace_scalar(b"\n\n\nabc"), 3);
        assert_eq!(count_whitespace_scalar(b"\r\n\t abc"), 4);
    }

    #[test]
    fn test_count_whitespace_scalar_no_ws() {
        assert_eq!(count_whitespace_scalar(b"abc"), 0);
        assert_eq!(count_whitespace_scalar(b""), 0);
    }

    #[test]
    fn test_count_whitespace_scalar_all_ws() {
        assert_eq!(count_whitespace_scalar(b"   \t\n\r  "), 8);
        assert_eq!(count_whitespace_scalar(b"     "), 5);
    }

    #[test]
    fn test_count_whitespace_dispatch() {
        // The dispatch function should return the same as scalar
        assert_eq!(count_whitespace(b"   abc"), 3);
        assert_eq!(count_whitespace(b""), 0);
        assert_eq!(count_whitespace(b"abc"), 0);
        assert_eq!(count_whitespace(b"     "), 5);
        assert_eq!(count_whitespace(b"\n\r\t   x"), 6);
    }

    #[test]
    fn test_count_whitespace_large_buf() {
        // Create a 128-byte buffer of whitespace, then non-whitespace
        let mut buf = vec![b' '; 128];
        buf.push(b'x');

        let count = count_whitespace(&buf);
        assert_eq!(count, 128);
    }

    #[test]
    fn test_count_whitespace_mixed() {
        // Test with various whitespace patterns
        let test_cases = vec![
            (b"" as &[u8], 0usize),
            (b" ", 1),
            (b"  ", 2),
            (b"   ", 3),
            (b"    ", 4),
            (b"     ", 5),
            (b"      ", 6),
            (b"       ", 7),
            (b"        ", 8),
            (b"         ", 9),
            (b"          ", 10),
            (b"           ", 11),
            (b"            ", 12),
            (b"             ", 13),
            (b"              ", 14),
            (b"               ", 15),
            (b"                ", 16),
            (b"                 ", 17),
            (b"                  ", 18),
            (b"                   ", 19),
            (b"                    ", 20),
            (b"x", 0),
            (b" \t\n\r x", 5),
            (b"\x20\x09\x0a\x0d x", 5),
        ];

        for (buf, expected) in &test_cases {
            let result = count_whitespace(buf);
            assert_eq!(
                result, *expected,
                "Failed for buffer {:?} (len={}): expected {}, got {}",
                std::str::from_utf8(buf).unwrap_or("<invalid>"),
                buf.len(),
                expected,
                result
            );
        }
    }

    #[test]
    fn test_count_whitespace_cross_chunk() {
        // Test crossing 16-byte and 32-byte boundaries
        let mut buf = vec![b' '; 40]; // 40 spaces
        buf.push(b'x');

        let count = count_whitespace(&buf);
        assert_eq!(count, 40);
    }

    #[test]
    fn test_count_whitespace_exact_chunks() {
        // Exactly 32 bytes (exact AVX2 chunk)
        let mut buf = vec![b' '; 32];
        buf.push(b'y');
        assert_eq!(count_whitespace(&buf), 32);

        // Exactly 16 bytes (exact SSE chunk)
        let mut buf = vec![b' '; 16];
        buf.push(b'z');
        assert_eq!(count_whitespace(&buf), 16);

        // Exactly 48 bytes (1.5 AVX2 chunks)
        let mut buf = vec![b' '; 48];
        buf.push(b'w');
        assert_eq!(count_whitespace(&buf), 48);
    }

    #[test]
    fn test_is_comment_start() {
        assert!(is_comment_start(b"// comment"));
        assert!(is_comment_start(b"/* block */"));
        assert!(!is_comment_start(b"/ not a comment"));
        assert!(!is_comment_start(b"a / comment"));
        assert!(!is_comment_start(b""));
        assert!(!is_comment_start(b"/"));
    }

    #[test]
    fn test_is_backtick() {
        assert!(is_backtick(b"`line 42"));
        assert!(!is_backtick(b"line 42"));
        assert!(!is_backtick(b""));
    }

    #[test]
    fn test_simd_exact_correctness() {
        // Verify dispatch function matches scalar for all patterns
        let patterns: Vec<&[u8]> = vec![
            b"",
            b" ",
            b"  ",
            b"   ",
            b"    ",
            b"                        x", // 24 spaces + x
            b"                                x", // 32 spaces + x
            b"                                        x", // 40 spaces + x
            b"x",
            b"   x   ",
            b"\n\r\t",
            b"\n\r\t x",
        ];

        for &p in &patterns {
            let dispatch = count_whitespace(p);
            let scalar = count_whitespace_scalar(p);
            assert_eq!(
                dispatch, scalar,
                "Dispatch mismatch for pattern {:?}: dispatch={}, scalar={}",
                std::str::from_utf8(p).unwrap_or("<invalid>"),
                dispatch, scalar
            );
        }
    }
}
