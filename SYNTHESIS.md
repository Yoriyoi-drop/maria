# Maria Synthesis — Desain Arsitektur (SIR + Pass Manager, ala Vivado/Yosys)

> **Status:** Draf Desain v0.2 — mengadopsi arsitektur SIR node-based + pass manager
> **Versi target Maria:** 0.5.0
> **Referensi flow:** Xilinx Vivado (synth_design → opt/place/route → report_timing)
> **Prinsip:** (1) reuse maksimal pipeline Maria (elaborator → `IrDesign`);
> (2) **SIR node-based** sebagai IR tengah synthesis — jangan langsung AST →
> Verilog; (3) teknologi hanya lewat device abstraction (`maria-tech`).

---

## 1. Pendahuluan & Visi

Maria saat ini adalah **simulator RTL** yang lengkap: preprocessor → lexer →
parser → AST → elaborator (`IrDesign`) → engine simulasi → VCD/FST. AUDIT.md
mencatat dua celah strategis yang desain ini jawab:

- **COMP-10** — *Tidak ada gate-level optimization* (High)
- **ENT-33** — *No integration with synthesis tools (DC, Genus, Yosys)* (Critical)

**Visi:** Maria bertransformasi dari *simulator* menjadi **hardware compiler
lengkap**: source HDL → elaboration → synthesis → technology mapping →
timing/area → netlist. Flow end-to-end ala Vivado:

```text
RTL (.sv/.svh/.mv)
   │  maria synth
   ▼
SIR (node-based) ──► optimizer passes ──► technology mapping (maria-tech)
   │
   ▼
Netlist gate-level (mapped)             ──► laporan utilisasi (LUT/FF/BRAM/DSP)
   │  maria mimpl
   ▼
Place & Route (FPGA grid)               ──► laporan util pasca-P&R
   │  maria msta
   ▼
STA (setup/hold, WNS/TNS)               ──► laporan timing + critical path
   │
   ▼
Netlist + SDF ──► gate-level simulation (mesin sim Maria sendiri!)
   └──► equivalence check RTL ↔ netlist (Z3 via maria-formal)
```

Tiga prinsip desain:

| # | Prinsip | Arti |
|---|---------|------|
| 1 | **Reuse, bukan tulis ulang** | Elaborator sudah menghasilkan `IrDesign` ter-flatten dengan clock/reset/width/signed final. Synthesis berjalan DI ATAS `IrDesign`. |
| 2 | **SIR node-based** | Synthesis IR adalah **graf nilai node** (`AND/OR/ADD/MUX/REGISTER`), BUKAN pohon statement (`always_ff`/`if`). Jangan pertahankan bentuk AST/Verilog terlalu lama. |
| 3 | **Pass manager, bukan pipeline hardcode** | Optimasi/mapping adalah kumpulan pass terdaftar (`SynthPass`) yang bisa dikomposisi via preset — bukan satu fungsi raksasa. |
| 4 | **Device abstraction** | Satu flow, banyak back-end: **Generic / FPGA / ASIC / Custom**. Tidak ada logika device di dalam core. |
| 5 | **Hasil selalu bisa diverifikasi** | Netlist harus bisa (a) disimulasikan engine Maria, (b) dicek setara dengan RTL via Z3, (c) dilaporkan utilisasi & timing ala Vivado. |

---

## 2. Arsitektur Menyeluruh

```
        .mv source                      .sv/.svh
             │                              │
             └────────────┬─────────────────┘
                          ▼
                 ┌─────────────────┐
                 │  Preprocessor   │        (reuse — MICD)
                 └────────┬────────┘
                          ▼
                 ┌─────────────────┐
                 │  Lexer / Parser │        (reuse)
                 └────────┬────────┘
                          ▼
                 ┌─────────────────┐
                 │   RTL IR        │        AST + HIR
                 └────────┬────────┘
                          ▼
                 ┌─────────────────┐
                 │  Elaboration    │        params/generate/hierarchy/types
                 │  → IrDesign     │        (reuse — sudah ada)
                 └────────┬────────┘
                          ▼
          ═══════ maria-sir (crate baru) ═══════
                 ┌─────────────────┐
                 │  RTL → SIR      │        lowering (lower.rs)
                 │  SirModule      │        node-based, register eksplisit
                 └────────┬────────┘
                          ▼
          ═══════ maria-synth (pass manager) ═══════
                 ┌─────────────────────────┐
                 │  Synthesis Optimizer    │  const prop · DCE · CSE
                 │  (pass manager)         │  boolean · mux · arith
                 │                         │  width · FSM · register
                 └───────────┬─────────────┘
                             ▼
                 ┌───────────────────────┐
                 │  Technology Mapping   │  maria-tech (generic/fpga/asic)
                 └───────────┬───────────┘
                             ▼
                 ┌───────────────────────┐
                 │  Technology Netlist   │  maria-netlist (mapped cells)
                 └───────┬───────────────┘
                         ▼
              ┌──────────┼──────────┐
              ▼          ▼          ▼
          Verilog    JSON / IR   Reports
          Netlist    Netlist     Area / Timing
              │
              ▼
        maria mimpl → maria msta (P&R + STA)
```

