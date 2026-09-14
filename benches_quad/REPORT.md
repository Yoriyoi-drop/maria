# Q2 — Audit O(n²) mendalam + Reproducer SystemVerilog

Audit O(n²)-hunting terhadap pipeline maria (preprocessor → lexer → parser →
AST → elaborator → IR → simulation engine → VCD). Semua reproducer `.sv`
ditulis sendiri di direktori ini (bukan memakai file project).

## Ringkasan eksekutif

| # | Kandidat | Mekanisme | Status | Impact |
|---|----------|-----------|--------|--------|
| 1 | `find_signal()` scan linear per akses hierarkis | O(S) per `IrExpr::HierRef`/`IrLValue::HierRef[Index]`, tanpa cache | **CONFIRMED O(n²)** | **HIGH** (hot path runtime) |
| 2 | Import prepass `module.items.iter().any(...)` per nama | O(M·P²) scan + O(M·P) clone | LIKELY | LOW |
| 3 | `commit_changes()` scan semua signal per delta | O(S) per delta | FALSE POSITIVE (total O(S·T) linear) | — |
| 4 | Inline function call | `HashMap` lookup O(1) | FALSE POSITIVE | — |
| 5 | Resolusi module per instance | `module_idx: HashMap` (sudah di-fix commit 7a74d68) | FALSE POSITIVE | — |
| 6 | VCD init `code_for_signal` | HashMap `code_by_key` O(1) | FALSE POSITIVE | — |
| 7 | Parser name sets | `HashSet<Symbol>` | FALSE POSITIVE | — |
| 8 | MICD `affected()` | BFS + visited set O(V+E) | FALSE POSITIVE | — |

---

