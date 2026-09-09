# Roadmap: Buru Lautan Bug Runtime & Simulator (IEEE 1800)

Status: **Ph-1 selesai** (gen.rs non-template, validitas ~99%, caret render fixed).
Tujuan: temukan BANYAK bug runtime/simulator maria — bukan cuma crash parser.

Perjanjian: bug = Panic / Hang sejati / sim_err pada input parse-clean /
mismatch differential. compile_err BUKAN bug (fitur LRM belum didukung).

## Ph-1 (selesai) — Generator & mutasi non-template
- [x] `gen.rs` ditulis ulang: builder rekursif (expr/stmt/proc/module) + `Bias`
      knob. Tidak ada string template per strategi; 13 strategi = 13 profil bias.
      Deterministik per stream RNG; output BEDA tiap panggilan (test diversity).
- [x] compile_err 32→~26 per kampanye; histogram generator 119/120 compile ok.
      Fix: replikasi `{N{...}}`, `$signed` (bukan `signed'`), $clog2/$bits
      operan-konstan, part-select OOB di-gate knob, `always_latch` tidak
      ref `rst_n` tanpa deklarasi, child+instance SELALU dipasangkan
      (top-ambiguity E3006), div-by-zero non-konstan (`a/0`) — runtime RT
      bukan constant-fold E9001.
- [x] `grammar.rs`: palet snippet +8 (casez/casex, fork/join_none, event-control,
      repeat/while, concat, ternary+#0, NBA) — mutasi & splice lebih dalam.
- [x] `ast_mutate.rs`: +4 mutasi semantik (op 19–22: over-shift, injeksi X/Z,
      tweak bound loop, part-select diperketat) → NUM_OPS 19→23.
- [x] `lib.rs`: gate validitas mutasi (retry 1×) — proporsi sim-reachable naik.
- [x] **Render caret fixed** (`maria-core/diagnostics/emitter.rs`): caret off-by-one
      di rich/plain/format_diagnostic + plain-mode alignment; 3 test baru.
- [x] **Duplikasi diagnostic fixed** (`src/main.rs`): error dicetak 2× (parser
      errors + top-level handler) → sekarang warning dicetak sekali di dalam,
      error SATU kali di top-level (line:col tidak ganda).
- [x] **`$fdisplay` newline fixed** (`maria-simulator/.../block_syscall.rs`):
      LRM `$fdisplay` = `$fwrite`+`\n` (maria sebelumnya concat tanpa newline —
      artefak differential trace; 2 handler di-patch).

## Ph-2 — Oracle & observability (prioritas tinggi)
- [x] **Differential vs tool referensi LRM** (`--ref-diff`): `reference_vs_ivl` —
      core pasif (`gen::passive_core`, bias race/XZ/sizing stress) + tb
      eksternal drive + banding trace `dtrace.txt` maria vs iverilog
      (LEMAH/no-FP: single-driver Same). **Temuan #1 tercatat**: 3-writer
      reset-race → maria deterministik vs iverilog X (parity gap VCS-class,
      teste regression `reference_vs_ivl_repro_minimized_race_finding`).
  satukan dengan verilator (proxy kedua, juga terpasang).
- [x] **Project-wide seed sweep** (v2.3): korpus nyata di-compile sebagai
      satu design → SEMUA error (parse+elab) file:line:col per kategori.
      Opentitan: 180 error tertangkap (Parse 168/hierarki 9/lain 3).
- [x] **Auto-fast pipeline** (v2.3): file >256KB otomatis `run_fast` —
      0.58s utk 21k baris (was 30–42s).
- [ ] **Trace/region oracle**: fingerprint BUKAN hanya final — rekam (waktu, delta,
      nilai) tiap signal tiap region (active/NBA/observed). Deteksi salah-urutan
      region yang pulih di akhir (determinism-check buta transien).
- [ ] **Ordering oracle**: dua proses yang menulis sinyal sama di delta sama —
      nilai akhir harus satu dari driver (race legal); tambahkan hook
      "writer terakhir = driver" sebagai invariant minimal maria.
- [ ] **Fault-sensitivity sweep** (faults.rs): ukur rasio op-swap TERLIHAT di
      fingerprint per konstruk baru gen.rs (ekspresi dalam, X/Z, signed).
      Palet oracle baru WAJIB sensitivity > 80% sebelum dipakai.
- [ ] **Event-consistent `$time` oracle**: proses yang `#N` — `$time` increment
      harus N×unit dari waktu sebelumnya; mismatch = scheduler bug.

## Ph-3 — Generasi terarah (bukan numpang valid)
- [ ] **Targeted sizing/race**: mode generator "adversarial module" — variasi
      operand width/signedness & multi-writer NBA per-konstruk (>10 tipe),
      tanpa mematikan validity (bootstrap 99%).
- [ ] **Generate/param/hierarchy dalam**: param override di child, nested
      gather/if-generate, part-selection dinamis `[idx-:w]` — area EL yang
      paling jarang di-stress.
- [ ] **Mem/array 2D+ & out-of-bounds index runtime** — evaluator mem.
      **DONE v2.3**: `gen.rs` `use_mem` — mem 1D/2D LIVE (tulis NBA + baca OOB),
      sahkan selengkapnya via kampanye panjang (belum divalidasi batch besar).
- [ ] **SVA/covergroup/clocking** — thread observed/reactive (jalan "sering
      zero-time region violation").

## Ph-4 — Korpus & minimisasi
- [ ] Korpus REAL lebih banyak (opentitan/cva6 sudah ada; tambah wobbling:
      riscv, corescore, opencores) → parent mutasi kayak fitur dalam.
- [ ] Minimizer: awetkan "hang" (minimizer kini bisa merusak seed hingga
      parse-error — lihat bug_0000_hang.sv arti); verifikasi ulang status
      SEBELUM tulis bug (konfirmasi: minimized HARUS reproduce).
- [ ] Hang classification: while/for tak-batas = testbench bug; bedakan dari
      hang ENGINE (parser/infinite selamanya) — minimasi hang harus tetap
      REPRODUCE via `run_isolated` langsung, bukan status lama.

## Ph-5 — Pipeline angka (ukur, jangan nebak)
- [ ] Per kampanye cetak: sim_ok / iters (reach), sim_err_clean, bugs, op_top,
      histogram error-code seed generated (tes `debug_error_code_histogram`).
- [ ] KPI: reach↑ (sim_ok/iter), FP oracle ↓, fault-observability ↑, dan
      bug sejati terbentuk (panic/RT-clean/diff) per jam komputasi.

## Catatan operasional
- 1 file = 1 tanggung jawab; edit manual (tanpa script mass-edit).
- `cargo test -p maria-fuzz --features dev` — 84 test; jalankan sebelum/ssdh
  tiap perubahan generator.
- Ambang hang HARUS > settle delta-storm engine (12s debug) — jangan turunkan
  untuk kecepatan.