**Aturan emas:** `AST → Verilog` DILARANG. Semua tahap synthesis berjalan di
atas SIR/netlist, bukan di atas statement AST. Ini yang memungkinkan optimasi
serius (const folding, CSE, width reduction, retiming) tanpa melawan struktur
syntax.

---

## 3. SIR — Synthesis Intermediate Representation (crate `maria-sir`)

### 3.1 Mengapa SIR?

RTL `assign y = (a & b) | c;` TIDAK dipertahankan sebagai bentuk AST.
Diturunkan menjadi graf node:

```text
Node 0: AND   input0 = a   input1 = b
Node 1: OR    input0 = Node0  input1 = c
output y = Node1
```

```text
        a ───┐
             AND ───┐
        b ───┘      │
                    OR ─── y
        c ──────────┘
```

Synthesis tidak peduli struktur syntax; ia peduli *siapa menghitung apa*.

### 3.2 Data model (implementasi: `crates/maria-sir/src/sir.rs`)

```rust
pub struct SirModule {
    pub name: Symbol,
    pub inputs: Vec<SirPort>,
    pub outputs: Vec<SirPort>,
    pub values: Vec<SirValue>,       // tabel nilai (SSA-like)
    pub wires: Vec<SirWire>,         // net bernama (koneksi RTL)
    pub nodes: Vec<SirNode>,         // graf logika kombinasi
    pub registers: Vec<SirRegister>, // FF eksplisit
    pub src_map: HashMap<Symbol, (String, usize, usize)>, // traceability
}

pub enum SirNodeKind {
    And, Or, Xor, Not,
    Mux,
    Add, Sub, Mul, Div, Mod,
    Shl, Shr, Sar,
    Eq, Ne, Lt, Le, Gt, Ge,
    ReduceAnd, ReduceOr, ReduceXor,
    Concat, Slice { msb: usize, lsb: usize },
    Buffer, TriState,
}

pub struct SirRegister {
    pub name: Symbol,
    pub d: ValueId,          // D-logic (hasil mux/add/...)
    pub q: ValueId,          // selalu SirValue::Reg(id)
    pub clock: ValueId,
    pub reset: Option<ResetSpec>,   // { signal, value, polarity, async }
    pub enable: Option<ValueId>,    // clock enable (iff signal tunggal)
    pub width: usize,
}

pub enum SirValue { Port(PortId), Wire(WireId), Const(LogicVec), Node(NodeId), Reg(RegisterId) }
```

### 3.3 Jangan dekat dengan Verilog

```systemverilog
always_ff @(posedge clk) begin
    if (rst)      q <= 0;
    else if (en)  q <= d;
end
```

menjadi SIR:

```text
REGISTER
├── clock = clk
├── reset = rst      (nilai 0, async)
├── enable = en
└── D = <graf node: mux(cond, ...)>
```

Bukan `always_ff → if → else → assignment`. SIR menyimpan **semantik
register**, bukan syntax-nya.

### 3.4 Lowering RTL → SIR (implementasi: `lower.rs`)

Dari `IrDesign` (sudah type-checked + flatten):

- Port & signal → `SirPort`/`SirWire` + slot nilai.
- `Process::Sequential` → `SirRegister`: clock dari `ClockEdge`, reset dari
  `ResetInfo` (nilai/polaritas/async), enable dari `iff` (signal tunggal).
  **D-logic** dibangun dengan *mux builder*: statement `if/case` menjadi
  rantai `MUX` (branch tanpa assign = **hold/RegQ** untuk FF, 0 untuk comb).
