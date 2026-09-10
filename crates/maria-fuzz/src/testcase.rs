//! Testcase fuzzer — representasi kanonik + reproducibility.
//!
//! Backend `MvMediated`: testcase = pasangan (canonical `.mv`, HDL hasil
//! lower). Hash kanonik MV & HDL + seed + scenario_id + mutation trace membuat
//! setiap testcase reproducible penuh (task §9):
//!
//! ```text
//! seed N → MV sama → HDL sama → perilaku Maria sama
//! ```
//!
//! 1 file = 1 tanggung jawab: tipe representasi & metadata, tanpa eksekusi.

use std::time::SystemTime;

/// Backend eksekusi fuzzer (task §14 — backward compatibility).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Jalur lama: generator menghasilkan SV mentah langsung ke Maria.
    Direct,
    /// Jalur baru: fuzzer menghasilkan scenario `.mv` → Maria-MV → HDL → Maria.
    MvMediated,
}

impl Backend {
    pub fn label(&self) -> &'static str {
        match self {
            Backend::Direct => "direct",
            Backend::MvMediated => "maria-mv",
        }
    }
}

/// Hash FNV-1a 64-bit sederhana (bukan kriptografis — cukup deteksi perubahan).
pub fn hash_bytes(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

/// Satu testcase terlantik (mutation chain → lower → siap eksekusi).
#[derive(Debug, Clone)]
pub struct Testcase {
    /// Source `.mv` canonical (reproducible; hash = `mv_hash`).
    pub mv: String,
    /// Hash canonical MV — identitas testcase di corpus/bugdb.
    pub mv_hash: u64,
    /// HDL hasil lower (svh+sv digabung, baris `` `include `` di-strip).
    pub hdl: String,
    /// Hash HDL (bukti determinism lower: MV sama → HDL sama).
    pub hdl_hash: u64,
    /// Seed kampanye RNG.
    pub seed: u64,
    /// ID deterministik testcase dalam kampanye (iterasi parent).
    pub scenario_id: u64,
    /// Trace mutasi ("op12:width", "sem03:seq→comb", ...) — history lengkap.
    pub history: Vec<String>,
    /// Backend asal.
    pub backend: Backend,
    /// Config fingerprint (hang_ms/max_time) utk rerun identik.
    pub cfg_fp: u64,
    /// Waktu pembuatan (epoch detik) — audit reproduksi.
    pub created_at: u64,
}

impl Testcase {
    /// Bangun testcase MV-mediated dari canonical .mv + HDL hasil lower.
    pub fn from_mv(mv: String, hdl: String, seed: u64, scenario_id: u64, history: Vec<String>, cfg_fp: u64) -> Self {
        let mv_hash = hash_bytes(mv.as_bytes());
        let hdl_hash = hash_bytes(hdl.as_bytes());
        Testcase {
            mv,
            mv_hash,
            hdl,
            hdl_hash,
            seed,
            scenario_id,
            history,
            backend: Backend::MvMediated,
            cfg_fp,
            created_at: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        }
    }

    /// Ringkasan satu baris utk log/report.
    pub fn summary(&self) -> String {
        format!(
            "mv={} hdl={} seed={} id={} ops=[{}]",
            self.mv_hash,
            self.hdl_hash,
            self.seed,
            self.scenario_id,
            self.history.join(",")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_stable_and_sensitive() {
        let a = hash_bytes(b"seed-source");
        let b = hash_bytes(b"seed-source");
        let c = hash_bytes(b"seed-sourcE");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn backend_labels() {
        assert_eq!(Backend::Direct.label(), "direct");
        assert_eq!(Backend::MvMediated.label(), "maria-mv");
    }

    #[test]
    fn testcase_fields_and_hash() {
        let tc = Testcase::from_mv(
            "module m { }".to_string(),
            "module m; endmodule".to_string(),
            42,
            3,
            vec!["op0:+".to_string()],
            0x1234,
        );
        assert_eq!(tc.seed, 42);
        assert_eq!(tc.scenario_id, 3);
        assert_eq!(tc.history, vec!["op0:+"]);
        assert_eq!(tc.mv_hash, hash_bytes(b"module m { }"));
        assert!(tc.created_at > 0);
        assert!(tc.summary().contains("mv="));
    }
}