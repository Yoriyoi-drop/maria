# Maria Redesign — Roadmap Implementasi

> Roadmap ini berisi langkah-langkah konkret implementasi redesign Maria, termasuk dependencies, target waktu, dan milestone.

---

## Fase 0: Foundation (Minggu 1-3)

### Target: Arsitektur modular + arena allocator + string intern

#### Step 0.1 — Struktur Folder Baru
```bash
# Buat struktur folder baru
mkdir -p src/{frontend/{lexer,parser,preprocessor},ast,hir,mir,backend/{simulator,waveform},scheduler,cache,diagnostics,arena,intern,profiling,plugin}
# Pindahkan file existing
mv src/parser src/frontend/
mv src/ast/* src/ast/
mv src/ir/* src/hir/
mv src/simulator/* src/backend/simulator/
mv src/waveform/* src/backend/waveform/
mv src/debugger src/backend/
```

#### Step 0.2 — Dependencies Baru (Cargo.toml)
```toml
[dependencies]
# Existing
clap = { version = "4", features = ["derive"] }
rand = "0.8"
rayon = "1"
thiserror = "2"
wavefst = { version = "0.1", default-features = false, features = ["gzip"] }

# New — Phase 0
dashmap = "6"                           # Concurrent HashMap
crossbeam = "0.8"                       # Lock-free primitives
xxhash-rust = { version = "0.8", features = ["xxh3"] }  # Fast hashing
mimalloc = { version = "0.1", features = ["secure"] }   # Global allocator

# New — Phase 1
memmap2 = "0.9"                         # Memory-mapped files
num_cpus = "1"                          # CPU detection

# New — Phase 4+
walkdir = "2"                           # Directory walker
fxhash = "0.2"                          # Fast hash
thread_local = "1"                      # Thread-local storage
parking_lot = "0.12"                    # Fast mutex/rwlock

[profile.release]
opt-level = 3
lto = true
codegen-units = 1
```

#### Step 0.3 — Arena Allocator
- Implementasi `BumpArena` dengan chunk doubling (64KB → 16MB)
- Implementasi `TypedArena<T>` wrapper
- Thread-local arena per worker
- Test: allocation throughput benchmark

#### Step 0.4 — String Intern
- `ConcurrentStringTable` dengan DashMap
- `Symbol` sebagai u32 index
- Thread-local intern cache untuk mengurangi contention
- Test: 1M unique strings, measure time + memory

#### Step 0.5 — Immutable AST
- `AstNode` trait dengan `Send + Sync`
- `NodeRef = u32` sebagai arena index
- Migrasi semua existing AST node ke arena
- Hapus `#[derive(Clone)]` dari AST nodes (tidak perlu clone)
- Ganti `Box<Expr>` dengan `NodeRef`

#### Step 0.6 — Span Integration
- `Span { file: Symbol, start: Offset, end: Offset }`
- Semua node memiliki span
- Error messages menggunakan span untuk lokasi presisi

**Milestone Fase 0:** Semua test existing lulus, memory usage turun 40%, allocation speed 3x lebih cepat.

---

## Fase 1: Parallelism (Minggu 4-7)

### Target: Semua file diproses paralel

#### Step 1.1 — Parallel File Discovery
- `discover_files()` using `walkdir` + rayon `par_iter`
- Skip hidden dirs, `.git`, `node_modules`
- Output: `Vec<FileEntry>`

#### Step 1.2 — Per-File Preprocessing
```rust
// Preprocess each file in parallel
let preprocessed: Vec<PreprocessedFile> = files.par_iter()
    .map(|file| preprocess_file(file, &base_pp))
    .collect();
```

#### Step 1.3 — SIMD Lexer (Scalar Fallback)
```rust
// Per-file lexing in parallel
let token_streams: Vec<Arc<TokenStream>> = files.par_iter()
    .map(|file| {
        let lexer = SimdLexer::new(file.content());
        lexer.tokenize()
    })
    .collect();
```

#### Step 1.4 — Parallel Parser
```rust
// Per-file parsing in parallel
let asts: Vec<Arc<ArenaAst>> = token_streams.par_iter()
    .map(|tokens| {
        let arena = TypedArena::new();
        let parser = Parser::new(tokens, &arena);
        parser.parse_file()
    })
    .collect();
```

#### Step 1.5 — Module Index
```rust
// Global module index (after all files parsed)
let module_index: DashMap<Symbol, ModuleMeta> = DashMap::new();
for ast in &asts {
    for module in ast.modules() {
        module_index.insert(module.name, ModuleMeta {
            file: module.file,
            checksum: module.checksum,
            ports: module.ports().to_vec(),
            params: module.params().to_vec(),
            dependencies: module.dependencies(),
        });
    }
}
```

**Milestone Fase 1:** 100 file diparse dalam <100ms pada 16-core. Speedup linear sampai 16 threads.

---

## Fase 2: Caching (Minggu 8-10)

### Target: Incremental build untuk project medium

#### Step 2.1 — Content Checksum Cache
```rust
struct FileCache {
    entries: DashMap<PathBuf, FileCacheEntry>,
}

struct FileCacheEntry {
    checksum: u64,        // xxhash3
    mtime: SystemTime,
    size: u64,
}
```

#### Step 2.2 — AST Cache
```rust
struct AstCache {
    entries: DashMap<CacheKey, Arc<ArenaAst>>,
    lru: Mutex<LruList>,
    memory_budget: AtomicU64,
}
```