- `Process::Combinational`/`CombReactive` → DAG node.
- Ekspresi → node (`BinaryOp` → node, `Cond` → `MUX`, select → `SLICE`).
  Lebar konstanta dinormalisasi ke lebar operand (`count + 1` → ADD 8-bit,
  bukan 32-bit literal SV); assignment mematuhi *context rule* (truncate /
  zero-extend ke lebar signal).
- Konstruk tak didukung dicatat di `LowerResult.skipped` (jujur, bukan
  diam-diam salah).

Contoh nyata `counter.sv` 8-bit (output `maria synth --dump-sir`):

```text
── SIR module: counter ──
  ports  in=3 out=1     nodes 7     regs 1

Registers:
  r0  count  [7:0]  d=n6  q=q(count)  clk=clk  rst=rst_n(0x0,low,async)  ce=-

Nodes:
  n0  NOT      [0:0] <- rst_n
  n2  EQ       [0:0] <- q(count), 8'h63
  n3  ADD      [7:0] <- q(count), 8'h1
  n4  MUX      [7:0] <- n2, 8'h0, n3
  n5  MUX      [7:0] <- enable, n4, q(count)     # hold bila !enable
  n6  MUX      [7:0] <- n0, n1, n5               # reset branch
```

---

## 4. Pass Manager & Preset

### 4.1 Trait pass

```rust
pub trait SynthPass {
    fn name(&self) -> &'static str;
    /// Jalankan pass; kembalikan jumlah rewrite (fold/alias/simplify/eliminasi).
    fn run(&mut self, ctx: &mut SynthContext) -> Result<usize, SimError>;
}

pub struct SynthPipeline {
    passes: Vec<Box<dyn SynthPass>>,
}
impl SynthPipeline {
    pub fn add<P: SynthPass + 'static>(&mut self, pass: P) -> &mut Self;
    /// Ownership move (tanpa clone) → (module, Vec<PassResult>).
    pub fn run(&mut self, module: SirModule) -> Result<(SirModule, Vec<PassResult>), SimError>;
    pub fn with_preset(name: &str) -> Result<Self, SimError>;
}
```

`PassResult { name, nodes_before, nodes_after, changed }` dihitung pipeline per
pass (untuk report/statistik). Pipeline `with_preset` mengakhiri dengan
**fixed-point**: `ConstFold + DCE` sekali lagi agar optimasi yang membuka
peluang fold baru (mis. `Mux(Not(c),t,f)` → swap → sel konstanta) ikut
ter-simplify.

Contoh komposisi:

```rust
pipeline.add(ConstantPropagation);
pipeline.add(DeadLogicElimination);
pipeline.add(BooleanSimplify);
pipeline.add(MuxOptimize);
pipeline.add(ArithmeticOptimize);
pipeline.add(RegisterOptimize);
pipeline.add(TechMap);
```

### 4.2 Preset

| Preset | Output mapping | Tujuan |
|--------|----------------|--------|
| `generic` | `AND OR XOR MUX ADD REGISTER...` | debugging compiler / DAG bersih |
| `fpga` | `LUT6 FF BRAM DSP48 IO` (Xilinx 7-series style) | `maria synth --preset fpga` |
| `asic` | cell library (`NAND2 NOR2 INV AOI OAI MUX DFF BUF`) | `maria synth --preset asic --lib sky130.lib` |
| `custom` | library hardware sendiri (AETHERX dll) | `--lib` + `--tech-custom` |

Pemilihan preset di CLI: `maria synth --preset fpga` (default) /
`--preset generic` / `--preset asic --lib my.lib`.

---

## 5. Pipeline Synthesis (S0–S16)

```text
S0  Parse                    (reuse)          S9  FSM optimization
S1  Elaborate → IrDesign     (reuse)          S10 Register optimization
S2  RTL normalization                          S11 Structural optimization
S3  Lower RTL → SIR          (maria-sir)      S12 Technology mapping
S4  Constant propagation                      S13 Netlist optimization
S5  Dead logic elimination                    S14 Timing analysis (STA)
S6  Boolean optimization                      S15 Area estimation
S7  Arithmetic optimization                   S16 Emit netlist
S8  Mux optimization
```

Setiap pass stateless-deterministik: `SIR → SIR` (S4–S11), `SIR → Netlist`
(S12–S13), `Netlist → laporan` (S14–S16). Hasil tiap pass dapat di-dump untuk
debug: `maria synth --dump-sir` (sebelum opt), `--dump-sir-opt` (setelah opt),
`--dump-netlist`.