```
==================================================
O(n²) FINDING #1  —  CONFIRMED
==================================================

Location:
crates/maria-simulator/src/simulator/engine/eval/ast.rs:1094

Function:
pub(crate) fn find_signal(&self, name: &str) -> Option<usize>
    → design.top.signals.iter().position(|s| s.name == name)   // scan O(S)
    → .or_else(hier_signal_map.get(...))                        // fallback SETELAH scan

Call path:
IrStmt block/event loop (per cycle, per akses hierarkis):
  scheduler/block.rs:239,262,373,385  → signal_id_from_lvalue (event.rs:998)
      → IrLValue::HierRefIndex/HierRef → find_signal()          [1x scan]
  eval/lvalue.rs:527 (write_lvalue HierRefIndex) → find_signal() [1x scan]
  eval/lvalue.rs:602 (get_lvalue_width HierRefIndex) → find_signal() [1x scan]
  eval/expr.rs:2066 (IrExpr::HierRef read) → find_signal()      [1x scan]
  eval/ast.rs:67  (AST method/randomize Ident) → find_signal()  [1x scan]

Input N:
N = jumlah sinyal ter-flatten (design.top.signals.len(), S)
N = jumlah akses hierarkis per cycle (M)
Design: S ≈ 16·N + 8 (leaf punya 16 register), M = N akses `bus.arr[k]`
per cycle → kerja/cycle = M × O(S) = O(N²). Diukur: S(250)=4008, S(1000)=16008.

Candidate algorithm:
Resolver nama hierarkis runtime = linear scan seluruh flattened signal list
per evaluasi, tanpa cache/memoization, dan hier_signal_map (HashMap, O(1))
hanya di-fallback SETELAH scan selesai — jadi biaya scan selalu dibayar
penuh walau nama bisa di-resolve via map.

Why it may be O(n²):
Setiap evaluasi membayar O(S). Simulasi dengan M akses hierarkis per cycle
selama T cycle → total O(M · S · T). Untuk desain realistik (TB/scoreboard
mengakses M signal DUT per clock) dengan S ∝ M → O(N²) per cycle.

SystemVerilog reproducer:
benches_quad/q2_find_signal_stress.sv        (STRESS: akses hierarkis)
benches_quad/q2_find_signal_control.sv       (CONTROL: akses lokal array)

SV workload design:
- leaf module: 16 register, always_ff per clk (eval linear, pekerjaan dasar).
- generate loop: N leaf → S = 16N+8 sinyal flat.
- interface bus_if #(.DN) dengan unpacked array arr[0:DN-1], instance
  `sif_late` DIDEKLARASIKAN SETELAH generate → posisi di flat list = akhir
  vector → scan find_signal() membayar O(S) penuh per akses (early-exit
  tidak mungkin). Realistis: bus kedua di TB yang dipakai scoreboard.
- top initial: per cycle, loop k=0..N-1 menjalankan `sif_late.arr[k] =
  sif_late.arr[k] + 1` → HierRefIndex write (2-3x find_signal) + HierRef
  read (1x find_signal) per k → M=N akses/cycle.
- Size dikontrol via `-D NVAL=N` (parameter = preprocessor macro, satu knob).

How reproducer triggers the path:
setiap `sif_late.arr[k]` → IrLValue::HierRefIndex("sif_late.arr", k) →
write_lvalue → find_signal("sif_late.arr") — scan 16007 elemen di N=1000.
Terbukti aktif: env MARIA_DBG_HIERWR mencetak [DBG-HIERWR] 'sif_late.arr'
idx=... (5000 hit di N=500×T=10).

Benchmark (release binary, 3 run, min, T=10 cycle, -T 1000):

STRESS (hier):
N       S            wall_ms    (wall-floor)
100     1608          666        ~266
250     4008         3452       ~3052
500     8008        11984      ~11584
1000   16008        36140      ~35740
2000   32008       165916     ~165516
(4000   64008       >10 min — tidak dijalankan, mencegah kelaparan RAM)

CONTROL (local array, O(1)/akses):
N       wall_ms
100      445
500     1647
1000    3726
2000    7203
4000   14935

Scaling:
- CONTROL: tiap N×2 → runtime ×2.0-2.3 → konsisten LINEAR.
- STRESS:  tiap N×2 → runtime ×3.1-4.6 (500→1000 ×3.1; 1000→2000 ×4.6)
  → konsisten O(N²). Rasio STRESS/CONTROL: N=500 → 7.3×, N=1000 → 9.7×,
  N=2000 → 23× → divergen secara kuadratik. N=100 margin kecil karena
  floor startup (~400ms) mendominasi.
- Per-fase (N=500): preproc 0.61µs, lexer 175µs, parser 233µs, elab 50ms,
  sim ≈ 5.5s → bottleneck murni di runtime engine (find_signal).

Control result:
q2_find_signal_control.sv: workload identik (jumlah akses, jumlah leaf,
cycle sama) tapi akses memakai local unpacked array (IrLValue::ArrayIndex,
id di-resolve saat elaborasi) → linear. Perbedaan scaling = biaya find_signal.

Stress result:
q2_find_signal_stress.sv: 166 s di N=2000 (23× control) — kuadratik.

Theoretical complexity:
O(M · S · T) dengan S = jumlah sinyal flat, M = akses hier per cycle.
S ∝ M ∝ N → O(N² · T) per run. Setiap akses ~4x scan O(S).

Observed behavior:
runtime N×2 → ×3.1-4.6 (kuadratik), control ×2 (linear).

Verdict:
CONFIRMED

Confidence:
HIGH

Impact:
HIGH — hot path runtime; workload real (scoreboard/TB/interface multi-bus
dengan akses hierarkis per cycle) langsung membayar O(S) per akses.
Fix yang mungkin (di luar scope audit ini): cek hier_signal_map HashMap
DULU, atau cache (name→id) per process/cycle.

Catatan jalur paralel (relate): parallel.rs:398 resolve_hier_signal
melakukan 3 scan (exact/suffix/last-segment) per hier ref — keluarga yang
sama, aktif bila parallel evaluator dipakai.
```

---

```
==================================================
O(n²) FINDING #2  —  LIKELY (impact LOW)
==================================================

Location:
crates/maria-elaboration/src/elaborator/mod.rs:827-898 (pre-pass import)

Function:
for module in modules:                       // M module
  for (package, import_item) in imports:
    for name in pkg_items:                   // P symbol
      if !module.items.iter().any(Func(name))  // scan items yang TUMBUH
        module.items.push(clone)             // clone FunctionDecl

Call path:
elaborate() → pre-pass import → .any() scan per nama → push

Input N:
N = P jumlah fungsi package yang di-import (`import pkg::*`)
N = M jumlah module yang meng-import
N = jumlah item per module (tumbuh 1 setiap push)

Candidate algorithm:
`any()` di-scan pada module.items yang bertambah seiring push → per module
Σ items = P(P+1)/2 → total O(M · P²). Plus clone AST function per push
O(M · P). File sumber hanya berukuran O(M + P), tapi kerja = O(M·P²).

Why it may be O(n²):
M dan P keduanya ∝ N → O(N³) worst; M konstan, P ∝ N → O(N²).

SV reproducer:
benches_quad/q2_import_50.sv   (N=50)
benches_quad/q2_import_100.sv  (N=100)

Workload design:
package pkg_x berisi NVAL function (macro `` `define `` + token concat,
satu baris per fungsi); NVAL module `m0..m`N-1`` masing-masing
`import pkg_x::*` + satu pemanggilan fungsi; top meng-instansiasi semua
module (biar elaborated penuh). Per module: 100 nama di-import → 100× clone
FunctionDecl + scan items yang membesar.

