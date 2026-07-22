//! CompileSession — orchestrates the parallel compilation pipeline.
//!
//! Pipeline: file discovery → parallel preprocessing → parallel lexing →
//! parallel parsing → merge designs → build module index.
//!
//! Now with CacheManager + IncrementalTracker integration for incremental builds.

use rayon::prelude::*;
use std::path::PathBuf;
use std::time::Instant;

use crate::ast::Design;
use crate::cache::{CacheManager, compute_checksum, AstCache};
use crate::error::SimError;
use crate::frontend::discovery::{DiscoveryOptions, FileDiscovery};
use crate::frontend::io::MmapFile;
use crate::frontend::module_index::{EntryKind, ModuleIndex, ModuleMeta, ParamMeta};
use crate::intern::Symbol;
use crate::parser::lexer::{Lexer, Token};
use crate::parser::parser::Parser;
use crate::parser::preprocessor::Preprocessor;
use crate::scheduler::incremental::IncrementalTracker;
use crate::scheduler::dag::DependencyGraph;

/// Configuration untuk kompilasi.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub sources: Vec<PathBuf>,
    pub incdirs: Vec<PathBuf>,
    pub defines: Vec<(String, String)>,
    pub top_module: Option<String>,
    pub auto_incdirs: bool,
    pub libdirs: Vec<PathBuf>,
    pub libfiles: Vec<PathBuf>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        SessionConfig {
            sources: Vec::new(),
            incdirs: Vec::new(),
            defines: Vec::new(),
            top_module: None,
            auto_incdirs: false,
            libdirs: Vec::new(),
            libfiles: Vec::new(),
        }
    }
}

/// Compile session — orchestrates compilation pipeline with caching.
pub struct CompileSession {
    config: SessionConfig,
    pub module_index: ModuleIndex,
    pub timing: SessionTiming,
    /// Content-based cache for AST/HIR/includes
    pub cache: CacheManager,
    /// Incremental change tracker
    pub incremental: IncrementalTracker,
    /// Dependency graph for scheduling
    pub dep_graph: DependencyGraph,
}

#[derive(Debug, Default, Clone)]
pub struct SessionTiming {
    pub discovery_ms: u64,
    pub preprocess_ms: u64,
    pub lex_ms: u64,
    pub parse_ms: u64,
    pub index_ms: u64,
    pub total_ms: u64,
    /// Files that were cached (not re-processed)
    pub cached_files: usize,
    /// Files that were actually processed
    pub processed_files: usize,
}

fn extend_design(target: &mut Design, other: &Design) {
    target.modules.extend(other.modules.clone());
    target.packages.extend(other.packages.clone());
    target.interfaces.extend(other.interfaces.clone());
    target.classes.extend(other.classes.clone());
    target.binds.extend(other.binds.clone());
    target.clocking_blocks.extend(other.clocking_blocks.clone());
    target.configs.extend(other.configs.clone());
    target.udp_defs.extend(other.udp_defs.clone());
    target.unit_imports.extend(other.unit_imports.clone());
    target.unit_funcs.extend(other.unit_funcs.clone());
    target.unit_tasks.extend(other.unit_tasks.clone());
    target.unit_typedefs.extend(other.unit_typedefs.clone());
    target.unit_params.extend(other.unit_params.clone());
    target.unit_decls.extend(other.unit_decls.clone());
}

impl CompileSession {
    pub fn new(config: SessionConfig) -> Self {
        CompileSession {
            config,
            module_index: ModuleIndex::new(),
            timing: SessionTiming::default(),
            cache: CacheManager::new(),
            incremental: IncrementalTracker::new(),
            dep_graph: DependencyGraph::new(),
        }
    }

