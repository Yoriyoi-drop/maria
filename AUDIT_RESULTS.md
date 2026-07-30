# Audit Performa — maria RTL Simulator

Tanggal: 2026-07-30  
Total file .rs: ~180  
Total baris: ~70.700

---

## RINGKASAN EKSEKUTIF

Project ini memiliki **masalah arsitektur fundamental** yang akan membatasi skalabilitas:

1. **Parser 2-pass** — token stream dilewati 2x penuh, packages diparse 2x (hasil pass 1 dibuang)
2. **Zero arena allocation di parser** — ~150+ `Box::new()` per design, 13 Vecs stack-local per `parse_design()`
3. **Clone berlebihan di elaborator** — 237+ `.clone()` di elaborator, termasuk full AST module clone per instance
4. **~68 parse functions** dengan mutual recursion dalam, 742 `.peek()` calls
5. **String interner bottleneck** — `parking_lot::Mutex` serializes semua `Symbol::as_str()`
6. **Preprocessor replace() O(n²)** — `String::replace()` per parameter di tiap ekspansi macro
7. **File dibaca 2x** — MmapFile::open + std::fs::read() untuk checksum di compile_session.rs

---

## 1. BOTTLENECK CPU

### Top 10 fungsi paling mahal (estimasi)

| Rank | Function | File | Line | Kompleksitas | Penyebab |
|------|----------|------|------|--------------|----------|
| 1 | `elaborate_module_with_params_and_type` | `elaboration/elaborator/mod.rs` | 1005 | O(M²·I·S) | 5 pass module items, clone berat, linear scan signals |
| 2 | `parse_design` | `parser/mod.rs` | 239 | O(2N + N·skip) | 2 full token passes, skip_to_next_top_level O(n) inner |
| 3 | `expand_inline_macros_depth` | `parser/preprocessor.rs` | 455 | O(n·m·p) | String::replace() per parameter, Vec<char> alloc per call |
| 4 | `substitute_ident_in_expr` | `elaboration/elaborator/expr.rs` | 929 | O(N·depth) | Clone berat tiap recursive call, 30+ clones per descent |
| 5 | `flatten_module` | `elaboration/elaborator/flatten.rs` | 17 | O(I·M) | clone full AST module per instance, linear find O(M) |
| 6 | `parse_module_item_body` | `parser/mod.rs` | 735 | O(n) per item | Switch raksasa, 14 cabang, banyak Ok(None) + stuck detection |
| 7 | `read_ident_or_keyword` | `parser/lexer.rs` | 578 | O(n) per token | String s dibangun lalu dibuang untuk keyword (mayoritas token) |
| 8 | `elaborate` | `elaboration/elaborator/mod.rs` | 165 | O(M·8) | Modules list di-iterasi 8x untuk tujuan berbeda |
| 9 | `parse_module_fast` + `parse_module` | `parser/instance.rs` | 14-223 | O(2N) | Module di-scan 2x (fast + full), harusnya 1x |
| 10 | `Symbol::as_str` | `intern/string_intern.rs` | 322 | O(1) but serialized | parking_lot::Mutex global serializes all reads |

### Hotspot clone()
- **237+** panggilan `.clone()` di elaborator
- **107+** di parser (total 14 file)
- **150+** `Box::new()` di parser (heap alloc, no arena)

### Hotspot format!()
- **66** di elaborator (beberapa di hot path per-iteration)
- **52** di parser
- **16** di preprocessor

---

## 2. AUDIT PARSER

### Struktur Parse Pass

```
parse_design():
  ├─ Pass 1: class discovery (line 275-432)
  │   ├─ parse_module_fast()   → skip body, collect class names
  │   ├─ parse_class_fast()    → collect class name only
  │   ├─ parse_package_decl()  → FULL PARSE (hasil dibuang!)
  │   └─ skip untuk construct lain
  │
  ├─ saved_pos restore (line 434)
  │
  └─ Pass 2: full parse (line 445-690)
      ├─ parse_module()        → FULL PARSE
      ├─ parse_class()         → FULL PARSE
      ├─ parse_package_decl()  → FULL PARSE (lagi!)
      └─ parse untuk sisanya
```

### Duplicate Work — Packages Parse 2x
- `parser/mod.rs:312` — pass 1: `parse_package_decl()` full parse
- `parser/mod.rs:436` — `packages.clear()` — **hasil pass 1 dibuang**
- `parser/mod.rs:483` — pass 2: `parse_package_decl()` full parse lagi

**Dampak**: 2x parsing untuk setiap package. Untuk 100 file dengan 50 packages = 50x parsing sia-sia.

### Jumlah Parse Pass
- **2 full passes** token stream (+ partial per module/class/package body)
- Setiap module body full traversal di pass 2
- Setiap class body full traversal di pass 2

### Token Traversal Count
- 742 `.peek()` panggilan
- ~450 `.advance()` panggilan
- ~150 `.expect()` panggilan
- Stuck detection loop: `while self.pos == before` di instance.rs:116