---

## 6. Optimisasi yang Wajib (S4–S11)

Minimal yang harus dimiliki Maria:

| Optimasi | Contoh | Hasil |
|----------|--------|-------|
| **Constant propagation** | `a & 0` | `0` |
| **Identity** | `a & 1`, `a \| 0`, `a + 0`, `a * 1` | `a` |
| **Double inversion** | `~~a` | `a` |
| **Dead logic elimination** | node tanpa load (kecuali port/`(* keep *)`) | dihapus + fanout |
| **CSE** | `x = a&b; y = a&b` | `t = a&b; x=t; y=t` (1 node) |
| **MUX simplification** | `mux(1, a, b)`, `mux(c, a, a)` | `a` |
| **Arithmetic simplify** | `a * 2^k`, `a << k`, `x ? x : y` | `a << k`, `x \| y` |

---

## 7. Width Optimization (Bit-Width Analysis)

Penting untuk hardware: `logic [2:0] x; assign x = a + b;` (a,b 32-bit) —
tidak perlu adder 32-bit jika analisis range menunjukkan hanya 3 bit.

```rust
pub struct BitWidthInfo {
    pub known_bits: ...,
    pub unknown_bits: ...,
    pub minimum_width: usize,
    pub maximum_width: usize,
    pub signedness: bool,
}
```

SIR menyimpan lebar per node/register (`SirNode.width`, `SirRegister.width`).
Optimizer mengecilkan datapath: operasi yang hanya memengaruhi bit
`[required_width-1:0]` di-truncate, input upper-bit yang tidak memengaruhi
output di-prune (bit-tracking dua arah). Hasilnya diukur di report area
("bits saved").

---

## 8. FSM Synthesis

RTL state machine (`IDLE → READ → WRITE → IDLE`) dikenali secara eksplisit:

```text
FSM
├── states        (IDLE, READ, WRITE)
├── transitions   (kondisi antar state)
├── inputs
└── outputs       (Moore/Mealy)
```

Encoding dapat dipilih: **binary** (`IDLE=00, READ=01, WRITE=10`),
**one-hot** (`001, 010, 100`), **gray**, atau **custom**. Detection: signal
yang di-assign `case` dengan label konstanta & hanya di proses sequential
(sekarang: `fsm_hint` di S1; ekstraksi penuh menyusul).

---

## 9. Clock, Reset & CDC

Clock/reset BUKAN signal biasa:

```rust
pub struct ClockDomain {
    pub clock: ValueId,
    pub frequency: Option<f64>,
    pub phase: f64,
    pub reset: Option<ValueId>,
    pub registers: Vec<RegisterId>,
    pub crossings: Vec<(RegisterId, RegisterId)>,
}
```

- Per `SirRegister.clock` → group ke `ClockDomain`.
- **CDC analysis** mendeteksi crossing antar domain (`clk_100 → clk_50`) dan
  memberi warning bila tidak ada sinkronisasi (`--cdc-report` sudah ada di
  simulator; synthesis menyediakan grafik domain dari SIR).

---

## 10. Constraint `.mcs` & Timing Engine

### 10.1 `.mcs` — Maria Constraint Specification

```text
clock clk {
    period = 10ns;
}
input_delay 2ns;
output_delay 2ns;
max_fanout 32;

false_path {
    from = rst;
}
multicycle_path 2 {
    from = reg_a;
    to = reg_b;
}
```

Pipeline: `.mcs` → Constraint IR → Timing Engine. (Sebelumnya bernama `.mdc`
di draf v0.1; format `.mcs` diadopsi agar selaras dengan spesifikasi.)

### 10.2 Timing Engine

1. **Timing graph** — node = pin sel; edge = pin-delay + wire delay (route).
2. **Forward** — *arrival time* dari input port + FF clock-to-Q (rise/fall
   terpisah).
3. **Backward** — *required time* dari output port + FF setup/hold.
4. **Slack = required − arrival**; **WNS** & **TNS** (terminologi Vivado).
5. **Critical path** — segmen terpanjang:

```text
Critical Path:  clk → FF1 → ADD → MUX → NAND → FF2
delay = 8.7 ns   constraint = 10 ns   slack = +1.3 ns   (MEET)
delay = 12.4 ns                                           → TIMING VIOLATION
```

