# maria — RTL Simulator untuk SystemVerilog

## Aturan 
1 file = 1 tanggung jawab tidak boleh lebih dari 1
Rust-based SystemVerilog simulator. Pipeline: preprocessor → lexer → parser → AST → elaborator → IR → simulation engine → VCD output.

### 🚫 LARANGAN TOTAL: SCRIPT UNTUK MEMODIFIKASI PROJECT
**DILARANG TOTAL, TANPA TOLERANSI** — dilarang menggunakan script apa pun (Python, Perl, Ruby, Bash/sed/awk mass-edit, `python -c` one-liner, dan sejenisnya) untuk mengubah/memodifikasi file di project ini.

**Berlaku untuk SEMUA agent dan subagent yang bekerja di project ini, tanpa terkecuali.**

Aturan:
1. **Semua perubahan file WAJIB manual** via editor (str_replace / write_file), satu per satu, dengan konteks yang diverifikasi.
2. **Tidak boleh menulis script** (apa pun bahasanya) untuk melakukan mass-edit, batch-replace, atau transformasi kode otomatis.
3. **Tidak boleh menjalankan script** yang menulis ke file project (mis. `cargo fix`, `cargo clippy --fix`, `rustfmt`, formatter otomatis massal, sed/awk `-i`, python write). Tidak ada pengecualian dari agent/subagent mana pun. Satu-satunya jalur penggunaan script adalah jika user memerintahkannya secara eksplisit dan tertulis di percakapan; jika instruksinya ambigu, konfirmasi dulu sebelum eksekusi. Script temporer di `/tmp` yang hasilnya disalin ke project juga termasuk pelanggaran.
4. Alasan: script mass-edit merusak tipe/borrow-check (mis. menghapus `.clone()` pada `Symbol`/Copy yang butuh deref, menambahkan `*` di receiver yang salah) dan menimbulkan 100+ error build yang butuh waktu lama dipulihkan.
5. Boleh digunakan untuk **membaca/menganalisis** (grep, awk print, sed read-only) — tidak boleh untuk **menulis**.

## Build & Test

```shell
cargo build
cargo test                    # all unit tests (in src/lib.rs, in-module tests)
cargo test --lib              # same, excludes main.rs
cargo test <test_name>        # single test (no --lib needed if unique)
```

No CI, no lint, no typecheck shortcuts. Just `cargo test`. 1247 tests pass (+16 ignored).

## CLI Tools (`src/tools/`, subcommand `maria <tool>`)

10 tool terminal dari tools.md — satu file per tool (aturan 1 file = 1 tanggung jawab):