### Risiko Infinite Loop / Hang
| Lokasi | Baris | Mekanisme | Risiko |
|--------|-------|-----------|--------|
| `parse_module_item_body` final `_ => Ok(None)` | `mod.rs:1202` | Fallback tanpa advance | Tertangkap guard di instance.rs:116 |
| `skip_to_next_top_level` + Eof | `mod.rs:1211-1250` | Return tanpa advance saat Eof | Aman (loop parent lihat Eof) |
| Stuck detection (1M iter) | `mod.rs:279, 449` | Counter stuck | Safety net untuk loop tak maju |
| `skip_balanced_paren_light` + Eof | `mod.rs:1374` | Return tanpa advance | Aman |
| `skip_class_body` header loop | `mod.rs:1316` | Return jika EndClass/Eof | Aman |

### Backtracking Sites (5 total)
- `instance.rs:867` — drive strength detection: `self.pos = saved`
- `stmt.rs:694` — case vs case_inside peek-ahead
- `specify.rs:188, 390` — timing check detection
- `instance.rs:188` — class name extraction in fast pass

### Kompleksitas Teoritis vs Aktual
| Fungsi | Teoritis | Aktual | Alasan |
|--------|----------|--------|--------|
| `parse_design` | O(N) | O(2N + N·skip) | 2 pass + error recovery skip O(n) |
| `parse_module_item_body` | O(1) per item | O(n) per item | 14 cabang match, beberapa fallthrough |
| `parse_expr` (Pratt) | O(1) per token | O(1) per token | Sudah optimal — single pass, no backtrack |
| `skip_to_next_top_level` | O(k) | O(k) | Linear scan ke keyword berikutnya |

---

## 3. AUDIT TOKEN

### Temuan Utama: Keyword String Allocation Wasted
`parser/lexer.rs:578-765` — `read_ident_or_keyword()` membangun `String s` char-by-char, lalu match keyword. Untuk **semua keyword token** (yang merupakan mayoritas token dalam SV), String ini dibangun lalu **dibuang**.

```
// Setiap keyword token:
String::new()            → alloc (line ~580)
push(char) * N           → grow (lines 590, 598)
// match keyword:
Token::Module            → String s DROP (no ownership transfer)
```

Estimasi: ~70% token adalah keyword. Setiap keyword = 1 String alloc + dealloc sia-sia.

### Token Copy
- Token enum: 0-24 bytes, `Clone` + `Copy` pada sebagian besar variant
- Tidak ada `Vec<Token>.clone()` — token stream dipindah (move) ke Parser
- `Symbol` adalah `u32` (Copy), cheap
- Satu-satunya heap variant: `Token::Error(String)` — jarang

### Total Token Traversal
- 1x oleh lexer (streaming, no Vec)
- 2x oleh parser (2 passes)
- 0x traversal tidak diperlukan selain pass 2

---

## 4. AUDIT AST

### AST Dibangun 2x (untuk packages)
- Package AST dibangun penuh di pass 1 (dibuang) dan pass 2 (dipakai)

### Full AST Clone (Besar!)
| Lokasi | Baris | Ukuran | Dampak |
|--------|-------|--------|--------|
| `mod.rs:383` | `self.design.modules.clone()` | Full Module AST | Snapshot untuk checksum — butuh hanya `name + items` |
| `flatten.rs:23` | `m.clone()` per instance | Full Module AST | Clone per N instances — bisa sharing |
| `mod.rs:1758` | `other.clone()` per item | ModuleItem | Clone semua non-generate item |
| `mod.rs:806` | `self.modules.clone()` | HashMap<Symbol, IrModule> | Clone semua IR modules ke IrDesign |
| `mod.rs:418` | `cached_ir.clone()` | IrModule | Clone dari cache — seharusnya move/clone-on-write |
| `mod.rs:422` | `ir.clone()` | IrModule | Clone ke cache |

### AST Vec Besar
- `Design.modules: Vec<Module>` — semua modules
- `Module.items: Vec<ModuleItem>` — semua items per module
- `Module.decls: Vec<Decl>` — semua deklarasi
- `ClassDecl.methods: Vec<MethodDecl>` — methods
- `FunctionDecl.stmts: Vec<Stmt>` — body statements

Semua Vec ini di-clone berulang kali.

---

## 5. AUDIT SYMBOL COLLECTION

### Fast Scan (parse_module_fast / parse_class_fast)
Sudah ada: `parser/instance.rs:161-223` (fast), `parser/class.rs:17-82` (fast).

Fast scan hanya membaca nama module/class — tidak parse body penuh. **Ini sudah benar.**

### Masih Ada:
- **Packages tidak punya fast scan** — diparse penuh 2x
- **Interfaces tidak punya fast scan** — `parse_interface_fast` (instance.rs:224) skip body tapi `parse_interface` di pass 2 parse penuh
- **Programs tidak punya fast scan** — sama dengan interface

### Rekomendasi: Fast scan untuk packages
Package body tidak perlu diparse penuh di pass 1. Cukup baca nama package dan export.

