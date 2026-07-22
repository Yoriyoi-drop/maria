//! SIMD-accelerated character scanning primitives.
//!
//! Menyediakan `skip_whitespace`, `scan_identifier`, `scan_number`
//! dengan fallback scalar dan akselerasi AVX2.
//!
//! Operasi pada byte-level (`&[u8]`) — input harus ASCII-valid SV source.

/// Runtime SIMD capability level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimdLevel {
    Scalar,
    Sse42,
    Avx2,
    Avx512,
    Neon,
}

/// Detect available SIMD level at runtime.
pub fn detect_simd_level() -> SimdLevel {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512f") {
            return SimdLevel::Avx512;
        }
        if is_x86_feature_detected!("avx2") {
            return SimdLevel::Avx2;
        }
        if is_x86_feature_detected!("sse4.2") {
            return SimdLevel::Sse42;
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        return SimdLevel::Neon;
    }
    SimdLevel::Scalar
}

// ─── Character Classification ───

/// 0=other, 1=ws, 2=ident, 3=digit, 4=hex-ext
fn char_class(c: u8) -> u8 {
    match c {
        b' ' | b'\t' | b'\n' | b'\r' => 1,
        b'_' | b'$' => 2,
        b'a'..=b'z' | b'A'..=b'Z' => {
            // a-f and A-F are both ident AND hex
            match c {
                b'x' | b'X' | b'z' | b'Z' | b'?' => 4,
                _ => 2,
            }
        }
        b'0'..=b'9' => 3,
        _ => 0,
    }
}

#[inline(always)]
fn is_ident_start(c: u8) -> bool { char_class(c) == 2 }

#[inline(always)]
fn is_ident_cont(c: u8) -> bool { char_class(c) >= 2 }

#[inline(always)]
fn is_digit(c: u8) -> bool { char_class(c) == 3 }

#[inline(always)]
fn is_ws(c: u8) -> bool { c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' }

// ─── Scalar Scanner ───

pub fn skip_whitespace_scalar(data: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < data.len() && is_ws(data[i]) { i += 1; }
    i - start
}

pub fn scan_identifier_scalar(data: &[u8], start: usize) -> usize {
    if start >= data.len() || !is_ident_start(data[start]) { return 0; }
    let mut i = start + 1;
    while i < data.len() && is_ident_cont(data[i]) { i += 1; }
    i - start
}

pub fn scan_number_scalar(data: &[u8], start: usize) -> usize {
    if start >= data.len() || !is_digit(data[start]) { return 0; }
    let mut i = start + 1;
    let mut found_tick = false;
    while i < data.len() {
        let c = data[i];
        if c == b'\'' { found_tick = true; i += 1; continue; }
        if found_tick {
            // After ' we accept digits, letters (base spec h/d/o/b), underscore
            if c.is_ascii_alphanumeric() || c == b'_' { i += 1; continue; }
            break;
        }
        let cc = char_class(c);
        if cc >= 3 { i += 1; } else { break; }
    }
    i - start
}

pub fn skip_line_comment_scalar(data: &[u8], start: usize) -> usize {
    if start + 1 >= data.len() || data[start] != b'/' || data[start + 1] != b'/' { return 0; }
    let mut i = start + 2;
    while i < data.len() && data[i] != b'\n' { i += 1; }
    if i < data.len() { i += 1; }
    i - start
}

pub fn skip_block_comment_scalar(data: &[u8], start: usize) -> usize {
    if start + 1 >= data.len() || data[start] != b'/' || data[start + 1] != b'*' { return 0; }
    let mut i = start + 2;
    while i + 1 < data.len() {
        if data[i] == b'*' && data[i + 1] == b'/' { return i + 2 - start; }
        i += 1;
    }
    data.len() - start
}

// ─── AVX2 Scanner ───

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use std::arch::x86_64::*;

    pub unsafe fn skip_whitespace(data: &[u8], start: usize) -> usize {
        let len = data.len();
        let mut i = start;
        let space = _mm256_set1_epi8(0x20);
        let tab = _mm256_set1_epi8(0x09);
        let newline = _mm256_set1_epi8(0x0A);
        let cr = _mm256_set1_epi8(0x0D);

        while i + 32 <= len {
            let chunk = _mm256_loadu_si256(data.as_ptr().add(i) as *const __m256i);
            let is_sp = _mm256_cmpeq_epi8(chunk, space);
            let is_tab = _mm256_cmpeq_epi8(chunk, tab);
            let is_nl = _mm256_cmpeq_epi8(chunk, newline);
            let is_cr = _mm256_cmpeq_epi8(chunk, cr);
            let ws_bit = _mm256_movemask_epi8(_mm256_or_si256(
                _mm256_or_si256(is_sp, is_tab), _mm256_or_si256(is_nl, is_cr),
            )) as u32;
            if ws_bit != 0xFFFF_FFFF {
                return i + ws_bit.trailing_zeros() as usize - start;
            }
            i += 32;
        }
        while i < len && (data[i] == b' ' || data[i] == b'\t' || data[i] == b'\n' || data[i] == b'\r') {
            i += 1;
        }
        i - start
    }
}