#### Step 2.3 — Dependency Graph
```rust
struct DepGraph {
    nodes: FxHashMap<Symbol, DepNodeInfo>,
    edges: Vec<DepEdge>,
    reverse_edges: Vec<Vec<usize>>,
    topo_order: Vec<Symbol>,
    dirty: FxHashSet<Symbol>,
}
```

#### Step 2.4 — Include & Macro Cache
```rust
struct IncludeCache {
    entries: DashMap<(Symbol, u64), Arc<String>>,
}
struct MacroCache {
    entries: DashMap<(Symbol, u64, u64), Arc<String>>,
}
```

#### Step 2.5 — Incremental Pipeline
```rust
fn compile_incremental(changes: &[PathBuf]) -> Result<IrDesign> {
    // 1. Update dependency graph
    for path in changes { dep_graph.mark_dirty(path); }

    // 2. Schedule only dirty subgraph
    let tasks = dep_graph.dirty_tasks();
    scheduler.schedule(tasks);

    // 3. Reuse cached results for unchanged modules
    let hir = hir_cache.get_or_elaborate(top);
    Ok(hir)
}
```

**Milestone Fase 2:** Incremental compile <0.5s untuk single file change di project 1000 files.

---

## Fase 3: Lazy Evaluation (Minggu 11-14)

### Target: Elaborasi on-demand, bukan full project

#### Step 3.1 — Lazy Type Resolution
```rust
struct LazyTypeResolver {
    type_cache: DashMap<(Symbol, Symbol), Arc<ResolvedType>>,
    module_table: Arc<ModuleIndex>,
}

impl LazyTypeResolver {
    fn resolve(&self, module: Symbol, type_name: Symbol) -> Arc<ResolvedType> {
        self.type_cache
            .entry((module, type_name))
            .or_insert_with(|| self.resolve_impl(module, type_name))
            .clone()
    }
}
```

#### Step 3.2 — Lazy Elaborator
```rust
impl LazyElaborator {
    fn elaborate(&self, module: Symbol) -> Result<Arc<HirModule>> {
        // 1. Check cache
        if let Some(hir) = self.hir_cache.get(&module) {
            return Ok(hir);
        }
        // 2. Elaborate dependencies first
        let deps = self.dep_graph.dependencies(module);
        for dep in deps { self.elaborate(dep)?; }
        // 3. Elaborate this module
        let hir = self.elaborate_one(module)?;
        self.hir_cache.insert(module, Arc::new(hir));
        Ok(self.hir_cache.get(&module).unwrap())
    }
}
```

#### Step 3.3 — HIR Cache
```rust
struct HirCache {
    entries: DashMap<Symbol, Arc<HirModule>>,
    dep_versions: DashMap<Symbol, u64>,  // version for invalidation
}
```

**Milestone Fase 3:** Elaborasi partial 5x lebih cepat dari elaborasi full untuk project besar.

---

## Fase 4: Diagnostics (Minggu 15-16)

### Target: Error recovery di semua pipeline

- [ ] Diagnostic types + codes
- [ ] Thread-safe diagnostic sink (crossbeam channel)
- [ ] Parser error recovery (sync token strategy)
- [ ] Recovery strategies: insert, delete, replace token
- [ ] Formatted terminal output (like Rust compiler)
- [ ] LSP-compatible JSON output

---

## Fase 5: Profiling & Optimization (Minggu 17-19)

### Target: Built-in profiler + data-driven optimization

- [ ] Atomic performance counters
- [ ] Per-phase timing
- [ ] Memory tracking (arena usage, cache usage)
- [ ] Thread utilization histogram
- [ ] SIMD: AVX2 whitespace/identifier scanning
- [ ] SIMD: ARM NEON support
- [ ] MMAP I/O optimization
- [ ] Benchmark vs Verilator

---

## Fase 6: Integration (Ongoing)

### Target: Production-ready untuk project besar

- [ ] OpenTitan full compile test
- [ ] CVA6 full compile test
- [ ] Synthetic 10K module test
- [ ] Memory profiling with heaptrack/valgrind
- [ ] Thread sanitizer tests
- [ ] Performance regression CI

---

## Quick Start — Langkah Pertama

```bash
# 1. Install dependencies
cargo add dashmap crossbeam xxhash-rust mimalloc memmap2

# 2. Buat struktur folder baru
mkdir -p src/{arena,intern,cache,scheduler,diagnostics,profiling}

# 3. Implementasi arena allocator
cat > src/arena/mod.rs << 'EOF'
pub mod bump;
pub mod typed;
pub use bump::BumpArena;
pub use typed::TypedArena;
EOF

# 4. Implementasi string intern
cat > src/intern/mod.rs << 'EOF'
pub mod string_intern;
pub use string_intern::*;
EOF

# 5. Update lib.rs
# Tambahkan mod declarations baru

# 6. Test
cargo test
```

---

## Referensi Cepat — Cargo Commands

```bash
# Build with mimalloc
RUSTFLAGS="-C target-cpu=native" cargo build --release

# Test with thread sanitizer
RUSTFLAGS="-Z sanitizer=thread" cargo test -Zbuild-std

# Memory profiling
cargo install heaptrack
heaptrack cargo test --release

# Flamegraph for CPU profiling
cargo install flamegraph
cargo flamegraph --bin maria -- test/counter.sv

# Benchmark
cargo bench
```