---

## 6. AUDIT MEMORY

### Clone Count by Directory
| Direktori | Clone | Main Source |
|-----------|-------|-------------|
| `elaboration/` | 237 | mod.rs (125), expr.rs (80), flatten.rs (72) |
| `parser/` | 107 | decl.rs (23), instance.rs (15), proc.rs (13) |
| `preprocessor/` | 3 | Minimal |
| `lexer/` | 1 | path.clone() |

### String Allocation Hotspots
| Location | Count | Pattern |
|----------|-------|---------|
| Parser keyword tokens | ~70% of tokens | String built + dropped |
| Elaborator format!() | 66 calls | format!("initial_{}", n) per block |
| Preprocessor to_string() | 17+ calls | trim().to_string() redundant |
| Preprocessor replace() | Per parameter | String::replace() alloc baru tiap iter |

### Arena Allocation
**Parser tidak menggunakan arena sama sekali.** Semua AST node via `Box::new()` (~150+ calls). 

`BumpArena`, `TypedArena` sudah ada di `src/arena/` tapi hanya dipakai di `SimulationArena` (simulator), bukan parser.

Perbandingan:
- `Box::new(Expr::Binary { ... })` = heap alloc per node
- `arena.alloc(Expr::Binary { ... })` = bump alloc, batch free

Untuk design 10K statements, parser melakukan ~30K+ heap allocations via Box.

---

## 7. AUDIT IO

### Duplicate File Reads (Production Code)
| File | Baris | Read 1 | Read 2 | Problem |
|------|-------|--------|--------|---------|
| `compile_session.rs` | 175, 317 | `MmapFile::open` | `std::fs::read` | Checksum bisa dari mmap |
| `compile_session.rs` | 175, 357 | `MmapFile::open` | `std::fs::read` | detect_changed baca ulang |
| `compile_session.rs` | 175, 398 | `MmapFile::open` | `std::fs::read` | build_index baca ulang |

### File Read Count
- `io.rs`: 10 reads (5 path, 5 test)
- `compile_session.rs`: 5 reads (3 duplicate dengan mmap)
- `discovery.rs`: 2 reads
- `checksum.rs`: 1 read
- `lib.rs`: 2 reads

### Metadata / Canonicalize
- `fs::metadata`: 3 calls (io.rs:32, io.rs:80, discovery.rs:99)
- `canonicalize`: 1 call (main.rs:142)
- `walkdir`: 1 call (discovery.rs:60)
- `read_dir`: 3 calls (main.rs:156, 180, 304)

### Path Operations
- `frontend/`: ~57 PathBuf/Path ops
- `main.rs`: ~30 ops

---

## 8. AUDIT PARALLELISM

### Rayon Usage (7 par_iter total)
| Location | Line | Work Unit Size |
|----------|------|---------------|
| `discovery.rs:97` | `.par_iter()` on FileEntry | Checksum per file |
| `compile_session.rs:165` | `.par_iter()` on files | Parse per file — **task terlalu besar?** |
| `main.rs:211` | `.par_iter()` on preprocess | Preprocess per file |
| `scheduler/sim_dag.rs:542, 588` | `.par_iter()` on DAG levels | Parallel sim |
| `simulator/parallel.rs:519` | `.par_iter()` on signals | Parallel eval |
| `engine/scheduler/event.rs:220` | `.par_iter()` on events | Event processing |

### Mutex / Lock Contention
| Lock | Location | Contention |
|------|----------|-----------|
| `parking_lot::Mutex<Vec<&'static str>>` | `intern/string_intern.rs:183` | **HIGH** — semua thread serialized di `as_str()` |
| `Mutex<HashMap>` | `scheduler/dag.rs:8` | Medium — thread contention di DAG |
| `Mutex<HashMap>` | `mir/jit.rs:17` | Low — cold path |
| `Mutex<Vec<TraceEvent>>` | `profiling/trace.rs:3` | Low — profiling only |
| `DashMap` (sharded) | `intern/string_intern.rs:179` | Low — sharded, mostly read |

### Key Finding: Tidak ada RwLock di seluruh codebase
Semua lock adalah Mutex. Beberapa lokasi (seperti `string_intern.rs`) bisa pakai RwLock untuk read-heavy patterns.

### thread::spawn
- **1 production spawn**: co-simulation server (cosim/mod.rs:88)
- 2 test-only spawns

---

## 9. AUDIT LOGGING

### format!() yang tetap dieksekusi walau log mati
**Tidak ada conditional logging guard** (seperti `log_enabled!()`). Semua `format!()` di error/warning path tetap alokasi String.

### Hot path format!() yang bermasalah
| Location | Line | Pattern | Dampak |
|----------|------|---------|--------|
| `elaborator/mod.rs:828` | `format!("{:?}", module)` | Debug-format entire module untuk checksum — **SANGAT MAHAL** |
| `elaborator/stmt.rs:88` | per assignment | width mismatch message — alloc String per assign |
| `elaborator/mod.rs:1782-1809` | per process | `format!("initial_{}", n)` — alloc per initial/final/assign block |