---

## 11. Model Netlist (crate `maria-netlist`)

Netlist hasil technology mapping (dari SIR, bukan dari AST):

```rust
pub struct Netlist {
    pub modules: Vec<NetlistModule>,
    pub cells: Vec<CellInstance>,
    pub nets: Vec<Net>,
}

pub struct CellInstance {
    pub id: CellInstanceId,
    pub cell_type: CellTypeId,
    pub connections: Vec<PinConnection>,
}

pub struct Net {
    pub id: NetId,
    pub driver: Option<PinRef>,   // 1 driver
    pub sinks: Vec<PinRef>,       // N loads
}
```

Contoh:

```text
U1 NAND2_X1   U2 INV_X1   U3 DFF_X1

NET0  A → U1.A   B → U1.B
NET1  U1.Y → U2.A
NET2  U2.Y → U3.D
```

Aturan: (1) **1 driver, N loads** — acyclic DAG; (2) lebar final; (3)
traceability `src_map` → `file:line:col` RTL. Serialisasi `.mvnet` (teks
deterministik, bisa di-diff/commit) + `netlist.v` (Verilog structural) +
`netlist.json`.

Detil implementasi (fase 3):

- **Konstanta = wire + `assign`** (bukan literal di koneksi pin) — engine
  Maria tidak mendukung literal `8'h0` pada port instance; net ber-const
  lebih netlist-like dan portabel lintas tool.
- **Koneksi port COMma-separated** `(.a(net), .b(net))` — fase 3 menemukan
  bug engine: koneksi space-separated `.a(net) .b(net)` tidak pernah di-parse
  `parse_instance` → instance tanpa koneksi → `always_ff` di module sel tak
  ter-resolve (FF tidak berdetak, output z). Fix root di
  `crates/maria-parser/src/instance.rs` (branch `.name(expr)` setelah nama
  instance + `.*` wildcard + shorthand `.port`) + regresi
  `test_space_separated_instance_connections`; emitter memakai bentuk
  comma-separated yang paling umum.
- **FF parameterized** `#(.W(n), .RST(v))` — `module_key` menyandikan
  spesifikasi reset (nilai/polarity/async) agar dua FF dgn reset berbeda
  tidak bertabrakan pada nama modul; `W` = lebar operand (`in_width`),
  output compare/reduce 1-bit. Sel `Slice` juga parameterized (lebar operand
  penuh — potongan `a[msb:lsb]` dari nilai lebar apa pun).

---

## 12. Technology Library (crate `maria-tech`)

Maria tidak menganggap semua hardware punya gate yang sama:

```text
Technology
├── cells        (NAND2_X1, area=1.2, pins A/B/Y)
├── pins
├── timing       (A→Y rise/fall, setup/hold)
├── area
├── power
└── constraints
```

Sumber: **Liberty (`.lib`)** → parser subset → *Maria Technology Database*
(biner content-addressed, pola MICD):

```text
.target/technology/
├── cells.mdb
├── timing.mdb
├── area.mdb
└── power.mdb
```

Back-end bawaan: `generic`, `fpga` (fpga-x7: LUT6/FF/CARRY4/BRAM36/DSP48/IO/
BUFG), `asic` (cell library dari `.lib`), `custom`. P&R ASIC → external tool
(konservatif, ditolak `mimpl` dengan pesan jelas).

---

## 13. Report

Output `maria synth` (ke `build/synth/`):

| File | Isi |
|------|-----|
| `netlist.v` | Verilog structural |
| `netlist.json` | netlist JSON (GUI/CI) |
| `synthesis.rpt` | ringkasan sintesizability + utilisasi |
| `area.rpt` | estimasi area (gate count / LUT+FF) |
| `timing.rpt` | WNS/TNS + critical path |
| `hierarchy.rpt` | hierarki modul |

```text
Maria Synthesis Report
=======================
Modules:           1,582
Registers:         84,231
Combinational:    391,822
Mapped cells:      NAND2 128,221 · NOR2 31,442 · INV 64,231 · MUX 22,981 · DFF 84,231
Area:              124,832.4
Critical path:     8.72 ns
Target:            10.00 ns
Slack:             +1.28 ns
```

---

## 14. MICD Extension + Incremental Synthesis

### 14.1 MICD diperluas (bukan database synthesis terpisah)

