# maria — RTL Simulator untuk SystemVerilog

Rust-based SystemVerilog simulator. Pipeline: preprocessor → lexer → parser → AST → elaborator → IR → simulation engine → VCD output.

## Build & Test

```shell
cargo build
cargo test                    # all unit tests (in src/lib.rs, in-module tests)
cargo test --lib              # same, excludes main.rs
cargo test <test_name>        # single test (no --lib needed if unique)
```

No CI, no lint, no typecheck shortcuts. Just `cargo test`. 577 tests pass.

## Pipeline architecture

1. **`src/main.rs`** — CLI entrypoint. Reads `.sv` file(s), concatenates, feeds through lexer → parser → elaborator → engine.
2. **`src/lib.rs`** — Library entrypoint. Exposes `compile_str()`, `simulate_str()`, `simulate_signals()`. Tests live in `src/tests/mod.rs`.
3. **`src/parser/`** — `lexer.rs` (tokenizer), `parser.rs` (Pratt-style top-down operator precedence), `preprocessor.rs` (`` `ifdef ``/`define`).
4. **`src/ast/`** — `expr.rs`, `stmt.rs`, `types.rs`, `const_eval.rs`, `inline.rs` (function inlining for `loop_unroll` and `substitute_loop_var`).
5. **`src/elaboration/elaborator.rs`** — AST → IR, signal collection, type resolution, loop unrolling, constant folding for `$clog2`/`$bits`/`$size`/`$left`/`$right`/`$low`/`$high`.
6. **`src/ir/ir.rs`** — IR types (`IrStmt`, `IrExpr`, `LogicVec`).
7. **`src/simulator/`** — `engine.rs` (event-driven scheduler), `types.rs` (debug/event/UVM types), `state.rs` (signal storage), `value.rs` (`eval_binary`, `eval_unary`).
8. **`src/waveform/vcd.rs`** — VCD dump.
9. **`src/debugger/mod.rs`** — `Debugger` struct wrapping `SimulationEngine`. Step, breakpoint, watchpoint, timeline, hierarchy tree, reverse debug, memory inspect. 21 unit tests inline.

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
`%0d` (zero-padded) **tidak didukung** — hanya `%d` dasar. Format yang tidak dikenal dicetak literal.

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
File proyek mendaftar file `.sv` (satu per baris, `#` untuk komentar). Dibaca via `--start` flag. Path relatif terhadap direktori `.maria`.

## Files
- `src/simulator/engine.rs:6622` — largest file. Event loop, all statement handlers, loop unrolling, `$display`/`$fopen`/`$urandom`, fork/join tracking, `execute_randomize`, debug hook.
- `src/parser/parser.rs:4997` — second largest. Operator precedence table at line ~1968.
- `src/elaboration/elaborator.rs:3267` — AST→IR translation, constant folding, signal resolution, multidimensional packed array support.
- `src/debugger/mod.rs:585` — Debugger struct + 21 unit tests.

## Run
```shell
cargo run -- test/counter.sv              # single file
cargo run -- --start .maria               # project file
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