### println! / eprintln! di parser
- `parser/mod.rs`: 0
- `parser/preprocessor.rs`: 0 (pakai format! + push warning)
- `parser/lexer.rs`: 0
- Seluruh parser tidak menggunakan println!/eprintln! langsung

---

## 10. AUDIT DATA STRUCTURE

### Vec Digunakan Sebagai Set
| Location | Line | Problem | Fix |
|----------|------|---------|-----|
| `preprocessor.rs:142` | `include_stack.contains()` | O(d) linear scan | HashSet |
| `elaborator/mod.rs:887` | `order.contains(&module.name)` | O(M) per check, O(M²) worst | HashSet |

### HashMap Overuse
- `elaborator/mod.rs:40` — `HashMap<Symbol, HashMap<Symbol, PackageItem>>` nested HashMap
- Banyak HashMap dibuat ulang (27 `HashMap::new()` di elaborator)

### Vec<Vec<>> Patterns
- `IrStmt::Fork { processes: Vec<Vec<IrStmt>> }` — fork/join branches
- `RandSequence.productions: Vec<(Symbol, Vec<(IrExpr, Vec<IrStmt>)>)>` — triple nested

### String Interner
- `DashMap<String, u32>` — bagus (sharded, fast path)
- `Mutex<Vec<&'static str>>` — bottleneck (semua `as_str()` serialized)

---

## 11. AUDIT PROJECT STARTUP

### Startup Sequence (main.rs run())
```
Step 1: Rayon thread pool config              ~1ms
Step 2: CLI args parse                         ~2ms  
Step 3: Read project file / source list        ~1ms
Step 4: Auto-detect include paths (walk)       ~5-50ms (disk-bound)
Step 5: Preprocess (parallel)                  ~50-500ms (I/O + CPU)
Step 6: Lex tokens                             ~20-200ms (CPU)
Step 7: Parse design (2 pass)                  ~50-500ms (CPU)
Step 8: Library scan (per file: read+pre+lex+parse) ~100ms-5s (I/O + CPU)
Step 9: Elaborate                              ~50-500ms (CPU heavy)
───────────────────────────────────────────────────────
Total startup: ~300ms - 7s (tanpa sim)
```

### Estimasi scaling
| File count | Preprocess | Lex | Parse | Elaborate | Total |
|-----------|-----------|-----|-------|-----------|-------|
| 10 | 50ms | 20ms | 50ms | 50ms | 170ms |
| 100 | 500ms | 200ms | 500ms | 500ms | 1.7s |
| 1,000 | 5s | 2s | 5s | 5s | 17s |
| 10,000 | 50s | 20s | 50s | 50s | 170s |

Bottleneck utama di 1K+ files: **Parse 2-pass** (O(2N)), **Elaborator clone** (O(N²) dengan clone berantai), **Preprocessor replace()** (O(n²) untuk macro kompleks).

---

## 12. AUDIT CACHE

### Cache Infrastructure (src/cache/)
| Cache | File | Status |
|-------|------|--------|
| AST Cache | `ast_cache.rs` | Defined tapi **tidak dipakai di production path** |
| HIR Cache | `hir_cache.rs` | Defined, tidak dipakai |
| Dep Cache | `dep_cache.rs` | Defined, tidak dipakai |
| Cache Manager | `cache_manager.rs` | Defined, tidak dipakai |
| Remote | `remote.rs` | Defined, tidak dipakai |

**Semua cache infrastructure ada tapi tidak diintegrasikan.** Tidak ada cache hit/miss di pipeline utama.

### Elaborator Cache (manual)
`elaborator/mod.rs:396` — `self.module_cache: HashMap<(String, String), IrModule>` — cache manual untuk IR module hasil elaborasi.

`module_cache` dipakai di `elaborator/mod.rs:420-424` — check cache sebelum re-elaborate module yang sama dengan parameter sama.

Ini **sudah benar** tapi masih ada clone berlebihan (line 418, 422: `cached_ir.clone()`).

### Yang Bisa Di-cache (tapi belum)
- **Token stream** — bisa di-cache per file (hanya 1x lex per file saat ini — OK)
- **AST** — bisa di-cache per file (AST saat ini dibangun 2x untuk packages)
- **Preprocessor output** — bisa di-cache per file (preprocess 1x per file saat ini — OK)

---

## 13. AUDIT DUPLICATE WORK