```text
.target/database/
├── metadata.mdb  source.mdb  symbol.mdb  ast.mdb  types.mdb  graph.mdb
├── elaboration.mdb
├── sir.mdb       synthesis.mdb  netlist.mdb
├── technology.mdb  timing.mdb  constraints.mdb
├── diagnostics.mdb
└── cache/  lexer/ parser/ semantic/ elaboration/ synthesis/ mapping/ timing/
```

### 14.2 Incremental synthesis

Bukan sekadar incremental compilation — **synthesis ikut incremental**:

```text
A ──┐
B ──┤
    ├── TOP        user mengubah module C
C ──┤
D ──┘
```

```text
C (berubah) → affected nodes → affected modules → affected SIR
            → affected synthesis region → affected netlist
```

Modul tak berubah → SIR/netlist-nya di-cache content-addressed (hash
`IrDesign`/`SirModule`) → tidak di-lower/opt/map ulang.

---

## 15. CLI `maria synth`

Nama utama **`synth`** (nama lama `msynth` tetap berfungsi sebagai alias).

```text
maria synth --top opentitan_top
maria synth --preset generic | fpga | asic | custom
maria synth --preset asic --lib sky130.lib
maria synth --constraint chip.mcs
maria synth counter.sv --top counter --emit-mvnet --report-util
maria synth counter.sv --dump-sir            # SIR sebelum optimasi
maria synth counter.sv --dump-sir-opt        # SIR setelah optimasi
maria synth counter.sv --dump-netlist
maria synth --check-only rtl/                # hanya SYN subset check
maria synth counter.sv --opt speed --lut-merge on
maria synth counter.sv --equiv 20            # equivalence check Z3 (S6)
```

| Flag | Fungsi |
|------|--------|
| `--preset` | generic / fpga (default) / asic / custom |
| `--lib` | file `.lib` (wajib untuk preset asic) |
| `--constraint` | file `.mcs` |
| `--check-only` | analisis sintesizability SYN-1..9 tanpa netlist |
| `--dump-sir` / `--dump-sir-opt` / `--dump-netlist` | debugging per tahap |
| `--emit-mvnet` / `--emit-verilog` | netlist `.mvnet` / `netlist.v` |
| `--report-util` / `--report-json` | report utilisasi |
| `--equiv [BOUND]` | equivalence check RTL↔netlist via Z3 (`maria-formal`) |
| `--opt area\|speed` | tujuan optimasi |

Tool lain: `maria mimpl` (P&R FPGA) dan `maria msta` (STA) — membaca `.mvnet`
hasil `synth`.

---

## 16. `.mv` sebagai Source Language

`.mv` (Maria HDL) adalah source language utama; SystemVerilog tetap sebagai
**format interop**:

```text
design.mv ──► Maria Compiler ──► SIR ──► Synthesis
     │
     └──────────► .sv  (compatibility backend, `mgen` — tetap ada)
```

Catatan penting: saat ini `.mv` di-transpile **in-memory** ke AST SV lalu
di-elaborasi menjadi `IrDesign` — tanpa file `.sv` di disk. Karena SIR dibangun
dari `IrDesign`, jalur `.mv → SIR` **sudah efektif tercapai** tanpa melewati
file `.sv`. `.sv` hanya format output/kompatibilitas.

---

## 17. GUI (maria-gui)

Workspace synthesis baru:

```text
Maria
├── Project  Sources  Hierarchy  Diagnostics  Simulation
├── Synthesis
│   ├── Overview   (target, technology, cells, registers, area)
│   ├── RTL        (SIR)   Optimization   Technology
│   ├── Timing     (WNS/TNS bar, critical path)   Area
│   └── Netlist    (schematic)
└── MICD
```

**Critical Path Graph yang dapat diklik:**

```text
FF128 → U2381 ADD → U8821 MUX → U9912 NAND → FF991
```

Render via `netlist.json` + panel `Schematic` (reuse canvas dependency.rs).

---

## 18. Struktur Crate & File (1 file = 1 tanggung jawab)

