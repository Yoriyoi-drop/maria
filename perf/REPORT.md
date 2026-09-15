# PERF — Reproducer O(n²) `find_signal` + Fix

Demo `.sv`: `perf/find_signal_stress.sv` (hierarchal access) vs
`perf/find_signal_control.sv` (local array, kontrol linear).

## Bottleneck

`SimulationEngine::find_signal` (crates/maria-simulator/src/simulator/engine/eval/ast.rs)
melakukan **linear scan** `design.top.signals.iter().position(..)` O(S) PER
evaluasi `IrLValue::HierRefIndex` / `IrExpr::HierRef` (akses hierarkis seperti
`sif_late.arr[k]`), dengan `hier_signal_map` (HashMap O(1)) hanya sebagai
fallback SETELAH scan. Desain dengan S ≈ N·P sinyal flat dan M ≈ N akses
hierarkis per cycle → kerja per cycle O(M·S) = O(N²).

## Reproducer

- leaf module: 16 register (`always_ff`), `generate` N instance → S = 16N+2
  sinyal flat.
- Interface `bus_if #(.DN(N))` instance `sif_late` DIDEKLARASIKAN setelah
  generate → posisi paling akhir di flat list → scan membayar O(S) penuh
  (early-exit tidak mungkin).
- Top `initial`: per cycle loop k=0..N-1 `sif_late.arr[k] = sif_late.arr[k] + 1`
  → 2-3× find_signal per akses × N akses × T cycle.
- Kontrol: workload identik tapi `larr[k]` local → resolve waktu elab, O(1).

Jalankan:
```shell
maria perf/find_signal_stress.sv  -T 10 -D NVAL=500
maria perf/find_signal_control.sv -T 10 -D NVAL=500
```

## Baseline (sebelum fix, release, T=10)

### Stress (hierarkis — kena find_signal)
| N | S (flat signal) | wall (s) | scaling |
|---|-----------------|----------|---------|
| 250 | 4002 | 0.82 | — |
| 500 | 8002 | 1.52 | ×1.85 |
| 1000 | 16002 | 6.05 | ×3.98 |
| 2000 | 32002 | 20.85 | ×3.45 |

Scaling N×2 → wall ×~3.5-4 → **kuadratik**.

### Control (local array — O(1)/access)
| N | wall (s) | scaling |
|---|----------|---------|
| 1000 | 2.32 | — |
| 2000 | 3.31 | ×1.43 |
| 4000 | 7.32 | ×2.21 |

Scaling N×2 → wall ×~2 → **linear**.

Rasio STRESS/CONTROL: N=1000 → 2.6×, N=2000 → 6.3× → divergen kuadratik.

## Fix (PERF-16)

`SimulationEngine` + field `signal_lookup: OnceLock<HashMap<Symbol, usize>>`
(di `engine/mod.rs`, init di `new_with_limit` `engine/core.rs`). `find_signal`
build map SEKALI (lazy): `hier_signal_map` + semua `top.signals` name → index.
Lookup jadi O(1). Aman: `top.signals`/`hier_signal_map` statis setelah elaborasi
(tidak ada push ke `top.signals` di runtime simulator). Konflik key → prefer
entry hier_signal_map (alias hasil elaborator).

## After fix (release, T=10)

### Stress (hierarkis)
| N | wall before (s) | wall after (s) | speedup | scaling after |
|---|-----------------|----------------|---------|---------------|
| 250 | 0.82 | 1.07 | 0.8× | — |
| 500 | 1.52 | 1.49 | 1.0× | — |
| 1000 | 6.05 | 2.11 | **2.9×** | ×1.42 |
| 2000 | 20.85 | 3.54 | **5.9×** | ×1.68 |

### Control (local array)
| N | wall before (s) | wall after (s) |
|---|-----------------|----------------|
| 1000 | 2.32 | 2.12 |
| 2000 | 3.31 | 3.32 |
| 4000 | 7.32 | 7.70 |

Control tidak berubah (tidak menyentuh find_signal) → fix tepat sasaran.

### Kesimpulan
- Scaling stress after-fix N×2 → wall ×~1.4-1.7 → **linear** (bukan ×3.5-4 lagi).
- Stress N=2000: 20.85 s → 3.54 s (5.9×), kini setara control (3.32 s) —
  biaya find_signal O(N²) hilang, sisa pekerjaan = parse/elab/sim linear.