### Item | Lokasi | Duplikasi | Dampak
1. **Packages diparse 2x** | `parser/mod.rs:312` + `483` | Full AST build di pass 1, dibuang | 2x waktu package parsing
2. **File dibaca 2x** | `compile_session.rs:175` + `317/357/398` | mmap + std::fs::read | 2x I/O per file
3. **Modules list di-iterasi 8x** | `elaborator/mod.rs:194-796` | 8 iterasi terpisah | 8x traversal modules
4. **Module items di-iterasi 5x** | `elaborator/mod.rs:1121-2103` | 5 pass filter by variant | 5x traversal items
5. **signals.find() 5x per var** | `elaborator/mod.rs:1580-1710` | Linear scan 5x | 5x O(N) per variable
6. **DPI scan 3x** | `stmt.rs:393, 777, 787` | Full O(M*I) scan | 3x untuk $display yang sama
7. **Module AST clone Nx** | `flatten.rs:23` | Clone per instance | Nx AST clone untuk module yang sama
8. **trim() multiple** | `preprocessor.rs:87, 91, 101-114` | 2-3x trim data sama | Wasted slice alloc
9. **cond_stack.all() tiap baris** | `preprocessor.rs:104` | Iterasi stack tiap baris | O(L*K) wasted

---

## 14. AUDIT KOMPLEKSITAS

### Fungsi Parser Utama

| Function | File:Line | Teoritis | Aktual | Iterasi | Alloc | Clone |
|----------|-----------|----------|--------|---------|-------|-------|
| `parse_design` | mod.rs:239 | O(N) | O(2N + N·skip) | 2 passes | 13 Vecs | 0 |
| `parse_module_item_body` | mod.rs:735 | O(1) | O(1) avg | 1 per item | ~3 per item | ~1 |
| `parse_module` | instance.rs:14 | O(N) | O(N) | 1 per item | 4 Vecs | 0 |
| `parse_class` | class.rs:84 | O(N) | O(N) | 1 per field | 5 Vecs | ~2 |
| `parse_package_decl` | package.rs:16 | O(N) | O(N) | 1 per item | 1 Vec | ~9 |
| `parse_expr` | expr.rs:99 | O(1) | O(1) | 1 per token | ~8 total | ~5 |
| `parse_decl_names` | decl.rs:252 | O(n) | O(n) | 1 per var | ~10 total | ~23 |
| `parse_function` | proc.rs:159 | O(N) | O(N) | 1 per stmt | ~16 total | ~13 |
| `skip_to_next_top_level` | mod.rs:1211 | O(k) | O(k) | hingga keyword | 0 | 0 |

### Preprocessor

| Function | File:Line | Teoritis | Aktual | Iterasi | Alloc | Clone |
|----------|-----------|----------|--------|---------|-------|-------|
| `preprocess` | preprocessor.rs:74 | O(L) | O(L) | L baris | 2 Vecs + strings | 1 |
| `expand_inline_macros_depth` | preprocessor.rs:455 | O(M) | O(M·P·R) | M chars | ~5+ per macro | 1 |
| `split_macro_args` | preprocessor.rs:531 | O(A) | O(A) | A args | A Strings | 0 |
| `eval_ifdef_expr` | preprocessor.rs:422 | O(K) | O(K) | K lookups | ~2 | 0 |

Keterangan: L=baris, M=char, P=parameter, R=recursion depth, A=arg count, K=cond stack depth

---

## 15. AUDIT HOTSPOT — TOP 30

| Rank | Function | File | Line | Est. % | Penyebab | Solusi |
|------|----------|------|------|--------|-----------|--------|
| 1 | `elaborate_module_with_params_and_type` | elaborator/mod.rs | 1005 | 25% | 5-pass items, clone berat, O(N²) signal search | 1-pass items, HashMap signal lookup, arena |
| 2 | `parse_design` | parser/mod.rs | 239 | 15% | 2-pass token stream, skip recovery | Eliminate pass 1, lazy class discovery |
| 3 | `expand_inline_macros_depth` | preprocessor.rs | 455 | 10% | replace() O(n²) per param, Vec<char> alloc | replace_all() 1-pass, iter chars langsung |
| 4 | `flatten_module` | flatten.rs | 17 | 8% | Clone full AST per instance, linear module find | Cache clone, HashMap lookup |
| 5 | `substitute_ident_in_expr` | expr.rs | 929 | 5% | Clone tree tiap recursive descent | Mutate in-place atau copy-on-write |
| 6 | `elaborate` (top-level) | elaborator/mod.rs | 165 | 5% | 8x module list iteration | Combine loops, single pass |
| 7 | `read_ident_or_keyword` | lexer.rs | 578 | 4% | String build + drop untuk keyword | Match chars langsung tanpa String |
| 8 | `parse_module_item_body` | parser/mod.rs | 735 | 4% | 14-branch switch, Ok(None) fallthrough | Inline common branches |
| 9 | `parse_module` | instance.rs | 14 | 3% | Loop + parse_module_item | Minimal |
| 10 | `elaborate_stmt` | stmt.rs | 1-1000 | 3% | O(M*I) DPI scan 3x | Cache DPI imports |
| 11 | `elaborate_expr` | expr.rs | 1-1263 | 3% | Clone tree, format!() | Arena alloc, lazy format |
| 12 | `compute_checksum` | elaborator/mod.rs | 828 | 2% | format!("{:?}", module) debug | Hash selective fields |
| 13 | `parser/parse_expr` | expr.rs | 99 | 2% | OK — Pratt sudah optimal | — |
| 14 | `Symbol::as_str` | intern/string_intern.rs | 322 | 2% | Mutex global serializes | RwLock atau lock-free |
| 15 | `parse_decl_names` | decl.rs | 252 | 2% | 23 clone calls | Arena, reference |
| 16 | `parse_function` | proc.rs | 159 | 2% | 13 clones, 16 Vec allocs | Arena |
| 17 | `signal_analysis` | util/signal_analysis.rs | 1-315 | 2% | Linear scan signals | HashMap |
| 18 | `compute_topo_order` | elaborator/mod.rs | 887 | 1.5% | order.contains() O(M) | HashSet |
| 19 | `parse_package_decl` | package.rs | 16 | 1.5% | Diparse 2x | Fast scan pass 1 |
| 20 | `preprocess` (main loop) | preprocessor.rs | 84 | 1.5% | O(L·K) cond_stack.all() | Cache emit status |
| 21 | `parse_class` | class.rs | 84 | 1% | 5 Vecs, class body | Arena |
| 22 | `parse_instance` | instance.rs | 675 | 1% | Port/param HashMap | Arena |
| 23 | `parse_gate_primitive` | instance.rs | 824 | 1% | Drive strength backtrack | Minimal |
| 24 | `skip_to_next_top_level` | parser/mod.rs | 1211 | 1% | Called ~12x from parse_design | Reduce calls |
| 25 | `type_resolution` | elaborator/mod.rs | 1197-1314 | 1% | Clone typedefs | Reference |
| 26 | `param_substitution` | elaborator/util/type_subst.rs | 1-143 | 1% | Clone expr tree | In-place |
| 27 | `split_macro_args` | preprocessor.rs | 531 | 0.5% | A to_string() calls | Slice &str |
| 28 | `parse_udp_declaration` | udp.rs | 166 | 0.5% | 3 Vecs | Arena |
| 29 | `parse_config_decl` | config.rs | 16 | 0.5% | 8 clones | Reference |
| 30 | `parse_specify_item` | specify.rs | 184 | 0.5% | 10 clones, backtrack | Arena |