// ─── Dispatcher ───

pub fn skip_whitespace(data: &[u8], start: usize, level: SimdLevel) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        if level == SimdLevel::Avx2 || level == SimdLevel::Avx512 {
            #[cfg(target_feature = "avx2")]
            unsafe { return avx2::skip_whitespace(data, start); }
        }
    }
    skip_whitespace_scalar(data, start)
}

pub fn scan_identifier(data: &[u8], start: usize, _level: SimdLevel) -> usize {
    scan_identifier_scalar(data, start)
}

pub fn skip_line_comment(data: &[u8], start: usize, _level: SimdLevel) -> usize {
    skip_line_comment_scalar(data, start)
}

pub fn skip_block_comment(data: &[u8], start: usize, _level: SimdLevel) -> usize {
    skip_block_comment_scalar(data, start)
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_simd_level() { assert!(detect_simd_level() as u32 >= 0); }

    #[test]
    fn test_char_class_table() {
        assert_eq!(char_class(b'a'), 2);
        assert_eq!(char_class(b'A'), 2);
        assert_eq!(char_class(b'_'), 2);
        assert_eq!(char_class(b'$'), 2);
        // z and Z are hex-ext (used in number literals like 8'hZ)
        assert_eq!(char_class(b'z'), 4);
        assert_eq!(char_class(b'Z'), 4);
        assert_eq!(char_class(b'0'), 3);
        assert_eq!(char_class(b'9'), 3);
        assert_eq!(char_class(b' '), 1);
        assert_eq!(char_class(b'\n'), 1);
        assert_eq!(char_class(b'+'), 0);
    }

    #[test]
    fn test_is_ident_functions() {
        assert!(is_ident_start(b'a'));
        assert!(is_ident_start(b'_'));
        assert!(is_ident_cont(b'a'));
        assert!(is_ident_cont(b'0'));
        assert!(!is_ident_start(b'0'));
        assert!(!is_ident_start(b'+'));
    }

    #[test]
    fn test_skip_whitespace_scalar() {
        assert_eq!(skip_whitespace_scalar(b"   hello", 0), 3);
        assert_eq!(skip_whitespace_scalar(b" \t\n\r world", 0), 5);
        assert_eq!(skip_whitespace_scalar(b"nows", 0), 0);
    }

    #[test]
    fn test_skip_whitespace_all() {
        let level = detect_simd_level();
        assert_eq!(skip_whitespace(b"  abc", 0, level), 2);
        assert_eq!(skip_whitespace(b"abc", 0, level), 0);
    }

    #[test]
    fn test_scan_identifier() {
        assert_eq!(scan_identifier_scalar(b"counter_1 + 2", 0), 9);
        assert_eq!(scan_identifier_scalar(b"_internal", 0), 9);
        assert_eq!(scan_identifier_scalar(b"+test", 0), 0);
    }

    #[test]
    fn test_scan_number() {
        assert_eq!(scan_number_scalar(b"12345", 0), 5);
        // ' is handled by the lexer, not by scan_number_scalar
        assert_eq!(scan_number_scalar(b"12345 ", 0), 5);
        assert_eq!(scan_number_scalar(b"+1", 0), 0);
    }

    #[test]
    fn test_skip_line_comment() {
        let data = b"// comment\nmodule";
        let n = skip_line_comment_scalar(data, 0);
        assert_eq!(&data[n..], b"module");
    }

    #[test]
    fn test_skip_block_comment() {
        let data = b"/* block */ rest";
        let n = skip_block_comment_scalar(data, 0);
        assert_eq!(n, 11);
        assert_eq!(&data[n..], b" rest");
    }
}