| Tool | File | Fungsi |
|------|------|--------|
| `minspect` | `inspect.rs` | X-ray project: `stats`, `modules`, `hierarchy`, `packages`, `classes`, `interfaces`, `parameters`, `deps`. Subcommand boleh di posisi pertama (`minspect stats rtl/`) |
| `mlint` | `lint.rs` | Static linter: unused signal, width mismatch, latch, combinational loop, FSM |
| `melab` | `elab.rs` | Elaborasi saja: hierarchy tree, param, signal top |
| `msim` | `sim.rs` | Simulasi: VCD (+FST), ringkasan assertion/coverage |
| `mcov` | `cov.rs` | Coverage → `coverage.json` + `coverage.html` (via CoverageDatabase) |
| `mwave` | `wave.rs` | Utility VCD: `merge` (offset kumulatif), `export` (csv/txt), `filter` (subset sinyal) |
| `mfmt` | `fmt.rs` | Formatter SV/Verilog berbasis lexer (stdout/--inplace/--check) |
| `mprof` | `prof.rs` | Profiler pipeline: timing per fase + bottleneck + hint |
| `mcheck` | `check.rs` | Health check: missing `include, circular include, unresolved deps, cycle module, timescale |
| `mbench` | `bench.rs` | Benchmark: compile speed, throughput, peak RSS (VmHWM), cache hit |

Shared infra di `src/tools/mod.rs`:
- `collect_targets()` — expand file/direktori/file list
- `open_project()` — CompileSession → merged `Design` (parse saja, cepat, MICD cache)
- `open_elaborated()` — + elaborasi penuh → `IrDesign` (dipakai melab/msim/mcov/mprof)
- `expr_to_string()`, `section()`, `kv()`, `human_bytes()` — output konsisten

Cara kerja: semua tool memakai `CompileSession` (parallel parse + MICD), bukan pipeline legacy. Subcommand di-dispatch di `main.rs` via `dispatch_*()` + `exit_tool()`. CLI args di `src/cli.rs` (`MariaCmd` enum + struct args per tool).

Catatan formatter (`mfmt`): token-based, 1 file = 1 tanggung jawab, tidak pakai `Token::Display` untuk operator (Debug output `Plus` bukan `+`) — pakai `token_text()` manual.

## Pipeline architecture

1. **`src/main.rs`** — CLI entrypoint. Reads `.sv` file(s), concatenates, feeds through lexer → parser → elaborator → engine.
2. **`src/lib.rs`** — Library entrypoint. Exposes `compile_str()`, `simulate_str()`, `simulate_signals()`. Tests live in `src/tests/mod.rs`.
3. **`src/parser/`** — `lexer.rs` (tokenizer), `parser.rs` (Pratt-style top-down operator precedence), `preprocessor.rs` (`` `ifdef ``/`define`).
4. **`src/ast/`** — `expr.rs`, `stmt.rs`, `types.rs`, `const_eval.rs`, `inline.rs` (function inlining for `loop_unroll` and `substitute_loop_var`).
5. **`src/elaboration/elaborator.rs`** — AST → IR, signal collection, type resolution, loop unrolling, constant folding for `$clog2`/`$bits`/`$size`/`$left`/`$right`/`$low`/`$high`.
6. **`src/ir/ir.rs`** — IR types (`IrStmt`, `IrExpr`, `LogicVec`).
7. **`src/simulator/`** — `engine.rs` (event-driven scheduler), `types.rs` (debug/event/UVM types), `state.rs` (signal storage), `value.rs` (`eval_binary`, `eval_unary`), `sdf.rs` (SDF annotation), `jit.rs` (JIT stubs), `parallel.rs` (parallel eval), `util.rs`.
8. **`src/waveform/`** — `vcd.rs` (VCD dump), `fst.rs` (FST waveform via wavefst crate).
9. **`src/debugger/mod.rs`** — `Debugger` struct wrapping `SimulationEngine`. Step, breakpoint, watchpoint, timeline, hierarchy tree, reverse debug, memory inspect. 21 unit tests inline.
10. **`uvm_macros.svh`** — UVM macro definitions (info/warning/error/fatal, factory utils).

## Enterprise Context Architecture (`src/env/`)

Desain 5 doc/env.md: `GlobalEnv` root object menampung 12 context (masing-masing `Arc`), bukan satu Env raksasa. Dependency satu arah: `Config → Workspace → Runtime → Compiler → Cache/Database/Diagnostics/Telemetry → Verification → Simulation`.

- `src/env/global/` — `GlobalEnv` (aggregator + accessor), `startup.rs` (lifecycle `startup()`/`startup_with()`/`for_cli()`), `shutdown.rs`, `build.rs`/`version.rs`.
- `src/env/config/` — `ConfigContext` (wrap `MariaConfig`), `loader.rs` (TOML/JSON), `validator.rs`, `cli.rs` (`EnvCliOptions` — CLI menang), `environment.rs` (`MARIA_*`).
- `src/env/workspace/` — `WorkspaceContext` (`open`/`open_in`), `set_explicit_sources()` untuk seed dari CLI (menghindari scan direktori lambat), `filelist.rs`, `project.rs`, `include.rs`, `search.rs`.
- `src/env/runtime/` — `RuntimeContext` (CPU/memori/threadpool/scheduler), `init(&config)` memakai `config.max_threads()`.
- `src/env/compiler/` — `CompilerContext` (wrap `CompileSession`), helper `preprocess`/`lex`/`parse`/`elaborate`/`merge_all`/`OptimizeLevel`.
- `src/env/database/` — `DatabaseContext` (wrap `MicdDatabase`: symbol/graph/metadata/diag accessor).
- `src/env/{cache,diagnostics,telemetry,plugins,security,verification,simulation}/` — context lain sesuai doc.

Integrasi CLI: `main.rs` bangun env via `maria::env::for_cli(cfgctx, ws)` (workspace di-seed dari CLI sources), threading ke `run()`/`run_fast()` untuk telemetry/metrics, `maria::env::shutdown(&mut env)` di akhir. Pipeline compile/sim tidak diubah — env hanya shell orchestrator.

## Key conventions & gotchas

