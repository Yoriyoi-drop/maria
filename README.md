# maria — RTL Simulator untuk SystemVerilog

**Versi 0.2.9** | Rust | 577 tests | MIT

A Rust-based RTL simulator for SystemVerilog. Compiles `.sv` files through a pipeline of preprocessor → lexer → parser → AST → elaborator → IR → simulation engine → VCD/FST output.

## Pipeline

```
.sv → preprocessor → lexer → parser → AST → elaborator → IR → engine → VCD/FST
```

## Quick start

```shell
cargo run -- test/counter.sv              # simulate counter
cargo run -- test/tb_counter.sv -T 200    # with max time
cargo run -- file.sv --ast                # print AST
cargo run -- file.sv --tokens             # print tokens
```

## Project file

A `.maria` file lists `.sv` sources (one per line, `#` for comments):

```
counter.sv
tb_counter.sv
```

```shell
cargo run -- --start .maria
```

## CLI flags

```
- T <N>        max simulation time
--ast          print AST (no simulation)
--tokens       print tokens (no simulation)
--top <MOD>    top-level module name
--debug        enable debug mode (breakpoints)
--deep-debug   enable + snapshots for reverse debug
--step         single-cycle execution
-I <DIR>       include directory for `include
-D <MACRO>     define macro
-f <FILE>      file list (like -f in VCS)
--coverage     print coverage report
--coverage-ucis [PATH]  export UCIS XML
--start <FILE> project file (.maria)
```

## Fitur utama

- Full 4-state logic (X/Z/0/1) dengan propagation
- IEEE 1800 12-region stratified event scheduler
- `always_ff` / `always_comb` / `always_latch` / `initial` / `final`
- `fork`/`join`/`join_any`/`join_none` concurrent execution
- `interface` + `modport`, `package` + `import`, `program` block
- OOP: class, `extends`, virtual dispatch, `super.new()`, parameterized class
- UVM: `uvm_object`, `uvm_component`, `uvm_sequence`/`sequencer`/`driver`, factory, TLM, phases
- SVA: immediate `assert`/`assume`/`cover`, concurrent property parsing
- Coverage: `covergroup`/`coverpoint`/`cross`/`bins` + UCIS XML export
- Constraint randomize: `rand`/`randc`, `constraint`, `solve...before`, `dist`
- DPI-C import, `bind` construct, `clocking` block, `config`/`libmap`/`use`
- SDF annotation, FST waveform (zlib compression), VCD hierarchical dump
- `mailbox`/`semaphore`/`process` class, `randcase`/`randsequence`
- `$sformatf`, `$fopen`/`$fclose`/`$fdisplay`/`$fwrite`/`$fstrobe`/`$fmonitor`/`$fscanf`/`$fread`
- `$urandom`/`$random(seed)`/`$urandom_range`/`$realtime`
- Debugger: breakpoint, watchpoint, step, reverse debug, timeline, hierarchy tree
- Parallel simulation framework: `ParallelConfig`, `evaluate_expr_simple`, `evaluate_stmt_block_parallel`, `parallel_snapshot` (rayon-based)
- JIT stub: basic expression compilation via `JITCompiler` (Cranelift integration planned)
- UVM macros: `uvm_macros.svh` — info/warning/error/fatal, factory utils, field macros
- Picorv32 RISC-V CPU (3049 LOC) compilation + simulation completed
- AXI + Wishbone wrapper simulation via `--top`
- IEEE 1800 compliance ~78% fitur relevan RTL

## Build & test

```shell
cargo build
cargo test
cargo test <test_name>
```

## Architecture

| Layer | File | LOC |
|-------|------|-----|
| CLI | `src/main.rs` | — |
| Library | `src/lib.rs` | — |
| Preprocessor | `src/parser/preprocessor.rs` | — |
| Lexer | `src/parser/lexer.rs` | — |
| Parser | `src/parser/parser.rs` | ~5100 |
| AST | `src/ast/` | expr, stmt, types, const_eval, inline |
| Elaborator | `src/elaboration/elaborator.rs` | ~3400 |
| IR | `src/ir/ir.rs` | — |
| Simulator | `src/simulator/` | engine(~6700), state, value, types, sdf, util, jit, parallel |
| Waveform | `src/waveform/` | vcd.rs, fst.rs |
| Debugger | `src/debugger/` | mod.rs (~585) |
| Tests | `src/tests/` | mod.rs + edge, parse_error, elab_error, fuzz, regression |
| UVM macros | `uvm_macros.svh` | — |

## License

MIT