```text
crates/maria-sir/          ← IR tengah synthesis (SUDAH ADA, fase 1)
  src/sir.rs               data model (SirModule/SirNode/SirRegister)
  src/lower.rs             lowering IrDesign → SirModule
  src/print.rs             dump teks (--dump-sir)

crates/maria-synth/        ← pass manager + optimasi + mapping
  src/pass.rs              trait SynthPass + SynthPipeline + preset
  src/opt/                 const_prop.rs · dce.rs · cse.rs · boolean.rs
                           mux.rs · arith.rs · width.rs · fsm.rs
  src/subset.rs            analisis sintesizability (SYN-1..9) [S1, ada]
  src/techmap.rs           dispatcher mapping (device abstraction)

crates/maria-netlist/      ← netlist hasil mapping (dipisah dari maria-synth)
  src/net.rs  cell.rs  pin.rs  wire.rs  graph.rs  emit.rs

crates/maria-tech/         ← device abstraction
  src/generic.rs  fpga.rs  asic.rs  liberty.rs  timing.rs  arch.rs

crates/maria-timing/       ← STA + .mcs (bisa digabung ke maria-impl dulu)
crates/maria-area/         ← estimasi area
crates/maria-backend/      ← emisi netlist.v / netlist.json / edif

crates/maria-impl/         ← P&R + STA tools (mimpl/msta)
crates/maria-tools/        ← CLI synth.rs (tool `synth`)
```

> Pemisahan dilakukan bertahap agar tidak menunda nilai: `maria-sir` sudah
> menjadi crate terpisah (fase 1). `maria-netlist`/`maria-tech` dipisah dari
> `maria-synth` saat phase 3/4 (SYNTHESIS.md v0.2 — lihat roadmap §19).

---

## 19. Roadmap Bertahap

Mengikuti urutan implementasi (fondasi IR benar dulu — jangan langsung
technology mapping ASIC):

| Phase | Cakupan | Deliverable | Verifikasi |
|-------|---------|-------------|------------|
| **1** ✅ | RTL → SIR | `maria-sir` (sir.rs/lower.rs/print.rs) + `--dump-sir` | `counter.sv` → SIR node ADD/EQ/MUX + register d/q/clk/rst benar; unit test 9 |
| **2** ✅ | SIR optimizer | `pass.rs` (SynthPass trait + SynthPipeline + preset + fixed-point) + `opt/` (const_fold, arith, mux, cse, dce) + `--dump-sir-opt`/`--preset` | `counter.sv` 7→5 node (CONCAT fold, NOT push-through-MUX); `alu_opt.sv` `(a&0)|(b&FF)|~~a → a\|b`, `(a+0)+(b*4) → a+(b<<2)`; unit test 29 + 9 maria-sir; full workspace 34 suite EXIT=0 |
| **3** ✅ | SIR → generic netlist | `maria-netlist` crate (net.rs/cell.rs/lower.rs/graph.rs/emit.rs/json.rs) + emit `.mvnet`/`netlist.v`/`netlist.json` + `--dump-netlist`/`--emit-netlist` | **sim netlist = sim RTL**: counter netlist → `TB_COUNT 10` (sama dgn RTL), alu_opt netlist → `y=7 z=19`; DAG 1-driver/N-load + deterministik; unit test 11 + regresi space-separated di maria-tests; full workspace 36 suite EXIT=0 |
| **4** | generic tech mapping | LUT cut (n≤6 → LUT6 init), carry chain, AIG dekomposisi | `alu.sv` → LUT count sesuai ekspektasi |
| **5** | timing + area | `maria-timing`/`maria-area` + constraint `.mcs` | WNS/TNS/critical path benar (path manual dihitung ulang) |
| **6** | Liberty (`.lib`) | parser subset → `maria-tech/liberty.rs` → `.mdb` | unit test parser .lib (area/delay) |
| **7** | ASIC mapping | boolean → pohon cell 2-input (INV/NAND/NOR/XOR/MUX) | `--preset asic --lib sky130.lib` → netlist cell |
| **8** | FPGA mapping | LUT/FF/BRAM/DSP inference penuh + P&R (`mimpl`) + STA (`msta`) | `mimpl`/`msta` end-to-end; `msynth --equiv` Z3 pass |
| **9** | incremental synthesis | MICD `sir.mdb`/`netlist.mdb` + dependency graph → affected region | ubah 1 modul → hanya region itu di-synth ulang |

**Status:** Phase 1 (RTL→SIR), Phase 2 (SIR optimizer), & Phase 3 (SIR →
generic netlist) selesai — `maria-sir` + `maria-synth` (pass manager + 5 pass
optimizer + `--dump-sir-opt`/`--preset`) + `maria-netlist` (lowering SIR →
netlist 1-driver/N-load + `--dump-netlist`/`--emit-netlist`). Loop verifikasi
tertutup: netlist yang di-emit **disimulasikan engine Maria** dan hasilnya
sama dengan sim RTL (kriteria fase 3). Fondasi S1 sebelumnya (SYN check +
netlist pra-map + `.mvnet` + report utilisasi) tetap ada dan tidak
di-refactor.

