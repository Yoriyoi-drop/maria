//! category — kategori `cache/` db.md (baris 1141-1605).
//!
//! Lapisan cache pipeline memecah cache persisten menjadi satu store per
//! tahap kompilasi: lexer, parser, semantic, elaborate, optimize, verify,
//! preprocess, macro, include, dependency, resolve, constant, generate,
//! expression, type, hierarchy, simulation, waveform, coverage, lint, profile.
//!
//! Setiap kategori memakai struktur seragam (lihat [`super::store`]) sehingga
//! mudah dikelola; perbedaan antar kategori hanya nama + preferensi kompresi
//! (Kritik 15 db.md: tidak semua data cocok dengan algoritma yang sama).

use serde::{Deserialize, Serialize};

use super::super::format::Compression;

/// Kategori cache pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CacheCategory {
    /// Token stream + trivia + line mapping (db.md "2. lexer/").
    Lexer,
    /// Parse tree, AST summary, syntax error, recovery point (db.md "3. parser/").
    Parser,
    /// Resolved symbol/type, scope, width, constant value (db.md "4. semantic/").
    Semantic,
    /// Generate expansion, hierarchy, port binding (db.md "5. elaborate/").
    Elaborate,
    /// Constant folding, dead code, loop unroll (db.md "6. optimize/").
    Optimize,
    /// Verification cache terpisah per kategori analisis (db.md "7. verify/").
    Verify,
    /// Hasil preprocessing: expanded source, include list, defines (db.md "1. preprocess/").
    Preprocess,
    /// Cache makro: define/undef/body/argument/expansion (db.md "13. macro/").
    Macro,
    /// Include tree + dependency + hash (db.md "14. include/").
    Include,
    /// Forward/reverse edge, import, include, param deps (db.md "8. dependency/").
    Dependency,
    /// Cache resolver: name → TypeID/SymbolID lookup (db.md "9. resolve/").
    Resolve,
    /// Constant folding: parameter, localparam, enum value (db.md "11. constant/").
    Constant,
    /// Cache generate if/for/case (db.md "16. generate/").
    Generate,
    /// Cache evaluasi expression: `4+5 → 9` (db.md "10. expression/").
    Expression,
    /// Index tipe: TypeID untuk semua jenis (db.md "12. type/").
    Type,
    /// Hierarki module: Top → CPU → ALU (db.md "15. hierarchy/").
    Hierarchy,
    /// State awal simulasi, sensitivity list, scheduler (db.md "17. simulation/").
    Simulation,
    /// Index sinyal waveform: hierarki, metadata, alias (db.md "18. waveform/").
    Waveform,
    /// Coverage branch/toggle/FSM/statement (db.md "19. coverage/").
    Coverage,
    /// Hasil lint per kategori (db.md "7. verify/ → lint/").
    Lint,
    /// Profil build: timing, memori, worker utilization (db.md "20. profile/").
    Profile,
}

impl CacheCategory {
    /// Semua kategori, urutan stable (urutan db.md).
    pub const ALL: [CacheCategory; 21] = [
        CacheCategory::Preprocess,
        CacheCategory::Lexer,
        CacheCategory::Parser,
        CacheCategory::Semantic,
        CacheCategory::Elaborate,
        CacheCategory::Optimize,
        CacheCategory::Verify,
        CacheCategory::Macro,
        CacheCategory::Include,
        CacheCategory::Dependency,
        CacheCategory::Resolve,
        CacheCategory::Constant,
        CacheCategory::Generate,
        CacheCategory::Expression,
        CacheCategory::Type,
        CacheCategory::Hierarchy,
        CacheCategory::Simulation,
        CacheCategory::Waveform,
        CacheCategory::Coverage,
        CacheCategory::Lint,
        CacheCategory::Profile,
    ];