---

## 16. AUDIT PARSER HANG

### Loop dengan exit condition
Semua `while` dan `loop` sudah diperiksa. Tidak ada infinite loop yang tidak tertangani.

### Stuck Detection
| Loop | Baris | Limit | Action |
|------|-------|-------|--------|
| pass 1 main loop | mod.rs:279 | 1,000,000 | `bail!("parser stuck")` |
| pass 2 main loop | mod.rs:449 | 1,000,000 | `bail!("parser stuck")` |
| module body loop | instance.rs:84 | 1,000,000 | `bail!("parser stuck")` |
| port list loop | instance.rs:366 | 1,000,000 | `bail!("parser stuck")` |
| skip_class_body | mod.rs:1289 | 500,000 | `bail!("parser stuck...skip_class_body")` |
| skip_to_next_top_level | mod.rs:1217 | 500,000 | `bail!("stuck skipping to next top level")` |
| skip_until_semi_or_end | mod.rs:1256 | 500,000 | `bail!("stuck skip_until_semi_or_end")` |
| skip_balanced_paren_light | mod.rs:1364 | 500,000 | `bail!("stuck skip_balanced_paren_light")` |
| skip_attribute | mod.rs:1408 | 500,000 | `bail!("stuck in skip_attribute")` |

**Semua loop punya stuck detection.** Tidak ada risiko hang permanen.

### Mutual Recursion Depth
- Parser: max ~10-15 calls (parse_module → parse_module_item → parse_module_item_body → parse_decl → ...)
- Expression: max ~50-100 calls (Pratt parser recursive, tapi bounded oleh token count)
- Preprocessor: max 64 (recursion guard di expand_inline_macros_depth)

---

## 17. AUDIT SKALABILITAS

### Scalability Analysis

| Scale | Bottleneck #1 | Bottleneck #2 | Bottleneck #3 | Feasible? |
|-------|--------------|--------------|--------------|-----------|
| **100 files** | Parse 2-pass (~500ms) | Elaborator clones (~500ms) | None | **Yes** |
| **500 files** | Parse 2-pass (~2.5s) | Elaborator O(N²) clones (~5s) | Preprocessor replace() | **With issues** |
| **1,000 files** | Parse 2-pass (~5s) | Elaborator O(N²) (~20s) | Memory (clones) | **Slow** |
| **5,000 files** | Parse 2-pass (~25s) | Elaborator O(N²) (~5min) | Memory (~5GB+) | **No** |
| **10,000 files** | Elaborator O(N²) (~20min) | Parse 2-pass (~50s) | Memory (~20GB+) | **No** |
| **50,000 files** | Elaborator O(N²) (~8hrs) | Parser O(2N) (~4min) | Startup I/O | **Infeasible** |
| **100M tokens** | Lexer O(N) string alloc (~30s) | Parse O(2N) (~60s) | Memory (~8GB) | **With issues** |