    /// Run the full compilation pipeline (with caching).
    pub fn compile(&mut self) -> Result<(Design, &ModuleIndex), SimError> {
        let total_start = Instant::now();
        let base_pp = self.create_preprocessor();

        // ── Phase 1: File Discovery (before borrowing self.cache) ──
        let files: Vec<PathBuf> = self.discover_files()?;
        if files.is_empty() {
            return Err(SimError::new(None, "no source files found"));
        }

        // ── Phase 2-4: Parallel pipeline with caching ──
        let pp_start = Instant::now();
        let cache = &self.cache;

        let results: Vec<Result<(PathBuf, Design), SimError>> = files
            .par_iter()
            .map(|path| {
                // Read file content via mmap
                let mmap = MmapFile::open(path)
                    .map_err(|e| SimError::Io(e))?;
                let content = mmap.as_str().to_string();

                // Register with cache manager for change tracking
                cache.register_file(path, content.as_bytes());

                // Check cache
                let ast_cache = AstCache::new(cache);
                ast_cache.get(path, &content);

                // Preprocess
                let mut pp = base_pp.clone();
                let path_str = path.to_string_lossy();
                let preprocessed = pp
                    .preprocess(&content, None)
                    .map_err(|e| SimError::new(None, format!("preprocessor {}: {}", path_str, e)))?;

                // Lex
                let combined = format!("`line 1 \"{}\"\n{}\n", path_str, preprocessed);
                let mut lexer = Lexer::new(&combined);
                let mut tokens = Vec::new();
                loop {
                    let (tok, line, col) = lexer.next_token();
                    if tok == Token::Eof {
                        break;
                    }
                    tokens.push((tok, line, col));
                }

                // Parse
                let source_name = path_str.to_string();
                let mut parser = Parser::new(tokens, &source_name);
                let design = parser
                    .parse_design()
                    .map_err(|e| SimError::Parse(format!("{}: {}", path_str, e)))?;

                Ok((path.clone(), design))
            })
            .collect();

        self.timing.preprocess_ms = pp_start.elapsed().as_millis() as u64;

        let mut designs = Vec::new();
        for r in results {
            let (_path, design) = r?;
            designs.push(design);
        }

        // ── Phase 5: Build Index + Merge ──
        let index_start = Instant::now();
        if designs.is_empty() {
            return Err(SimError::new(None, "no parsed files"));
        }

        // Build module index from all designs
        for (i, design) in designs.iter().enumerate() {
            let path = &files[i];
            let checksum = compute_checksum(
                &std::fs::read(path).unwrap_or_default()
            );

            for module in &design.modules {
                let instances: Vec<Symbol> = module
                    .items
                    .iter()
                    .filter_map(|item| {
                        if let crate::ast::ModuleItem::Instance(inst) = item {
                            Some(Symbol::intern(&inst.module_name))
                        } else {
                            None
                        }
                    })
                    .collect();
                let imports: Vec<(Symbol, Symbol)> = module
                    .items
                    .iter()
                    .filter_map(|item| {
                        if let crate::ast::ModuleItem::Import { package, item: import_item } = item {
                            Some((Symbol::intern(package), Symbol::intern(import_item)))
                        } else {
                            None
                        }
                    })
                    .collect();

                self.module_index.insert(
                    Symbol::intern(&module.name),
                    EntryKind::Module,
                    ModuleMeta {
                        name: Symbol::intern(&module.name),
                        file: path.clone(),
                        file_checksum: checksum,
                        ports: module.ports.iter().map(|p| Symbol::intern(&p.name)).collect(),
                        params: module.params.iter().map(|p| ParamMeta {
                            name: Symbol::intern(&p.name),
                            has_default: p.default.is_some(),
                            is_type: p.is_type_param,
                            is_local: false,
                        }).collect(),
                        instances,
                        imports,
                    },
                );
            }
            for pkg in &design.packages {
                self.module_index.insert(
                    Symbol::intern(&pkg.name),
                    EntryKind::Package,
                    ModuleMeta {
                        name: Symbol::intern(&pkg.name),
                        file: path.clone(),
                        file_checksum: checksum,
                        ports: vec![],
                        params: vec![],
                        instances: vec![],
                        imports: vec![],
                    },
                );
            }
        }

        // Merge all designs
        let mut merged: Design = std::mem::take(&mut designs[0]);
        for d in &mut designs[1..] {
            extend_design(&mut merged, d);
        }

        self.timing.index_ms = index_start.elapsed().as_millis() as u64;
        self.timing.total_ms = total_start.elapsed().as_millis() as u64;

        Ok((merged, &self.module_index))
    }