Benchmark (elaboration phase, 3 run, min):
N        elab_ms        wall_ms
50       15.8           148
100      35.6           239
200      (tidak dibuat)

Scaling:
elab ×2.25 saat N×2 — superlinear tapi konstanta kecil; pada ukuran
realistis (P≈50-300) tetap sub-ms..ms.

Theoretical complexity:
O(M·P²) scan + O(M·P) clone.

Observed behavior:
pertumbuhan elab 2.25×/doubling pada N kecil — tidak bisa dipisahkan dari
bagian linear (collection package, inlining) pada skala ini; eksperimen
lebih besar tak dibuat karena impact rendah.

Verdict:
LIKELY (mekanisme terbukti di source; scaling lemah di skala kecil)

Confidence:
MEDIUM

Impact:
LOW — butuh package dgn ratusan fungsi di-import ::* oleh banyak module;
jarang di workload real.
```

---

```
==================================================
FALSE POSITIVES (dianalisis & dibuang)
==================================================

#3 commit_changes() state.rs:180 — O(S) per delta.
    Total O(S·D). Dengan D (delta) konstan per benchmark (T tetap), kerja
    ∝ S → LINEAR. Bukti empiris: q2_find_signal_control.sv (S=64008 di
    N=4000, delta tetap) scale ×2 per doubling → linier, bukan kuadratik.

#4 inline.rs replace_func_calls_in_expr_inner — funcs: HashMap<Symbol,
    FunctionDecl> → get() O(1) per call site. Inlining O(call sites × depth).

#5 Resolusi module per instance — elaborator/mod.rs:1152 module_idx
    HashMap di-build sekali; lookup O(1) (fix commit 7a74d68
    "fix O(n²) module_idx").

#6 VCD init code_for_signal vcd.rs:224 — code_by_key HashMap → O(1);
    init scope: HashMap + sort O(S log S).

#7 Parser name sets — class_names/typedef_names = HashSet<Symbol> O(1).

#8 MICD affected() — BFS + visited HashSet → O(V+E).
```

---

## Ringkasan final

```
TOTAL CANDIDATES:        8 (dianalisis penuh) + 1 jalur relate (parallel.rs)
CONFIRMED:               1  → find_signal() O(S) per akses hierarkis
LIKELY:                  1  → import prepass O(M·P²)
FALSE POSITIVE:          6
INCONCLUSIVE:            0

WORST COMPLEXITY FOUND:  O(N²) per cycle di simulation engine (find_signal)

MOST IMPORTANT HOT PATH:
IrStmt/event loop → IrLValue::HierRefIndex / IrExpr::HierRef
→ engine.find_signal() → design.top.signals.iter().position() (scan O(S))

BEST REPRODUCER:
benches_quad/q2_find_signal_stress.sv  (CONFIRMED, N=100..2000, ×4/doubling,
                                       166 s di N=2000 vs 7.2 s control)

MOST REALISTIC SV STRESS TEST:
q2_find_signal_stress.sv — generate-loop leaf 16-reg + interface bus
`bus_if#(.DN) sif_late` dideklarasikan setelah generate (posisi akhir flat
list), top per cycle menulis N elemen `sif_late.arr[k]` → interaksi
akses×resolusi nyata, bukan nested loop artifisial.
```

## Cara reproduksi cepat

```shell
# CONFIRMED #1 — stress (kuadratik)
./target/release/maria benches_quad/q2_find_signal_stress.sv -D NVAL=1000 -T 1000
# CONFIRMED #1 — control (linear, pembanding)
./target/release/maria benches_quad/q2_find_signal_control.sv -D NVAL=1000 -T 1000
# LIKELY #2 — import prepass
./target/release/maria benches_quad/q2_import_100.sv -T 1000
```

## Catatan ukuran presisi (µs vs ms)

- Fase cold-start preprocessor: ~0.6-105 µs; lexer: ~150 µs - 2 ms; parser:
  ~215-244 µs — sudah skala µs.
- Elaboration: ms (5 ms di N=100 → 50 ms di N=500 → tumbuh).
- Simulation: detik di N besar — itu kerja O(N²) yang diukur, tidak bisa µs.
- Wall time terbatas floor startup CLI/env/MICD ~150-490 ms (run terkecil
  N=20 = 261 ms). Untuk ukuran < 1 ms wall, perlu harness API
  (maria_api::simulate_str) yang melewati startup CLI — di luar scope ini.