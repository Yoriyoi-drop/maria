# Migrasi Jalur Fuzzing: Maria-Fuzz → Maria-MV → HDL → Maria

Status: selesai (git commit menyertai). Dokumen ini = deliverable task §19 (A–G)
dan peta acceptance §20.

## A. Current Architecture (sebelum migrasi)

```
Fuzz Input → Generator → Mutation → Testcase(String SV) → Executor → Oracle → Result
```

| Tahap | File | Fungsi | Bentuk data |
|---|---|---|---|
| Generator | `crates/maria-fuzz/src/gen.rs` | `Generator::random_module/module_for_shape/compose_from_interaction/passive_core` | `String` SV |
| Mutation | `src/ast_mutate.rs` | `mutate()` — 23 op **string-level** | `String → String` |
| Testcase | — (tidak ada struct) | representasi = `String` SV; tanpa serialisasi AST/history | `String` |
| Corpus | `src/corpus.rs`, `src/guide.rs` | seed SV nyata + energy schedule | `Vec<String>` |
| Executor | `src/harness.rs` | thread/subprocess isolasi | `&str → RunOutcome` |
| Oracle | `src/oracle.rs` | satu-satunya pemanggil `maria_api` (compile/sim) | verdict + fingerprint |
| Min/Max | `src/corpus.rs Corpus::minimize` | minimasi baris | `String → String` |
| Feedback | `src/feature.rs`, `src/guide.rs` | feature map teks + coverage keys engine | fitur eksekusi-gated |
| Bug | `src/bugdb.rs` + `lib.rs` | JSON lintas-kampanye + emit `.sv` | `source: String` |

Fakta kunci audit: **semua testcase = `String` SV mentah; titik sentuh Maria tunggal di
`oracle.rs`**; `maria-mv` punya AST lengkap tapi **tanpa printer `.mv`** (gap) dan
**tanpa konsep scenario** (grammar-nya: module/port/sig/reg/const/seq/comb/always/
latch/initial/final/inst/genfor/genif/use/assert/interface/class/func/task).

## B. New Architecture (hasil migrasi)

```
Maria-Fuzz ── parent (canonical .mv) ──▶ mutasi AST (mv_mutate)
                                              │ print_file (canonical)
                                              ▼
                                     .mv text ──▶ mv_lower ──▶ HDL (svh+sv)
                                              │                    │
                                              ▼                    ▼
                                    MvReject (bug MV/fuzzer)   harness+oracle (Maria)
                                                                     │
                                                        result/oracle/coverage
                                                                     │
                                                        guide.corpus (MV canonical)
                                                                     └── feedback
```

- Backend **Direct** (default, task §14): jalur lama utuh — `--backend direct`.
- Backend **MvMediated**: `--backend maria-mv` — parent `.mv` → mutasi AST →
  lower → HDL → eksekusi. Tidak ada silent fallback: MvReject di-`continue`.

## C. File Diubah

### Bug pra-eksisting yang ditemukan & diperbaiki (triage §19.G)