### Breaking Points
1. **Elaborator O(N²) di 1K+ modules**: `order.contains()` + `signals.iter().find()` + 5x items pass + 8x module pass
2. **Clone memory di 5K+ modules**: Setiap module di-clone ~3-5x → memory multiplier 3-5x
3. **Parse 2-pass** di 50K files: 2x token traversal = 2x waktu parsing, tidak perlu

---

## 18. HASIL AUDIT — Semua Temuan

| ID | Severity | File | Function | Line | Bukti | Dampak | Penyebab | Estimasi Improvement |
|----|----------|------|----------|------|-------|--------|----------|-------------------|
| **C001** | **CRITICAL** | `parser/mod.rs` | `parse_design` | 312, 436, 483 | `packages.push(parse_package_decl())` then `packages.clear()` then parse again | 2x parsing semua packages | 2-pass arsitektur tanpa cache | **50%** waktu package parsing |
| **C002** | **CRITICAL** | `elaborator/mod.rs` | `elaborate_module_with_params_and_type` | 1580-1710 | `signals.iter_mut().find()` 5x per variable | O(5·N·V) linear scan | HashMap lookup tidak dipakai | **5x** lebih cepat signal matching |
| **C003** | **CRITICAL** | `flatten.rs` | `flatten_module` | 23 | `m.clone()` per instance — full AST clone | Nx clone per module | Tidak sharing AST antar instance | **Nx** memory saving |
| **C004** | **HIGH** | `elaborator/mod.rs` | `elaborate` | 194-796 | 8 iterasi terpisah `for module in &self.design.modules` | 8x traversal modules list | Tidak combine loops | **8x** lebih cepat |
| **C005** | **HIGH** | `elaborator/mod.rs` | `elaborate_module_with_params_and_type` | 1121-2103 | 5 iterasi `for item in &module.items` | 5x traversal items | Filter by variant tiap pass | **5x** lebih cepat |
| **C006** | **HIGH** | `parser/lexer.rs` | `read_ident_or_keyword` | 578-765 | String built char-by-char lalu dropped untuk keyword | ~70% token alokasi wasted | Tidak match chars langsung | **~40%** lexer faster |
| **C007** | **HIGH** | `preprocessor.rs` | `expand_inline_macros_depth` | 518 | `expanded = expanded.replace(param, arg)` per parameter | O(n·m·p) alloc per expand | replace() 1-pass bisa | **Px** (parameter count) |
| **C008** | **HIGH** | `intern/string_intern.rs` | `as_str` | 322 | `self.strings.lock()` — Mutex global | Semua thread serialized | RwLock atau lock-free index | **Ncore x** throughput |
| **C009** | **HIGH** | `compile_session.rs` | `compile` | 175, 317 | `MmapFile::open` + `std::fs::read` path sama | 2x I/O per file | Checksum dari mmap bytes | **50%** I/O per file |
| **C010** | **HIGH** | `elaborator/mod.rs` | `compute_checksum` | 828 | `format!("{:?}", module)` — Debug entire AST | Format full module ke String | Hash selective fields | **10-100x** faster checksum |
| **M001** | **HIGH** | `parser/` (all) | All parse functions | Many | ~150+ `Box::new()` — heap alloc per AST node | ~30K+ allocs per design | Arena alloc tersedia tapi tidak dipakai | **~90%** alloc overhead |
| **H001** | **MEDIUM** | `elaborator/expr.rs` | `substitute_ident_in_expr` | 929-1133 | Clone tree 30+ calls per descent | Deep copy tiap recursive call | Mutate in-place | **10x** |
| **H002** | **MEDIUM** | `elaborator/stmt.rs` | `elaborate_stmt` | 393, 777, 787 | DPI scan 3x: `modules.iter().flat_map().any()` | O(3·M·I) | Cache DPI import list | **3x** |
| **H003** | **MEDIUM** | `elaborator/mod.rs` | `compute_topo_order` | 887 | `order.contains(&module.name)` | O(M²) worst | HashSet | **O(1)** instead of O(M) |
| **H004** | **MEDIUM** | `preprocessor.rs` | `preprocess` | 142 | `include_stack.contains()` linear scan | O(d) per include | HashSet | **O(1)** |
| **H005** | **MEDIUM** | `preprocessor.rs` | `preprocess` | 104 | `is_emitting()` iterasi cond_stack tiap baris | O(L·K) | Cache emit status boolean | **~50%** overhead conditional |
| **H006** | **MEDIUM** | `preprocessor.rs` | `expand_inline_macros_depth` | 464 | `Vec<char>` alloc per call — even for no-macro lines | M alloc per baris | `.chars()` iterator langsung | **~50%** alloc |
| **H007** | **MEDIUM** | `parser/mod.rs` | `parse_module_item_body` | 1202 | `_ => Ok(None)` fallback tanpa advance | Guard di caller perlu cek pos | Lebih baik advance atau return error | Minor |
| **L001** | **LOW** | `preprocessor.rs` | `preprocess` | 87 | `raw_line.trim_end()` dipanggil 2x | 2x slice alloc | Cache hasil trim | Minor |
| **L002** | **LOW** | `preprocessor.rs` | Several | 91, 101 | `trim().to_string()` redundant | Extra alloc | Slice langsung dari &str | Minor |
| **L003** | **LOW** | `elaborator/mod.rs` | `elaborate` | 1809 | `format!("initial_{}", n)` per process | Alloc per initial block | Pre-alloc atau incremental | Minor |

