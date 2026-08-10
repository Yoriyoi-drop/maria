//! verify.mdb — verification cache (reuse hasil analisis modul identik).
//!
//! Kunci utama = content hash. Untuk setiap file dengan hash konten yang sama,
//! hasil verifikasi di-reuse. Dua perbaikan dari db.md:
//!
//! * **Multi-level hash (Kritik 1)**: simpan juga `ast_hash` dan
//!   `semantic_hash`. Bila content hash berubah tapi AST identik (mis. hanya
//!   komentar berubah), verifikasi tetap bisa di-reuse — tidak perlu
//!   lint/verify ulang.
//! * **Verification cache dipisah (Kritik 9)**: hasil tiap kategori analisis
//!   (lint/width/race/xprop/cdc/fsm/dataflow/timing/coverage/assertion)
//!   disimpan terpisah, bukan satu blob — satu kategori dapat di-reuse
//!   independen dari kategori lain.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Kategori analisis verifikasi (Kritik 9 db.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VerifyCheckKind {
    Parse,
    Elaborate,
    Lint,
    Width,
    Race,
    Xprop,
    Cdc,
    Fsm,
    Dataflow,
    Timing,
    Coverage,
    Assertion,
}

impl VerifyCheckKind {
    /// Semua kategori, urutan stable.
    pub const ALL: [VerifyCheckKind; 12] = [
        VerifyCheckKind::Parse,
        VerifyCheckKind::Elaborate,
        VerifyCheckKind::Lint,
        VerifyCheckKind::Width,
        VerifyCheckKind::Race,
        VerifyCheckKind::Xprop,
        VerifyCheckKind::Cdc,
        VerifyCheckKind::Fsm,
        VerifyCheckKind::Dataflow,
        VerifyCheckKind::Timing,
        VerifyCheckKind::Coverage,
        VerifyCheckKind::Assertion,
    ];
}

/// Hasil satu kategori check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    pub ok: bool,
    pub err_count: usize,
    pub warn_count: usize,
    /// Hash hasil analisis kategori ini — reuse artefak turunan bila cocok.
    pub result_hash: u64,
}

impl CheckResult {
    pub fn fresh() -> Self {
        CheckResult {
            ok: false,
            err_count: 0,
            warn_count: 0,
            result_hash: 0,
        }
    }

    pub fn pass(result_hash: u64) -> Self {
        CheckResult {
            ok: true,
            err_count: 0,
            warn_count: 0,
            result_hash,
        }
    }
}

/// Hasil verifikasi satu file (kunci = content hash).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifyResult {
    pub content_hash: u64,
    /// Hash AST (serialized design) — Kritik 1: reuse bila content hash
    /// berubah tapi AST identik (mis. komentar berubah).
    pub ast_hash: u64,
    /// Hash semantic (signature tipe/port ter-resolve) — Kritik 1.
    pub semantic_hash: u64,
    /// Hash IR turunan — Kritik 1 (0 bila belum dihasilkan).
    pub ir_hash: u64,
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
    /// Hasil per-kategori check (Kritik 9) — bukan satu blob.
    pub checks: HashMap<VerifyCheckKind, CheckResult>,
}

impl VerifyResult {
    pub fn fresh(content_hash: u64) -> Self {
        VerifyResult {
            content_hash,
            ast_hash: 0,
            semantic_hash: 0,
            ir_hash: 0,
            parse_ok: false,
            elab_ok: false,
            err_count: 0,
            warn_count: 0,
            info_count: 0,
            parse_ms: 0,
            elab_ms: 0,
            result_hash: 0,
            verified_at_ns: now_ns(),
            checks: HashMap::new(),
        }
    }

    pub fn ok(&self) -> bool {
        self.parse_ok && self.elab_ok && self.err_count == 0
    }

    /// Level 1 multi-level hash: AST identik → verifikasi reuse-able walau
    /// content hash berubah (komentar/format-only change).
    pub fn matches_ast(&self, ast_hash: u64) -> bool {
        self.ast_hash != 0 && self.ast_hash == ast_hash
    }

    /// Level 2: semantic identik → verifikasi reuse-able walau AST berubah
    /// tapi tidak mengubah resolusi tipe/signature.
    pub fn matches_semantic(&self, semantic_hash: u64) -> bool {
        self.semantic_hash != 0 && self.semantic_hash == semantic_hash
    }

    /// Hasil sebuah kategori check.
    pub fn check(&self, kind: VerifyCheckKind) -> Option<&CheckResult> {
        self.checks.get(&kind)
    }

    /// Apakah kategori `kind` reuse-able untuk `result_hash` ini (Kritik 9)?
    pub fn check_reusable(&self, kind: VerifyCheckKind, result_hash: u64) -> bool {
        self.checks
            .get(&kind)
            .map(|c| c.ok && c.result_hash == result_hash)
            .unwrap_or(false)
    }

    /// Set hasil kategori check.
    pub fn set_check(&mut self, kind: VerifyCheckKind, r: CheckResult) {
        self.checks.insert(kind, r);
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
        let mut v = VerifyResult::fresh(42);
        v.ast_hash = 0xA11;
        v.semantic_hash = 0x51e;
        v.parse_ok = true;
        v.elab_ok = true;
        v.warn_count = 1;
        v.set_check(VerifyCheckKind::Width, CheckResult::pass(0x1234));
        let bytes = bincode::serialize(&v).unwrap();
        let v2: VerifyResult = bincode::deserialize(&bytes).unwrap();
        assert_eq!(v, v2);
        assert!(v.ok());
    }

    #[test]
    fn test_multi_level_hash_reuse() {
        // Comment-only change: content hash beda, AST hash sama.
        let mut v = VerifyResult::fresh(111);
        v.ast_hash = 0xAAAA;
        v.semantic_hash = 0xBBBB;
        v.parse_ok = true;
        v.elab_ok = true;
        assert!(v.matches_ast(0xAAAA));
        assert!(!v.matches_ast(0xCCCC));
        // AST berubah, semantic sama → reuse level 2.
        assert!(v.matches_semantic(0xBBBB));
        // Belum di-set → tidak reuse.
        let fresh = VerifyResult::fresh(222);
        assert!(!fresh.matches_ast(0xAAAA));
    }

    #[test]
    fn test_check_split_reuse_independent() {
        let mut v = VerifyResult::fresh(7);
        v.set_check(VerifyCheckKind::Lint, CheckResult::pass(1));
        v.set_check(VerifyCheckKind::Width, CheckResult::pass(2));
        // Lint hash beda → tidak reuse kategori itu.
        assert!(v.check_reusable(VerifyCheckKind::Lint, 1));
        assert!(!v.check_reusable(VerifyCheckKind::Lint, 99));
        // Kategori lain tetap reuse-able.
        assert!(v.check_reusable(VerifyCheckKind::Width, 2));
        // Kategori yang tidak pernah dijalankan → tidak reuse.
        assert!(!v.check_reusable(VerifyCheckKind::Fsm, 2));
        // Error pada kategori → tidak reuse.
        let mut bad = CheckResult::fresh();
        bad.ok = false;
        bad.err_count = 1;
        v.set_check(VerifyCheckKind::Race, bad);
        assert!(!v.check_reusable(VerifyCheckKind::Race, 0));
    }
}
