//! AST cache — serialisasi `Design` per file dengan bincode (binary).
//!
//! MICD menyimpan AST tiap file yang sudah di-parse. Pada run berikutnya,
//! jika hash konten file sama, AST di-deserialize langsung → parser di-skip
//! untuk file itu. `Symbol` di-serialisasi sebagai string (bukan u32 index)
//! karena index intern bersifat proses-lokal.

use crate::ast::Design;

/// Versi format serialisasi AST (increment bila skema AST berubah).
pub const AST_FORMAT_VERSION: u64 = 2;

/// Serialisasi `Design` → bytes biner (bincode).
pub fn serialize_design(design: &Design) -> Result<Vec<u8>, String> {
    bincode::serialize(design).map_err(|e| format!("MICD AST serialize: {}", e))
}

/// Deserialisasi `Design` dari bytes. `None` bila format tak dikenal/corrupt
/// (pemanggil fallback ke parse ulang).
pub fn deserialize_design(bytes: &[u8]) -> Option<Design> {
    bincode::deserialize(bytes).ok()
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::CompileSession;
    use crate::SessionConfig;

    #[test]
    fn test_design_roundtrip() {
        let config = SessionConfig {
            sources: vec!["test/counter.sv".into()],
            ..Default::default()
        };
        let mut session = CompileSession::new(config);
        let (design, _) = session.compile().unwrap();
        assert!(!design.modules.is_empty());

        let bytes = serialize_design(&design).expect("serialize");
        let restored = deserialize_design(&bytes).expect("deserialize");
        assert_eq!(restored, design, "Design harus round-trip identik");
    }

    #[test]
    fn test_deserialize_garbage_returns_none() {
        assert!(deserialize_design(&[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01]).is_none());
        assert!(deserialize_design(&[]).is_none());
    }
}