### Operator precedence (parser)
Higher number = tighter binding. `||`(1) < `&&`(2) < `|`(3) < `^`/`~^`(4) < `&`(5) < `<<`/`>>`(6) < `==`/`!=`/`===`(7) < `<`/`<=`/`>`/`>=`(8) < `+`/`-`(9) < `*`/`/`/`%`(10) < `**`(11). **Jangan balik** — higher-number = tighter-binding.

### Loop control flow
`control_flow: Option<FlowControl>` di `SimulationEngine`. Saat check `Continue`/`Break`, gunakan `let cf = self.control_flow.take()` **sekali**, lalu bandingkan `cf` — jangan panggil `take()` dua kali (nilai kedua selalu `None`). Check control_flow di setiap iterasi loop dan di awal setiap statement block.

### Fill literals (`'0`, `'1`, `'x`, `'z`)
Diexpand di `eval_assign_rhs()` (assignment level), bukan di `evaluate_expr()`, karena target width belum diketahui saat expression eval. `LogicVec::fill(val, width)` untuk membuat vector seragam.

### System functions
`$clog2`, `$bits`, `$size`, `$left`, `$right`, `$low`, `$high` dievaluasi di **elaborator** (compile-time) via constant folding. `$clog2` membutuhkan koreksi `is_power_of_two()` (jika power-of-two, hasil = msb - 1).

### `$display` format
`%0d` (zero-padded) **didukung penuh** — format `%0d`, `%0b`, `%0h` bekerja dengan zero-fill padding. Format yang tidak dikenal dicetak literal.

### Test pattern
Test menggunakan `simulate_signals(source, max_time)` yang mengembalikan `Vec<(String, LogicVec)>`. Cari signal dengan `.iter().find(|(n,_)| n == "name")`. Semua test ada di `src/lib.rs` di `mod tests`. Tidak ada test integration terpisah.

### Package support
`package`/`endpackage` + `import pkg::*` / `import pkg::item` di module body. Supports: `Typedef` (enum, struct, union, base) and `Param` (parameter/localparam with optional type keyword). Function/Task imports not yet supported.

### Fork/join support
`fork...join` / `join_any` / `join_none` untuk concurrent execution. Tiap branch berjalan independen, masing-masing dengan delay sendiri. Engine menggunakan `ForkGroup` untuk melacak branch aktif via `Continuation.fork_id`. `join` menunggu semua branch selesai; `join_any` lanjut saat branch pertama selesai; `join_none` lanjut segera. Branch yang berisi delay akan menjadwalkan kerja di masa depan, dan decrement `ForkGroup.remaining` saat semua statement branch habis dikonsumsi (tidak ada lagi delay).

### Constraint & randomize support
`rand`/`randc` modifier in class fields. `constraint name { expr; … }` blocks with relational/equality constraints.
`randomize()` uses rejection sampling (max 100 attempts) — generates random values for `rand` fields,
writes them into the object, and evaluates each constraint expression via `evaluate_ast_expr`.
User-defined `randomize()` methods override the built-in. `rand_fields` and `constraints` stored in
`IrClassDef` (cloned into `execute_randomize` to avoid borrow conflicts).

`.maria` project file
File proyek mendaftar file `.sv` (satu per baris, `#` untuk komentar). Dibaca via `--filelist`/`-f` flag (sama dengan filelist `.f`). Path relatif terhadap direktori `.maria`.

## Files
- `src/simulator/engine.rs:6622` — largest file. Event loop, all statement handlers, loop unrolling, `$display`/`$fopen`/`$urandom`, fork/join tracking, `execute_randomize`, debug hook.
- `src/parser/parser.rs:4997` — second largest. Operator precedence table at line ~1968.
- `src/elaboration/elaborator.rs:3267` — AST→IR translation, constant folding, signal resolution, multidimensional packed array support.
- `src/simulator/parallel.rs:448` — Parallel eval framework (rayon-based).
- `src/simulator/sdf.rs:369` — SDF annotation parser + annotator.
- `src/waveform/fst.rs:244` — FST waveform writer via wavefst crate.
- `src/debugger/mod.rs:585` — Debugger struct + 21 unit tests.

## Run
```shell
cargo run -- test/counter.sv              # single file
cargo run -- -f .maria                    # project file
cargo run -- test/tb_counter.sv -T 200    # max time
cargo run -- file.sv --ast                # print AST
cargo run -- file.sv --tokens             # print tokens
```