    /// Nama direktori kategori di dalam `cache/<pid>/`.
    pub const fn name(self) -> &'static str {
        match self {
            CacheCategory::Preprocess => "preprocess",
            CacheCategory::Lexer => "lexer",
            CacheCategory::Parser => "parser",
            CacheCategory::Semantic => "semantic",
            CacheCategory::Elaborate => "elaborate",
            CacheCategory::Optimize => "optimize",
            CacheCategory::Verify => "verify",
            CacheCategory::Macro => "macro",
            CacheCategory::Include => "include",
            CacheCategory::Dependency => "dependency",
            CacheCategory::Resolve => "resolve",
            CacheCategory::Constant => "constant",
            CacheCategory::Generate => "generate",
            CacheCategory::Expression => "expression",
            CacheCategory::Type => "type",
            CacheCategory::Hierarchy => "hierarchy",
            CacheCategory::Simulation => "simulation",
            CacheCategory::Waveform => "waveform",
            CacheCategory::Coverage => "coverage",
            CacheCategory::Lint => "lint",
            CacheCategory::Profile => "profile",
        }
    }

    /// Preferensi kompresi store (Kritik 15 db.md). Data besar/berulang
    /// (AST, graph, source) dikompresi; record kecil (KV) tidak.
    pub const fn compression(self) -> Compression {
        match self {
            // Data besar ala AST/source: LZ4 frame (cepat).
            CacheCategory::Lexer
            | CacheCategory::Parser
            | CacheCategory::Semantic
            | CacheCategory::Elaborate
            | CacheCategory::Optimize
            | CacheCategory::Preprocess
            | CacheCategory::Generate
            | CacheCategory::Coverage => Compression::Lz4,
            // Graph & hierarki: padat → LZ4 (Zstd idealnya, belum tersedia).
            CacheCategory::Dependency | CacheCategory::Hierarchy => Compression::Lz4,
            // Record kecil / KV: tanpa kompresi (overhead tidak sebanding).
            CacheCategory::Verify
            | CacheCategory::Macro
            | CacheCategory::Include
            | CacheCategory::Resolve
            | CacheCategory::Constant
            | CacheCategory::Expression
            | CacheCategory::Type
            | CacheCategory::Simulation
            | CacheCategory::Waveform
            | CacheCategory::Lint
            | CacheCategory::Profile => Compression::None,
        }
    }

    /// Kind byte objek index (unik per kategori).
    pub const fn kind(self) -> u8 {
        64 + match self {
            CacheCategory::Preprocess => 0,
            CacheCategory::Lexer => 1,
            CacheCategory::Parser => 2,
            CacheCategory::Semantic => 3,
            CacheCategory::Elaborate => 4,
            CacheCategory::Optimize => 5,
            CacheCategory::Verify => 6,
            CacheCategory::Macro => 7,
            CacheCategory::Include => 8,
            CacheCategory::Dependency => 9,
            CacheCategory::Resolve => 10,
            CacheCategory::Constant => 11,
            CacheCategory::Generate => 12,
            CacheCategory::Expression => 13,
            CacheCategory::Type => 14,
            CacheCategory::Hierarchy => 15,
            CacheCategory::Simulation => 16,
            CacheCategory::Waveform => 17,
            CacheCategory::Coverage => 18,
            CacheCategory::Lint => 19,
            CacheCategory::Profile => 20,
        }
    }

    /// Balikkan nama → kategori.
    pub fn from_name(s: &str) -> Option<CacheCategory> {
        CacheCategory::ALL.iter().copied().find(|c| c.name() == s)
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_names_unique() {
        let mut names: Vec<&str> = CacheCategory::ALL.iter().map(|c| c.name()).collect();
        let n = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), n, "nama kategori harus unik");
    }

    #[test]
    fn test_name_roundtrip() {
        for c in CacheCategory::ALL {
            assert_eq!(CacheCategory::from_name(c.name()), Some(c));
        }
        assert_eq!(CacheCategory::from_name("nope"), None);
    }

    #[test]
    fn test_kinds_unique() {
        let mut kinds: Vec<u8> = CacheCategory::ALL.iter().map(|c| c.kind()).collect();
        kinds.sort_unstable();
        kinds.dedup();
        assert_eq!(kinds.len(), CacheCategory::ALL.len());
    }

    #[test]
    fn test_compression_assigned() {
        // Kategori data besar harus terkompresi (Kritik 15).
        assert_eq!(CacheCategory::Parser.compression(), Compression::Lz4);
        assert_eq!(CacheCategory::Dependency.compression(), Compression::Lz4);
        // Record kecil tidak perlu kompresi.
        assert_eq!(CacheCategory::Resolve.compression(), Compression::None);
    }
}