---

## 19. PRIORITAS PERBAIKAN

### Tier 1: High ROI (effort kecil, dampak besar)

| # | Perbaikan | File | Dampak | Effort |
|---|-----------|------|--------|--------|
| 1 | **Hapus duplicate package parse** — skip package di pass 1, parse only pass 2 | `parser/mod.rs:312-436` | **~50%** waktu package | 1 jam |
| 2 | **HashMap signal lookup** — ganti `signals.iter().find()` dengan `HashMap<Symbol, SignalId>` | `elaborator/mod.rs:1580-1710` | **5x** signal matching | 2 jam |
| 3 | **Checksum from mmap** — compute hash from already-mapped bytes | `compile_session.rs:317` | **~50%** file I/O | 30 menit |
| 4 | **RwLock string interner** — ganti Mutex dengan RwLock untuk read-heavy | `intern/string_intern.rs:183` | **Ncore x** throughput | 1 jam |
| 5 | **HashSet untuk order.contains()** | `elaborator/mod.rs:887` | **O(1)** topo check | 15 menit |
| 6 | **HashSet untuk include_stack.contains()** | `preprocessor.rs:142` | **O(1)** include check | 15 menit |

### Tier 2: Medium ROI (effort sedang, dampak signifikan)

| # | Perbaikan | File | Dampak | Effort |
|---|-----------|------|--------|--------|
| 7 | **Combine loops** — 8 module passes → 1 pass | `elaborator/mod.rs:194-796` | **8x** module traversal | 4 jam |
| 8 | **Combine loops** — 5 item passes → 1 pass | `elaborator/mod.rs:1121-2103` | **5x** items traversal | 4 jam |
| 9 | **Arena alloc untuk parser** — ganti Box::new() dengan BumpArena | `parser/` | **~90%** alloc overhead | 8 jam |
| 10 | **replace_all() 1-pass** — ganti replace() loop per parameter | `preprocessor.rs:518` | **Px** macro expansion | 2 jam |
| 11 | **Match keywords without String** — match chars langsung | `parser/lexer.rs:578-765` | **~40%** lexer | 3 jam |
| 12 | **Cache emit status** — hindari iterasi cond_stack tiap baris | `preprocessor.rs:104` | **~50%** conditional overhead | 1 jam |

### Tier 3: Architectural (effort besar, dampak fundamental)

| # | Perbaikan | File | Dampak | Effort |
|---|-----------|------|--------|--------|
| 13 | **Eliminate 2-pass design** — lazy class discovery via symbol table | `parser/mod.rs:239` | **~50%** parse time | 2-3 hari |
| 14 | **Share AST antar instance** — clone-on-write atau reference counting | `flatten.rs:23` | **Nx** memory | 1-2 hari |
| 15 | **Selective checksum hash** — bukan Debug format | `elaborator/mod.rs:828` | **10-100x** checksum | 2 jam |
| 16 | **Cache implementation** — integrasikan ast_cache/hir_cache ke pipeline | `cache/` + pipeline | **10-50x** incremental | 3-5 hari |

### Prioritized Action Plan

**Sprint 1** (Tier 1 — 5 jam total):
1. Hapus duplicate package parse
2. HashMap signal lookup
3. Checksum from mmap
4. RwLock string interner
5. HashSet untuk linear scans

**Sprint 2** (Tier 2 — 22 jam total):
6. Combine loops (module + items)
7. Arena alloc parser
8. Macro expansion 1-pass
9. Keyword lex optimization

**Sprint 3** (Tier 3 — 40+ jam total):
10. Eliminate 2-pass
11. Share AST
12. Cache integration

---

## CATATAN: HIPOTESIS (perlu verifikasi)

| Hipotesis | Data Tambahan Diperlukan | 
|-----------|-------------------------|
| `parse_module_fast` masih bisa optimize lebih — mungkin skip terlalu cautious | Flamegraph per component, measure % time in fast vs full pass |
| `Design.modules` Vec besar cause cache miss di iterasi 8x | CPU cache miss counter (perf stat) |
| Parser stuck detection (1M limit) never triggers in practice | Count how many times stuck_count increments in normal runs |
| `Symbol::as_str()` Mutex contention signifikan di 16+ threads | Count lock contention via perf lock atau hot thread traces |
| Arena allocator overhead lebih kecil dari heap allocator untuk AST nodes | Benchmark: BumpArena vs Box::new() for 10K Expr nodes |
| `Vec<char>` in preprocessor (line 464) — apakah `.chars()` iterator lebih cepat? | Microbenchmark: chars().collect::<Vec<char>>() vs chars() iterator |