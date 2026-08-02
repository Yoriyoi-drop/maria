//! verify.mdb — verification cache (reuse hasil analisis modul identik).
//!
//! Untuk setiap file dengan hash konten yang sama, hasil verifikasi (parse,
//! jumlah diagnostic, timing) di-reuse — tidak perlu lint/verifikasi ulang.
//! Terhubung ke content hash, bukan path, sehingga file yang sama persis
//! di lokasi berbeda juga berbagi entri.

use serde::{Deserialize, Serialize};

/// Hasil verifikasi satu file (kunci = content hash).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifyResult {
    pub content_hash: u64,
    pub parse_ok: bool,
    pub elab_ok: bool,
    /// Jumlah diagnostic error.
    pub err_count: usize,
    /// Jumlah warning.
    pub warn_count: usize,
    /// Jumlah info/hint.
    pub info_count: usize,
    /// Waktu parse (ms).
    pub parse_ms: u64,
    /// Waktu elaborate (ms).
    pub elab_ms: u64,
    /// Hash hasil IR (untuk reuse artefak turunan).
    pub result_hash: u64,
    /// Waktu verifikasi (unix ns).
    pub verified_at_ns: u64,
}

impl VerifyResult {
    pub fn fresh(content_hash: u64) -> Self {
        VerifyResult {
            content_hash,
            parse_ok: false,
            elab_ok: false,
            err_count: 0,
            warn_count: 0,
            info_count: 0,
            parse_ms: 0,
            elab_ms: 0,
            result_hash: 0,
            verified_at_ns: now_ns(),
        }
    }

    pub fn ok(&self) -> bool {
        self.parse_ok && self.elab_ok && self.err_count == 0
    }
}

/// Waktu unix dalam nanoseconds.
pub fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_roundtrip() {
        let v = VerifyResult {
            content_hash: 42,
            parse_ok: true,
            elab_ok: true,
            err_count: 0,
            warn_count: 1,
            info_count: 2,
            parse_ms: 10,
            elab_ms: 20,
            result_hash: 999,
            verified_at_ns: 123,
        };
        let bytes = bincode::serialize(&v).unwrap();
        let v2: VerifyResult = bincode::deserialize(&bytes).unwrap();
        assert_eq!(v, v2);
        assert!(v.ok());
    }
}