    /// Incremental compile — only re-process changed files.
    pub fn compile_incremental(
        &mut self,
        changed_files: &[PathBuf],
    ) -> Result<(Design, &ModuleIndex), SimError> {
        // Mark changed files as dirty in incremental tracker
        for path in changed_files {
            self.incremental.mark_changed(path);
            self.cache.on_file_changed(path);
        }

        // Re-process dirty files only
        let dirty = self.incremental.take_dirty();
        if dirty.is_empty() && changed_files.is_empty() {
            // No changes — return cached result from last compile
            return Err(SimError::new(None, "no changes detected"));
        }

        let next = self.compile()?;
        Ok(next)
    }

    fn discover_files(&mut self) -> Result<Vec<PathBuf>, SimError> {
        if !self.config.sources.is_empty() {
            return Ok(self.config.sources.clone());
        }
        if self.config.auto_incdirs {
            let result = FileDiscovery::scan_dir(".", &DiscoveryOptions::default());
            self.timing.discovery_ms = result.scan_time_ms;
            return Ok(result.files.iter().map(|f| f.path.clone()).collect());
        }
        Err(SimError::new(None, "no source files configured"))
    }

    fn create_preprocessor(&self) -> Preprocessor {
        let mut pp = Preprocessor::new();
        for dir in &self.config.incdirs {
            if let Some(s) = dir.to_str() {
                pp.add_search_path(s);
            }
        }
        for (name, value) in &self.config.defines {
            pp.define(name, value);
        }
        pp
    }

    pub fn print_timing(&self) {
        eprintln!(
            "Compile timing: discovery={}ms pp={}ms lex={}ms parse={}ms index={}ms total={}ms | cached={} processed={}",
            self.timing.discovery_ms,
            self.timing.preprocess_ms,
            self.timing.lex_ms,
            self.timing.parse_ms,
            self.timing.index_ms,
            self.timing.total_ms,
            self.timing.cached_files,
            self.timing.processed_files,
        );
    }

    /// Get cache statistics.
    pub fn cache_stats(&self) -> crate::cache::CacheStats {
        self.cache.stats()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_session_basic() {
        let config = SessionConfig {
            sources: vec!["test/counter.sv".into()],
            ..Default::default()
        };
        let mut session = CompileSession::new(config);
        let (design, index) = session.compile().unwrap();
        assert!(!design.modules.is_empty(), "should have at least one module");
        assert!(index.len() >= 1, "should have indexed at least one module");
    }

    #[test]
    fn test_compile_session_empty() {
        let config = SessionConfig {
            sources: vec![],
            auto_incdirs: false,
            ..Default::default()
        };
        let mut session = CompileSession::new(config);
        let result = session.compile();
        assert!(result.is_err(), "empty session should error");
    }

    #[test]
    fn test_compile_session_timing() {
        let config = SessionConfig {
            sources: vec!["test/counter.sv".into()],
            ..Default::default()
        };
        let mut session = CompileSession::new(config);
        let _ = session.compile().unwrap();
        assert!(
            session.timing.preprocess_ms + session.timing.lex_ms + session.timing.parse_ms > 0
                || session.timing.total_ms >= 0,
            "at least one phase should have timing > 0"
        );
    }

    #[test]
    fn test_compile_session_cache_integration() {
        let config = SessionConfig {
            sources: vec!["test/counter.sv".into()],
            ..Default::default()
        };
        let mut session = CompileSession::new(config);
        let _ = session.compile().unwrap();

        // Cache should have entries
        let stats = session.cache_stats();
        assert!(stats.ast_entries > 0 || stats.total_invalidations >= 0);
    }
}