## Debug mode

Activate via `--debug` (basic) or `--deep-debug` (with snapshot/reverse). Debug types (`DebugMode`, `StepMode`, `Breakpoint`, `Watchpoint`) defined in `src/simulator/engine.rs`. `Debugger` wrapper in `src/debugger/mod.rs`.

### CLI flags
```shell
--debug                   # enable debug mode (pause at breakpoints)
--deep-debug              # enable + snapshots for reverse debug
--step                    # run one cycle then pause
--break-cycle <N>         # break at cycle N
--break-change <NAME>     # break when signal changes
--break-eq NAME=VAL       # break when signal == VAL (hex)
--watch <NAME>            # watchpoint (pause on change)
--timeline <NAME>         # print signal timeline post-sim
--timeline-len <N>        # max timeline entries (default 20)
--print-signal <NAME>     # print signal value post-sim
--print-state             # print all signal values
--tree                    # print hierarchy tree
--mem <ADDR> <LEN>        # memory inspector (hex)
--snap-interval <N>       # snapshot interval (default 1000)
```

### Breakpoint checking
`debug_check()` dipanggil di akhir setiap cycle (sebelum time increment) di `SimulationEngine::run()`. Cycle breakpoint `break cycle N` pause saat `state.time == N`. Signal breakpoint (`SignalEq`/`SignalNeq`/`SignalChange`) diperiksa setiap cycle via `signal_history`. Watchpoints juga diperiksa di sini — jika nilai berubah, engine pause dan event dicatat.

### Reverse debug (deep-debug)
Snapshot `StateSnapshot` disimpan setiap `snapshot_interval` cycle. `reverse_step()` pop snapshot terakhir dan restore state. `reverse_continue(target)` mundur ke snapshot terdekat ≤ target.

### Signal history
Semua signal dicatat di `signal_history: HashMap<String, Vec<(u64, LogicVec)>>` setiap cycle (maks 100k entry per signal). Dipakai oleh timeline, break-change, dan watchpoint.

## MICD — Maria Incremental Compilation Database

Object database biner (bukan SQL) di `project/.maria/database/` yang membuat compile
lintas run incremental: file yang tidak berubah tidak di-lex/di-parse/di-verifikasi
ulang. **Terintegrasi otomatis ke `run` dan `run_fast`** — bukan flag tambahan.

Struktur `src/micd/`:
- `format.rs` — format file `MDB1`: header 64B + object table + payload, dibaca via mmap (memmap2). Tulis atomik (temp+rename).
- `metadata.rs` — per-file: content hash (xxh3), mtime, size, status, flags hash, deps.
- `graph.rs` — dependency graph file-level + reverse index → `affected(changed)` transitive closure.
- `ast.rs` — `Design` terserialisasi per file via bincode (AST sudah `serde::Serialize/Deserialize`; `Symbol` diserialisasi sebagai string).
- `mod.rs` — layout Git-style (Opsi B db.md): `VERSION` + `registry.json` + `locks/` di root; payload IMMUTABLE content-addressed di `objects/<pid>/<hash>.ast|.preproc` (dedup antar file identik); index mutable di `state/<pid>/*.mdb`. Layout lama `projects/<pid>/` di-migrasi otomatis.
- `verify.rs` — verification cache keyed by content hash (parse/elab ok, diag counts, timing).
- `diag.rs`, `symbol.rs`, `snapshot.rs` — diagnostic per file, index simbol, snapshot build (build-NNN, rollback, di `state/<pid>/snapshots/`).

Integrasi:
- `CompileSession::attach_micd(MicdDatabase)` — restore `prev_designs`/`prev_checksums`/`prev_combined_sources` untuk file yang content hash-nya cocok → `compile()` melewati lexer+parser.
- `CompileSession::save_micd()` — simpan metadata/graph/ast-objek/preproc-objek/verify/symbol/types + auto-snapshot saat ada perubahan. Dipanggil `main.rs` sekali per build (bukan di `compile()` — agar statistik `changed_files` per-build akurat).
- `run_fast`: MICD penuh (restore AST). `run` (legacy): MICD preprocess cache (reuse combined source, skip preprocessor).
- Root: `.maria/database` (override `MARIA_MICD_DIR`). `--recompile` melewati MICD (full rebuild). `--cache-clear` menghapus MICD.