**Exit criteria per phase:** `cargo test --workspace` hijau + e2e contoh di
`examples/synth/` (SIR/netlist/report di-commit sebagai golden).

---

## 20. Contoh End-to-End

`examples/synth/counter.sv` (counter 8-bit, SYNTHESIS.md):

```systemverilog
module counter #(parameter WIDTH = 8)(
    input  logic clk, rst_n, enable,
    output logic [WIDTH-1:0] count
);
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n)        count <= '0;
        else if (enable)   count <= (count == 99) ? '0 : count + 1;
    end
endmodule
```

```shell
# 1. Synthesis + dump SIR
maria synth counter.sv --top counter --dump-sir --emit-mvnet --report-util
#    → SIR node ADD/EQ/MUX + register; counter.mvnet; util report

# 2. Constraint
cat > counter.mcs <<'EOF'
clock clk { period = 10ns; }
input_delay 2ns;
output_delay 2ns;
EOF

# 3. Place & Route + 4. STA
maria mimpl counter.mvnet --constraint counter.mcs --seed 7 --report-timing
maria msta counter.routed.mvnet --constraint counter.mcs --report-timing counter.timing.rpt

# 5. Gate-level simulation (timing) — engine Maria sendiri
maria sim counter.synth.v --top counter --sdf counter.synth.sdf -T 1000

# 6. Equivalence check (Z3) — RTL vs netlist hasil synth
maria synth counter.sv --top counter --equiv 20
```

---

## 21. Keterbatasan & Pekerjaan yang Ditunda (jujur)

1. **Bukan fab-grade QoR** — optimasi/STA sederhana dibanding Vivado/DC.
   Tujuannya: workflow benar, report familier, loop sim↔synth tertutup.
2. **P&R ASIC tidak disediakan** — ASIC sampai netlist + STA (library-based).
3. **Subset sintesizability** — konstruk dinamis (queue, dynamic array, class,
   assertion temporal di DUT) ditolak di SYN check.
4. **Multi-clock & CDC** — STA mendukung banyak clock; analisis CDC otomatis
   menyusul (sudah ada `--cdc-report` terpisah di simulator).
5. **X-propagation pasca-synthesis** — gate-level sim memakai engine yang
   sama; X/Z di LUT via init-table dengan xprop (reuse).
6. **Primitif gate SV** — parser perlu menerima `and/or/not/xor/buf` + sel
   library (`LUT6/FDRE/...`) sebagai modul fallback saat sim netlist (S6).
7. **Lowering SIR fase 1** — `iff` kompleks (bukan signal tunggal), part-select
   dinamis, `**`, `Dist`, `case inside` (label rentang), dan `inout` (tristate
   diperlakukan sebagai input sementara) dicatat di `LowerResult.skipped`
   (bukan disalah-lower); dukungan penuh menyusul.

---

## 22. Ringkasan

Desain ini memberi Maria **flow synthesis lengkap ala Vivado** dengan
pendekatan yang hemat dan benar-arah:

- **100% reuse elaborator** (`IrDesign`) — width/clock/reset sudah final.
- **SIR node-based** (`maria-sir`) sebagai IR tengah — fondasi yang memungkinkan
  optimasi serius tanpa melawan syntax (mengikuti pola Yosys/ABC).
- **Pass manager + preset** — pipeline terkomposisi, bukan hardcode.
- **Device abstraction** (`maria-tech`) — Generic/FPGA/ASIC/Custom.
- Netlist deterministik ber-traceability (`.mvnet`/`netlist.v`), laporan
  utilisasi & timing ala Vivado, **loop verifikasi tertutup** (sim + Z3 equiv +
  STA).
- **MICD diperluas** untuk incremental synthesis (SIR/netlist/tech di-cache
  per modul).

Fase 1 (RTL→SIR) selesai; phase 2–9 mengikuti roadmap §19 — menutup celah
COMP-10 dan ENT-33 dari AUDIT.md.

*Draf desain v0.2 — mengadopsi arsitektur SIR + pass manager + preset.*