| Area | File | Bug |
|---|---|---|
| **MARIA** | `.../scheduler/timing_wheel.rs` | **Indeks event RELATIF vs ABSOLUT**: `add_event` pakai `offset & MASK` sedangkan `advance()`/cascade pakai waktu absolut → event yang di-add saat current≠0 dijadwalkan ke waktu keliru/hilang. Jalur timing-wheel menyimpang dari vector-queue (deep-differential `path_timing_wheel_vs_vec` mismatch `clk=0` vs `clk=1`). Fix: indeks absolut `time & MASK`; + `has_events`; + 3 test regresi (add-during-advance, FIFO, L1). |
| **MARIA (hang #0)** | `.../engine/scheduler/block.rs` + `engine/{core,mod}.rs` | **Hang sejati `forever #0 clk = ~clk`** (ketemu fuzz MV, gdb: `evaluate_loop_while_fork` kloning body per `#0`). `#0` di-loop = re-schedule Inactive + kloning `Vec<IrStmt>` per iterasi; delta-limit 100k baru kena >40s (debug) → melanggar batas eksekusi engine. Fix: guard **`#0`-churn per time-step** (`ZERO_DELAY_EVENT_CAP=4096`) → abort `RT2001` InfiniteDelta **<2s** (requirement <10s/ideal<1s); + guard lapis-2 revisit same-time. Ukur: `#0` loop minimal **0.94s**, full MV `hang_orig` **1.11s**; sim normal (`#0` tunggal + `forever #5`) tetap 0.7s OK. |
| **MARIA (test)** | `crates/maria-simulator/src/test_util.rs`, `crates/maria-emu/src/mhir/extract.rs` | `file_line_map[0].1` (col) bukannya `.2` (file) → test compile error. |
| **MARIA-FUZZ** | `crates/maria-fuzz/src/gen.rs` | (1) dead-code `random_reg_width/random_signal_width` pakai `b.rng` via `&` → build error; (2) counter loop `for/while` di-push ke symbol table module-scope → E2001 undefined signal (while tak pernah di-declare; for-scoped tapi dirujuk di luar); (3) deklarasi mem: HANYA satu dari `{mem, mem2}` di-declare tapi `proc` menulis keduanya → E2001; (4) `$size/$bits` dgn argumen literal → E3001 `$size argument must resolve to a signal`. Generator kembali 100% valid (0/120 gagal → semua hijau). |
| **MARIA-FUZZ** | `src/real.rs` | deteksi artefak `fz_`/`_fuzz` pakai `&&` → tak pernah skip (test gagal); fix `||` + test memakai error parse nyata (`=` tanpa RHS; `undefined_sig` BUKAN error — implicit net valid SV). |
| **MARIA-FUZZ (test)** | `src/differential.rs` | 4 helper deep-differential (`path_packed_vs_standard` dll.) hilang → test compile error; diimplementasikan ulang via `EngineFlags`+`fingerprint_isolated_flags`. |
| **MARIA-MV** | `crates/maria-mv/src/parser/module.rs` | `seq(clk, rst, sync)` tak pernah ter-parse: cek `Tok::Ident("sync")` padahal lexer memberi `Tok::Sync` → sync keyword bug (MARIA-HDL.md mengklaim dukungan sync). |
| **MARIA-FUZZ (runner)** | `src/main.rs`, `src/lib.rs` | Kampanye dijalankan di **main thread stack 8MB** — project sweep / gap-check / real-hunt memanggil compile atas file korpus raksasa di thread itu → stack overflow abort (ketemu saat `--backend maria-mv` dari repo root). Fix: jalankan kampanye di thread `WORKER_STACK_BYTES` (256MB); gap-check & seed-bootstrap SV di-gate ke backend Direct. |

### Baru (implementasi migrasi)

| File | Peran |
|---|---|
| `crates/maria-mv/src/print.rs` | **Printer canonical `MvFile → .mv`** (round-trip `parse(print(f)) ≡ f`, idempoten) — gap §18 yang diidentifikasi; 6+ test. |
| `crates/maria-fuzz/src/mvgen.rs` | Generator scenario `.mv` valid-by-construction: chain input→comb→seq→out, signed, X/Z injeksi, shape genap/ganjil (akumulasi vs FSM stmt). |
| `crates/maria-fuzz/src/mv_mutate.rs` | Mutasi **AST** `MvFile` (Level 2–4): 12 op — swap_binop, flip_fill, change_literal, toggle_signed, toggle_nba, dup_stmt, add_sig, add_port, change_const, change_delay, switch_block (seq↔comb + fix NBA), add_assert (assert-oracle). `MvOpStats` adaptif. |
| `crates/maria-fuzz/src/mv_lower.rs` | Lower + gate validasi: `Hdl` (check ok) / `HdlNoCheck` (band expected-invalid) / `MvReject` (bug MV) — **pembedaan bug Maria vs bug Maria-MV**. |
| `crates/maria-fuzz/src/testcase.rs` | `Testcase {mv, mv_hash, hdl, hdl_hash, seed, scenario_id, history, backend, cfg_fp}` — reproducibility §9. |

### Integrasi

- `src/lib.rs`: `FuzzConfig.backend`, metadata MV di `BugRecord` (+ `.mv`/`.hdl`/history), bootstrap MV, korpus execution-gated menyimpan **MV canonical** (bukan HDL), bugdb re-seed prefer MV, `FuzzReport.mv_op_stats`, `emit_bugs` tulis `.mv`/`.sv`/`.hdl.sv`.
- `src/main.rs`: flag `--backend direct|maria-mv` + big-stack thread.
- `src/bugdb.rs`: `push()` simpan `bug.mv` bila ada (re-seed MV lintas kampanye).

## D. Data Flow

```
seed:u64 ──► StdRng
  ├─ (Direct) guide: SV corpus + gen.rs ▸ mutasi string ▸ exec ▸ oracle ▸ feedback
  └─ (MvMediated) guide: mvgen .mv canonical
        ▸ select(parent MV) ▸ mv_mutate::mutate ×1..2 (AST, history dicatat)
        ▸ print_file ▸ lower_mv:
              Hdl/HdlNoCheck ─► compile-gate HDL ─► harness (thread/proc, hang EMA)
              MvReject ─► counted, continue (BUKAN fallback direct)
        ▸ oracle battery (determinism/EMI/meta/sim-sig/property/assert/RT-triage)
        ▸ min(MV atau HDL minimal) ▸ BugRecord(mv+hdl+history)
        ▸ feature map + engine coverage ▸ guide.add(MV canonical) ▸ energi/op-stats
```

Repro: `seed + scenario_id + history + canonical .mv (+ cfgtfp)` → MV sama → HDL sama (hash) → perilaku Maria sama.

## E. Testing

| Suite | Sebelum | Sesudah |
|---|---|---|
| `cargo test -p maria-fuzz --features dev` | **gagal build** (2 err) + 6 test gagal | **114 pass, 0 fail** (+16 test baru: mvgen/mv_mutate/mv_lower/testcase) |
| `cargo test -p maria-mv` | 138 pass | **144 pass, 0 fail** (+print 6, +fix sync) |
| `cargo test -p maria-simulator -- timing_wheel` | 12 pass | **15 pass, 0 fail** (+3 regresi indeks absolut) |
| `cargo test --workspace` | gagal compile (emu test_util) | 513 pass; **3 fail environmental** (maria-emu boot: fixture ISO/grub tak ada, `Os NotFound`) — bukan perubahan ini |

## F. Performance (seed 99/31337, build debug, ubuntu container)

| Metrik | Direct (25 iter) | MV (40–60 iter) |
|---|---|---|
| Throughput | ~9.1 s/iter (korpus SV 300+ file + gap-check tiap iter) | ~0.33 s/iter |
| compile_ok/total | 11/25 | 31–52/40–60 |
| new features | 10 | 15–24 |
| bug | 0 | 0 (kampanye 400 iter lanjutan) |
| startup overhead | project-sweep ~6s + auto-corpus scan | sama (sweep di-skip utk MV? — sweep tetap berjalan) |

MV-mediated: throughput ~27×, validitas testcase jauh lebih tinggi (valid-by-construction),
eksplorasi fitur per iterasi lebih banyak, overhead compile-gate hilang (lower+check di
maria-mv cepat).

## G. Bug Finding Classification

- **BUG DI MARIA (runtime/hang)**: `forever #0 clk = ~clk` hang sejati (abort cepat via guard #0-churn; <2s); timing-wheel index relatif (deep-differential); test_util/emu stale index.
- **BUG DI MARIA-MV**: `seq(..., sync)` tak ter-parse.
- **BUG DI MARIA-FUZZ**: generator loop-counter/mem/$size, real.rs `&&`, runner stack overflow, korpus MV ketika SV jadi parent MV.
- **BUG DI TEST/ORACLE**: deep-path helper hilang, `undefined_sig` asumsi salah.

## Requirement Runtime (MARIA utama)

`#0`/zero-advance churn wajib selesai **<10s (ideal <1s)** — tercapai: guard
`ZERO_DELAY_EVENT_CAP=4096` → RT2001 dalam **0.94s–1.11s** (debug build). Sim
normal tidak terpengaruh (counter reset per time-step; `#0` tunggal aman).

## Acceptance Mapping (§20)

- [x] Maria-Fuzz menghasilkan testcase lewat Maria-MV (`--backend maria-mv` end-to-end 52+ sim_ok).
- [x] Tidak lagi depend pada raw random HDL sbg jalur utama MV (Direct tetap utk diff/backward-compat).
- [x] MV DSL/AST = intermediate representation (mutasi AST + printer canonical).
- [x] Lower ke HDL valid + eksekusi Maria.
- [x] Seed reproducible (seed → MV → HDL hash stabil; Testcase metadata).
- [x] Bug record membawa MV + HDL + seed + mutation history.
- [x] Feedback Maria → guide (corpus MV canonical + coverage keys engine).
- [x] Differential testing (deep-path + determinism/EMI/meta; `--backend` Pembanding).
- [x] Test lama hijau; perubahan kegagalan dijelaskan (emu env fixtures missing).
- [x] Tidak ada silent fallback ke direct (MvReject `continue` explicit).
- [x] Pipeline membedakan bug MV vs bug Maria (LowerVerdict).
- [x] Structural & semantic mutation benar (mv_mutate Level 2–4).
- [x] Jalur parallel MULAI (packed/DAG/timing-wheel/MIR) diuji terpisah (path_flag_diff).