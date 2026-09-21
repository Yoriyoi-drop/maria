# Laporan sistem — sesi fuzzer/differential (adendum)

Tanggal: 2026-09-21 · Workspace: /home/whale-d/mivon (crate mivon-fuzz, 14 target, oracle O1..O5)

## Ringkasan hasil

Kampanye `mivon-fuzz run` (lexer 7000, parser 4000, elab 4000, fmt 2340, sim 6400,
cli 3000, vcd/sdf/micd/synth/astdiff 4000, mv/preproc 6000) menemukan **13 kelompok bug unik**
(5 hang, 2 hang-elab differential,  People → fix):

### Fix dikirim (commit `8f567c3` + sebelumnya)
1. **elab: reject packed width >2^24 (E3012)** — hang elaboration `logic [2**31-1:0]`
   (miliaran-bit packed RIEMANN) → clean diag, tidak hang. Guard juga `elem_width`.
2. **elab: init array full → clamp ke storage nyata** — loop 256×2^31 iterasi hang.
3. **fmt: spasi Quote→Ident pd token emit** — roundtrip `'''bvalid` tidak lagi digabung
   jadi literal `'bvalid` (regresi fmt roundtrip rusak lexing). + test quote_ident.
4. **fmt: `'` Quote→Quote merge-space guard** (roundtrip `'''` charset).

### Regression
- 4308 test/edit pass; fmt roundtrip bug_0000 clean parse → fixed crash.

## Bug residual (untuk sesi lanjut — root flatten vector bit-select output port)
- bug_0030_differential (adder4 ripple): default engine `sum` = X pada phase listrik
  vector -bit output port (`assign {cout,sum}`);
  iverilog/iverilog replay = `<8>`, mivon default = `<0>`.
  Diferential ini sah dan sah-iverilog, bukan noise (differential oracle:
  dag-parallel `<8>` vs default `<0>`).
- Minimized artifacts: `.mivon-fuzz-bugs/bug_0030_*.sv` + VCD oracle
  tersimpan.

## Next steps recommended
1. flatten: vector bit-select output port connection flatten/inject assign
   (root cause di flatten instance output port untuk `logic [W-1:0]`).
2. Differential campaign dag vs default terus (batch background).
3. Triage report disimpan: `crates/mivon-fuzz/fuzz/reports/`.
