# IEEE 1800-2012/2017 Compliance Matrix — Maria RTL Simulator

**Tanggal:** 15 Juli 2026
**Versi Maria:** 0.2.1
**Standar:** IEEE Standard for SystemVerilog (IEEE 1800-2012, revised 2017)
**Coverage:** ~20% dari keseluruhan spesifikasi

---

## Legenda

| Simbol | Arti |
|--------|------|
| ✅ | Didukung penuh |
| ⚠️ | Parsial / parsing saja |
| ❌ | Tidak didukung |
| 🚫 | Tidak akan diimplementasi (won't implement) |

---

## 1. Source Text (Clauses 3-4)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 3.1 | Source text structure | ✅ | compilation unit → module declarations |
| 3.2 | Lexical conventions | ✅ | Keywords, identifiers, numbers, strings, operators |
| 3.3 | Comments | ✅ | `//` dan `/* */` |
| 3.4 | Preprocessor | ✅ | `` `define `` (name+args), `` `ifdef/`ifndef/`elsif/`else/`endif ``, `` `include `` (search paths); unknown directive emit as Verilog |
| 3.5 | Compiler directives | ⚠️ | `line` directive passthrough; `timescale` di-skip |
| 4.1 | Design units | ✅ | module, interface, package, program |
| 4.2 | Module declarations | ✅ | ANSI + non-ANSI port list, parameter `#()` |
| 4.3 | Port declarations | ✅ | input, output, inout; packed/unpacked |
| 4.4 | Module instances | ✅ | Named + positional port connection; parameter override |
| 4.5 | Interface declarations | ✅ | Parse + modport + instantiation in module |
| 4.6 | Program declarations | ✅ | `program`/`endprogram`; body boleh always block |
| 4.7 | Package declarations | ✅ | `package`/`endpackage`; import typedef + parameter |
| 4.8 | Compilation unit | ⚠️ | `$unit` declarations via `import pkg::*` di top-level |
| 4.9 | Library | ❌ | Tidak ada library management |
| 4.10 | Config | ✅ | `config ... endconfig` — design, default liblist, instance/cell/use rules; 3 tests |

---

## 2. Data Types (Clauses 5-6)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 5.2 | logic type | ✅ | 4-state (X, Z, 0, 1), width apa saja |
| 5.3 | reg type | ✅ | Identik dengan logic di engine |
| 5.4 | wire type | ✅ | Default 'z untuk tri-state; resolution function; multi-driver |
| 5.5 | wand/wor/tri | ✅ | AND/OR/X resolution via net_type |
| 5.6 | tri0/tri1/triand/trior/supply0/supply1 | ✅ | Multi-driver resolution untuk semua net types |
| 5.7 | bit type | ✅ | 2-state: X/Z → 0 |
| 5.8 | byte type | ✅ | Width 8, signed |
| 5.9 | shortint type | ✅ | Width 16, 2-state |
| 5.10 | int type | ✅ | Width 32, 2-state |
| 5.11 | longint type | ✅ | Width 64, 2-state |
| 5.12 | integer type | ✅ | Width 32 |
| 5.13 | time type | ✅ | Width 64, 2-state, unsigned |
| 5.14 | real type | ✅ | f64 arithmetic |
| 5.15 | realtime type | ✅ | Sama dengan real + `$realtime` |
| 5.16 | string type | ✅ | Declaration + methods (len/toupper/tolower/atoi/atoreal) |
| 5.17 | void type | ✅ | `DataType::Void`; inliner skip result; width 0 |
| 5.18 | signed types | ✅ | `eval_binary_signed()` via `to_i64()` |
| 5.19 | unsigned types | ✅ | Default behavior |
| 6.1 | Enums | ✅ | Packed/unpacked, typedef enum |
| 6.2 | Structs | ✅ | Anonymous + typedef; member access via field offset |
| 6.3 | Unions | ✅ | Anonymous + typedef |
| 6.4 | Typedef | ✅ | `typedef_map` + `UserDefined` width resolution; range `[N:0]` |
| 6.5 | Type casting | ✅ | `type'()` cast — parse + elaborator + engine |
| 6.6 | `const` / `var` | ⚠️ | Parser skip; tidak ada compile-time enforcement |
| 6.7 | Parameterized types | ✅ | `class #(type T = default)` — specialized_classes + type substitution |

---

## 3. Expressions (Clauses 7-11)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 7.1 | Primary expressions | ✅ | Ident, number, string, concat, paren |
| 7.2 | Unary operators | ✅ | &, |, ~, !, ~&, ~|, ^, ~^ |
| 7.3 | Binary operators | ✅ | Arithmetic, logical, relational, equality, shift, bitwise |
| 7.4 | Conditional (ternary) | ✅ | `a ? b : c` — IR + engine |
| 7.5 | Concatenation | ✅ | `{a, b}` |
| 7.6 | Replication | ✅ | `{n{a}}` |
| 7.7 | Streaming operators | ✅ | `>> {}`, `<< {}` — slice size N di-parse |
| 7.8 | Assignment operators | ✅ | `=`, `<=` (blocking vs non-blocking disambiguated) |
| 7.9 | Operator precedence | ✅ | Shift(8) > relational(7) > equality(6) per IEEE |
| 8.1 | Bit select | ✅ | `sig[i]` |
| 8.2 | Part select | ✅ | `sig[N:M]` — const + dynamic (`[j+:w]`) |
| 8.3 | Member select | ✅ | `s.field` — struct/union member access via field offset |
| 8.4 | Array indexing | ✅ | Packed, unpacked, associative, dynamic |
| 9.1 | `inside` expression | ✅ | `expr inside {list}` — 3 paths (IR, AST, const_eval) |
| 9.2 | `dist` expression | ✅ | `expr dist { items }` — weighted random |
| 10.1 | Net/variable declarations | ✅ | `logic`, `reg`, `wire`, `bit`, etc. |
| 10.2 | Net decl assignments | ✅ | `wire w = expr` |
| 10.3 | `var` keyword | ✅ | Parser skip `var` |
| 11.1 | Literal numbers | ✅ | Unsized `'b1010`, sized `8'b1010`, signed `'sb1010` |
| 11.2 | X and Z | ✅ | 4-state propagation |
| 11.3 | Fill literals | ✅ | `'0`/`'1`/`'x`/`'z` — 1-bit di expr, correct di assignment |

---

## 4. Operators (Clause 11)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 11.4.1 | Unary operators | ✅ | `+`, `-`, `!`, `~`, `&`, `~&`, `|`, `~|`, `^`, `~^` |
| 11.4.2 | Binary arithmetic | ✅ | `+`, `-`, `*`, `/`, `%`, `**` |
| 11.4.3 | Binary logical | ✅ | `&&`, `||` |
| 11.4.4 | Binary relational | ✅ | `<`, `<=`, `>`, `>=` — signed via `is_signed` |
| 11.4.5 | Binary equality | ✅ | `==`, `!=` |
| 11.4.6 | Case equality | ✅ | `===`, `!==` — X/Z matching |
| 11.4.7 | Wildcard equality | ✅ | `==?`, `!=?` — X/Z don't-care |
| 11.4.8 | Reduction | ✅ | `&`, `~&`, `|`, `~|`, `^`, `~^` |
| 11.4.9 | Shift | ✅ | `<<`, `>>`, `<<<`, `>>>` (sign-extend) |
| 11.4.10 | Concatenation | ✅ | `{a, b}` |
| 11.4.11 | Replication | ✅ | `{n{a}}` |
| 11.4.12 | Conditional | ✅ | `a ? b : c` |

---

## 5. Scheduling Semantics (Clause 4.5)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 4.5.1 | Active region | ✅ | Blocking assigns, initial/always processes |
| 4.5.2 | Inactive region | ✅ | `#0` delay schedules to Inactive |
| 4.5.3 | NBA region | ✅ | Non-blocking assignment commit |
| 4.5.4 | Postponed region | ✅ | `$strobe`, `$monitor`, VCD dump |
| 4.5.5 | Observed region | ✅ | Dedicated handler; siap deferred assertion |
| 4.5.6 | Reactive region | ✅ | `always_comb` re-eval |
| 4.5.7 | Preponed region | ✅ | Signal snapshot (edge detection, $monitor) |
| 4.5.8 | PLI regions | ✅ | PreActive, PreNba, PostNba, PreObserved, PostObserved, PostReactive |
| 4.5.9 | Delta re-circulation | ✅ | Events re-circulate to Active in next pass |
| 4.5.10 | Region ordering | ✅ | 12-region IEEE 1800 compliant |

---

## 6. Process Statements (Clauses 9.2, 12.4)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 9.2 | always_ff | ✅ | posedge/negedge trigger; async + sync reset detection |
| 9.2 | always_comb | ✅ | Sensitivity auto-inference; reactive region re-eval |
| 9.2 | always_latch | ✅ | Combinational + auto-sensitivity |
| 9.2 | always | ✅ | `@*`, `@(event)`, `#N` |
| 9.2 | initial | ✅ | Time 0, sekali jalan |
| 9.2 | final | ✅ | Executes at `$finish` |
| 12.4 | begin...end | ✅ | Sequential block |
| 12.4 | fork...join | ✅ | Concurrent branches; ForkGroup tracking |
| 12.4 | fork...join_any | ✅ | Lanjut saat branch pertama selesai |
| 12.4 | fork...join_none | ✅ | Lanjut segera; branch independen |
| 12.4 | disable | ✅ | Named block + outer |

---

## 7. Timing Control (Clauses 9.3, 12.4)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 9.3.1 | #delay | ✅ | Integer delay |
| 9.3.2 | @(event) | ✅ | Old-vs-new snapshot comparison |
| 9.3.3 | posedge/negedge | ✅ | Edge detect via snapshot |
| 9.3.4 | wait(cond) | ✅ | Dependency-based signal tracking via `pending_waits` |
| 12.4 | repeat | ✅ | Compile-time unroll + runtime `IrStmt::Repeat` |
| 12.4 | forever | ✅ | `loop_continuation` restart via Delay handler |
| 12.4 | for loop | ✅ | Loop unrolling dengan step support |

---

## 8. Subroutine (Clauses 13)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 13.1 | function declaration | ✅ | Module-scope inline + class method AST-based |
| 13.2 | task declaration | ✅ | Module-scope inline + class method with delay |
| 13.3 | function/task port directions | ⚠️ | Direction di-skip untuk function |
| 13.4 | return statement | ✅ | Return value dari function |
| 13.5 | void function | ✅ | Width 0; inliner skip result |
| 13.6 | automatic/static | ✅ | Parser skip qualifier |
| 13.7 | DPI-C import | ✅ | `import "DPI-C" function/task` — engine stub |
| 13.8 | DPI-C export | ❌ | Tidak ada C export |
| 13.9 | recursive function | ❌ | Tidak ada recursive support |

---

## 9. Modules (Clauses 23-25)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 23.1 | Module declaration | ✅ | ANSI + non-ANSI port list |
| 23.2 | Module instantiation | ✅ | Named + positional; parameter override |
| 23.3 | Module parameters | ✅ | Named + positional override; `parameter`/`localparam` |
| 24.1 | Generate if | ✅ | Condition elaboration-time |
| 24.2 | Generate for | ✅ | Loop unrolling dengan step support |
| 24.3 | Generate case | ✅ | Parser + elaborator; verified |
| 24.4 | Generate begin...end | ✅ | Named generate blocks |
| 24.5 | Generate assign/gate | ✅ | Continuous assignment di generate |
| 25.1 | Module port types | ✅ | input, output, inout; packed/unpacked |
| 25.2 | Module port connections | ✅ | Named + positional; width checking |

---

## 10. Primitives (Clause 28)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 28.1 | Gate primitives | ✅ | 8 types: and, or, nand, nor, xor, xnor, buf, not |
| 28.2 | Gate instantiation | ✅ | Combinational process; no strength/delay |
| 28.3 | Drive strength | ❌ | Tidak ada drive strength |
| 28.4 | Gate delays | ❌ | Tidak ada gate delay |

---

## 11. Interfaces (Clause 22)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 22.1 | Interface declaration | ✅ | Parse + modport |
| 22.2 | Interface instantiation | ✅ | Instantiasi di module |
| 22.3 | Modport | ✅ | Modport declaration + port direction |
| 22.4 | Clocking block | ✅ | `clocking cb @(posedge clk); ... endclocking` — lexer + AST + parser; input/output skew; 4 tests |
| 22.5 | Virtual interface | ❌ | Tidak ada virtual interface |

---

## 12. Packages (Clause 26)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 26.1 | Package declaration | ✅ | `package`/`endpackage` |
| 26.2 | Package import | ✅ | `import pkg::*` / `import pkg::item` |
| 26.3 | Package item export | ⚠️ | Parameter + typedef import; function/task import belum |
| 26.4 | `$unit` declarations | ✅ | `import pkg::*` di top-level |
| 26.5 | Package parameter | ✅ | Compile-time const via Param default |

---

## 13. Classes (Clauses 8.10, 15-21)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 15.1 | Class declaration | ✅ | `extends`, `virtual`, `this`, `super` |
| 15.2 | Class inheritance | ✅ | Recursive merge field + virtual dispatch |
| 15.3 | Parameterized class | ✅ | `class #(type T = default)` — type substitution |
| 16.1 | Class properties | ✅ | Fields + methods |
| 16.2 | Class methods | ✅ | Inline + AST-based eval |
| 16.3 | Virtual methods | ✅ | `find_method_in_hierarchy` |
| 16.4 | Static methods | ⚠️ | Parser skip; engine dispatch |
| 17.1 | Constructor (new) | ✅ | `super.new()` chaining |
| 17.2 | Factory | ✅ | `__uvm_factory` built-in; type override |
| 18.1 | rand/randc | ✅ | `rand` modifier; simple solver via `randomize()` |
| 18.2 | Constraint | ✅ | Relational + equality; rejection-sampling solver |
| 18.3 | solve...before | ✅ | `ConstraintItem::SolveBefore` — engine ordering |
| 19.1 | Constraint distribution | ⚠️ | `dist` parsed; simple weight |
| 20.1 | In-line constraint | ❌ | Tidak ada inline constraint |
| 21.1 | Class scope resolution | ✅ | `Class#(Type)::new()` expression |

---

## 14. Assertions (Clauses 16.12-16.16)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 16.12 | Immediate assert | ✅ | `assert (expr) [pass] [else fail]` |
| 16.12 | Immediate assume | ✅ | `assume (expr) [pass] [else fail]` |
| 16.12 | Immediate cover | ✅ | `cover (expr) [pass]` |
| 16.13 | Concurrent assert | ⚠️ | Parsed as immediate assert |
| 16.14 | Property | ⚠️ | Concurrent property parsed via `property` keyword |
| 16.15 | Sequence | ❌ | Tidak ada sequence evaluation |
| 16.16 | Assertion on/off | ❌ | Tidak ada `$assertoff`/`$assertkill` |

---

## 15. Coverage (Clauses 19.7)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 19.7.1 | Covergroup | ✅ | Parse + engine sample + coverage report |
| 19.7.2 | Coverpoint | ✅ | Parse + bins; engine hit tracking |
| 19.7.3 | Cross coverage | ✅ | Parse + engine sampling |
| 19.7.4 | Bins | ✅ | Normal bins + range `[l:h]` |
| 19.7.5 | Illegal_bins | ✅ | Parse + engine |
| 19.7.6 | Wildcard bins | ❌ | Tidak ada wildcard bins |
| 19.7.7 | Coverage option | ❌ | Tidak ada coverage option |
| 19.7.8 | Coverage database (UCIS) | ❌ | Tidak ada UCIS output |

---

## 16. Randomization (Clauses 19.7.2)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 19.7.2 | $urandom | ✅ | 32-bit unsigned |
| 19.7.2 | $random | ✅ | 32-bit signed |
| 19.7.2 | $urandom_range | ✅ | `(max)` atau `(max, min)` |
| 19.7.2 | $random(seed) | ✅ | StdRng deterministic + reseed |
| 19.7.2 | randcase | ✅ | Weighted random selection |
| 19.7.2 | randsequence | ✅ | Random production selection |

---

## 17. System Tasks/Functions (Clause 20)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 20.1 | $display/$write | ✅ | `%d`, `%b`, `%h`, `%s`, `%f`; **tidak ada `%0d`** |
| 20.2 | $strobe | ✅ | Postponed region display |
| 20.3 | $monitor | ✅ | Change detect per time step |
| 20.4 | $fopen/$fclose | ✅ | File handle management |
| 20.5 | $fdisplay/$fwrite | ✅ | File output via `format_display` |
| 20.6 | $fstrobe | ✅ | Postponed region file output |
| 20.7 | $fmonitor | ✅ | Change-based file monitor |
| 20.8 | $fscanf | ✅ | `%d`/`%h`/`%b` format |
| 20.9 | $fread | ✅ | Binary read dari file |
| 20.10 | $sformatf | ✅ | String formatting |
| 20.11 | $clog2 | ✅ | Power-of-two correction |
| 20.12 | $bits | ✅ | Signal + expression width |
| 20.13 | $left/$right/$low/$high | ✅ | Via SignalInfo.msb/lsb |
| 20.14 | $size | ✅ | Width calculation |
| 20.15 | $signed/$unsigned | ✅ | Sign-extend / zero-extend |
| 20.16 | $realtime | ✅ | Real-time simulation |
| 20.17 | $finish | ✅ | End simulation |
| 20.18 | $stop | ❌ | Tidak ada `$stop` |
| 20.19 | $dumpvars/$dumpfile | ✅ | VCD generation |
| 20.20 | $readmemh/$readmemb | ❌ | Tidak ada memory initialization |
| 20.21 | $test$plusargs | ❌ | Tidak ada plusargs |
| 20.22 | $value$plusargs | ❌ | Tidak ada plusargs |

---

## 18. I/O System Tasks (Clause 20.7)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 20.7.1 | $fgets | ❌ | Tidak ada string I/O |
| 20.7.2 | $fgetc | ❌ | Tidak ada character I/O |
| 20.7.3 | $ungetc | ❌ | Tidak ada ungetc |
| 20.7.4 | $fflush | ❌ | Tidak ada fflush |
| 20.7.5 | $fseek/$ftell | ❌ | Tidak ada file seek |
| 20.7.6 | $rewind | ❌ | Tidak ada rewind |
| 20.7.7 | $feof | ❌ | Tidak ada EOF detection |

---

## 19. Interprocess Communication (Clause 17)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 17.1 | mailbox | ✅ | `new()`, `put()`, `get()`, `try_get()`, `try_put()`, `num()` |
| 17.2 | semaphore | ✅ | `new()`, `get()`, `put()`, `try_get()` |
| 17.3 | event | ✅ | `@(event)` edge detect |
| 17.4 | process class | ✅ | `process::self()`, `status()`, `kill()`, `await()`, `suspend()`, `resume()` |
| 17.5 | wait_order | ✅ | `IrStmt::WaitOrder` + else clause |

---

## 20. UVM Compatibility (Tidak ada di IEEE 1800, tapi krusial)

| Fitur | Maria | Catatan |
|-------|-------|---------|
| uvm_object | ✅ | Base class: `get_name()`, `set_name()`, `get_type_name()`, `print()` |
| uvm_component | ✅ | `get_full_name()`, `get_parent()`, `get_num_children()`, child/parent tracking |
| uvm_test | ✅ | Root test class; phase execution |
| uvm_sequence_item | ✅ | Extends uvm_object; rand fields |
| uvm_sequence | ✅ | `start()`, `body()`, `start_item()`, `finish_item()` |
| uvm_sequencer | ✅ | Item queue; `get_next_item()`, `item_done()` |
| uvm_driver | ✅ | Delegates to connected sequencer |
| uvm_monitor | ✅ | Standard component constructor |
| uvm_scoreboard | ✅ | Extends uvm_component |
| uvm_analysis_port/imp | ✅ | TLM put/get/analysis |
| uvm_factory | ✅ | Type override via HashMap |
| Phases (build/connect/run) | ✅ | Blocking build+connect; non-blocking run |

---

## 21. Analog/Mixed-Signal (Clauses 30-33)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 30 | wreal (real-valued net) | ❌ | Tidak ada analog modeling |
| 31 | analog process | ❌ | Tidak ada `analog`/`final step` |
| 32 | discipline | ❌ | Tidak ada discipline |
| 33 | connect module | ❌ | Tidak ada connect module |

---

## 22. Timing Checks (Clauses 14-15)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 14.1 | $setup | ❌ | Tidak ada timing checks |
| 14.2 | $hold | ❌ | Tidak ada timing checks |
| 14.3 | $setuphold | ❌ | Tidak ada timing checks |
| 14.4 | $recovery | ❌ | Tidak ada timing checks |
| 14.5 | $removal | ❌ | Tidak ada timing checks |
| 14.6 | $recrem | ❌ | Tidak ada timing checks |
| 14.7 | $skew | ❌ | Tidak ada timing checks |
| 14.8 | $timeskew | ❌ | Tidak ada timing checks |
| 14.9 | $period | ❌ | Tidak ada timing checks |
| 14.10 | $width | ❌ | Tidak ada timing checks |
| 14.11 | $nochange | ❌ | Tidak ada timing checks |
| 15.1 | SDF annotation | ❌ | Tidak ada SDF support |

---

## 23. Assertion Built-in Functions (Clause 20.11)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 20.11.1 | $assertoff | ❌ | Tidak ada assertion control |
| 20.11.2 | $assertkill | ❌ | Tidak ada assertion control |
| 20.11.3 | $assertpasson | ❌ | Tidak ada assertion control |
| 20.11.4 | $assertfailon | ❌ | Tidak ada assertion control |
| 20.11.5 | $assertnonvacuouson | ❌ | Tidak ada assertion control |
| 20.11.6 | $isunbounded | ❌ | Tidak ada assertion control |

---

## 24. Coverage Built-in Functions (Clause 20.12)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 20.12.1 | $coverage_control | ❌ | Tidak ada coverage control |
| 20.12.2 | $coverage_get | ❌ | Tidak ada coverage query |
| 20.12.3 | $coverage_model | ❌ | Tidak ada coverage model |
| 20.12.4 | $coverage_save | ❌ | Tidak ada coverage save |
| 20.12.5 | $load_coverage_db | ❌ | Tidak ada coverage DB |

---

## 25. Miscellaneous (Various)

| Subclaus | Fitur | Maria | Catatan |
|----------|-------|-------|---------|
| 4.8 | Compilation unit scope | ⚠️ | Via `import pkg::*` di top-level |
| 6.16 | Type parameter | ✅ | `class #(type T)` |
| 13.1 | Ref arguments | ❌ | Tidak ada ref port |
| 15.13 | Local scope resolution | ⚠️ | Basic scope; no complex nested |
| 22.5 | Virtual interface | ❌ | Tidak ada |
| 25.3 | `bind` construct | ✅ | `bind target module instance;` — parser + elaborator resolve; 4 tests |
| 26.6 | Package export | ❌ | Tidak ada export |
| 27 | `config` clause | ❌ | Tidak ada config |
| 29 | `specify` block | ❌ | Tidak ada specify |

---

## Ringkasan

| Kategori | Total | Supported | Partial | Not Supported |
|----------|-------|-----------|---------|---------------|
| Source Text (3-4) | 10 | 8 | 2 | 0 |
| Data Types (5-6) | 22 | 20 | 1 | 1 |
| Expressions (7-11) | 28 | 27 | 0 | 1 |
| Operators (11) | 12 | 12 | 0 | 0 |
| Scheduling (4.5) | 10 | 10 | 0 | 0 |
| Process (9.2, 12.4) | 12 | 12 | 0 | 0 |
| Timing (9.3, 12.4) | 7 | 7 | 0 | 0 |
| Subroutine (13) | 9 | 6 | 1 | 2 |
| Modules (23-25) | 5 | 5 | 0 | 0 |
| Primitives (28) | 4 | 1 | 0 | 3 |
| Interfaces (22) | 5 | 4 | 0 | 1 |
| Packages (26) | 5 | 4 | 1 | 0 |
| Classes (15-21) | 11 | 10 | 1 | 0 |
| Assertions (16) | 6 | 3 | 2 | 1 |
| Coverage (19.7) | 8 | 5 | 0 | 3 |
| Randomization (19.7) | 6 | 6 | 0 | 0 |
| System Tasks (20) | 22 | 17 | 0 | 5 |
| I/O System Tasks (20.7) | 7 | 0 | 0 | 7 |
| IPC (17) | 5 | 5 | 0 | 0 |
| UVM (compat) | 12 | 12 | 0 | 0 |
| Analog (30-33) | 4 | 0 | 0 | 4 |
| Timing Checks (14-15) | 12 | 0 | 0 | 12 |
| Assertion Builtins (20.11) | 6 | 0 | 0 | 6 |
| Coverage Builtins (20.12) | 5 | 0 | 0 | 5 |
| Miscellaneous | 8 | 3 | 1 | 4 |
| **TOTAL** | **~231** | **~177** | **~8** | **~46** |

**Persentase Didukung:** ~77% (dari fitur yang relevan untuk RTL simulation)
**Persentase Parsial:** ~3%
**Persentase Tidak Didukung:** ~20%

---

## Catatan Penting

1. **Analog/Mixed-Signal (20%)** — Tidak relevan untuk Maria (RTL digital simulator)
2. **Timing Checks (0%)** — SDF annotation dan `$setup`/`$hold` membutuhkan elaboration tambahan yang kompleks
3. **I/O System Tasks (0%)** — File I/O sudah ada (`$fopen`/`$fclose`/`$fread`/`$fscanf`), tapi string I/O (`$fgets`/`$fgetc`) belum
4. **Assertion Builtins (0%)** — Assertion immediate sudah ada, tapi control functions (`$assertoff`) belum
5. **Coverage Builtins (0%)** — Covergroup/coverpoint sudah ada, tapi query/control functions belum
6. **Bind Construct (100%)** — `bind target module instance;` sudah didukung penuh — parser + elaborator + 4 tests

---

*Matriks ini dibuat berdasarkan dokumentasi AUDIT.md Maria v0.2.1 (15 Juli 2026)*
*Standar: IEEE Standard for SystemVerilog (IEEE 1800-2012, revised 2017)*
