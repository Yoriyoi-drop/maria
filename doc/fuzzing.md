# Fuzzing maria — Landasan 20 Jurnal/Paper

> **Status:** Desain landasan — dokumen acuan utama untuk implementasi `crates/maria-fuzz`.
> **Audience:** Pengembang maria **saja** (dev-only). Bukan fitur CLI user, bukan bagian `maria-tests`.
> **Versi dokumen:** 1.0

---

## 1. Tujuan & Lingkup

`crates/maria-fuzz` adalah fuzzer internal maria (RTL simulator SystemVerilog berbasis
Rust). Fuzzer ini menguji **pipeline maria itu sendiri** — preprocessor → lexer → parser →
AST → elaborator → IR → engine → VCD — bukan DUT di dalam SV. Sumber bug yang dicari:

1. **Crash/hang** — panic, infinite loop, stack overflow, OOM saat compile/simulasi.
2. **Salah semantik** — hasil simulasi menyimpang dari oracle (differential testing).
3. **Recovery buruk** — diagnostic error tidak sampai, malformed error, exit code salah.
4. **Coverage stagnan** — fitur SV yang tidak pernah ter-exercise karena tidak ada seed yang memicunya.

Batasan tegas:

- **Tidak** menjadi bagian workspace default build user (`cargo build` biasa tidak menyentuh crate ini).
- **Tidak** berada di `crates/maria-tests/` (terpisah, crate sendiri `maria-fuzz`).
- Hanya aktif lewat feature `dev` (`required-features`).
- Dokumen ini = 1 file = 1 tanggung jawab: **landasan ilmiah + peta desain implementasi**.

### Konvensi penomoran paper

Setiap paper diberi nomor `Paper #N` (1–20). Nomor ini dipakai konsisten di komentar
kode fuzzer, mis. `// Paper #4 (AFLFast): power schedule` — agar setiap keputusan desain
bisa ditelusuri balik ke literaturnya. Menggantikan penomoran fuzz lama (ChipFuzzer/EMI/MOpt
dengan nomor lama dibuang — sistem baru, penomoran baru).

---

## 2. Ringkasan 20 Paper (6 Pilar)

| Pilar | # | Paper | Kontribusi desain ke maria-fuzz |
|-------|---|-------|----------------------------------|
| **A. Fondasi fuzzing** | 1 | Miller 1990 (UNIX utilities) | Generator input acak murni — baseline `GenInput::random()` |
| | 2 | Liang 2018 (TDSC survey) | Taksonomi strategi → layout modul fuzzer |
| | 3 | Manès 2019 (S&P survey) | Klasifikasi generation vs mutation, komponen feedback loop |
| **B. Coverage-guided greybox** | 4 | AFLFast (CCS 2017) | Power schedule `energy ∝ 1/freq(path)^α` — seed energy scheduling |
| | 5 | FairFuzz (ASE 2018) | Targeting cabang langka (rare-branch) utk mutasi prioritas |
| | 6 | CollAFL (S&P 2018) | Coverage sensitif-lintasan (path-sensitive) → per-stage pipeline |
| | 7 | VUzzer (NDSS 2017) | Fitur dataflow: op, lebar bit, outcome → feature map |
| | 8 | EcoFuzz (USENIX 2020) | Energi adaptif — alokasi budget iterasi dinamis |
| **C. Grammar & struktur-aware** | 9 | NAUTILUS (NDSS 2019) | Grammar kontekstual SV (token-level) utk generate & mutate |
| | 10 | Superion (ASPLOS 2019) | Mutasi struct-aware — memakai **AST maria sendiri** sbg struktur |
| | 11 | Grammarinator (A-TEST 2018) | Grammar-based generation berulang (ekspansi produksi) |
| | 12 | Code Fragments (USENIX 2012) | Mutasi dari serpihan SV nyata (corpus: `test/`, `opentitan/`, `cva6/`) |
| **D. Differential & compiler testing** | 13 | EMI (PLDI 2014) | Equivalence Modulo Inputs — oracle differential utama |
| | 14 | Csmith (PLDI 2011) | Generator modul SV acak lengkap (random program generation) |
| | 15 | YARPGen (OOPSLA 2020) | Random gen **type-aware** (signed/lebar/4-state, hindari UB) |
| **E. Fuzzing RTL spesifik** | 16 | RFUZZ (ICCAD 2018) | Coverage-directed RTL: umpan balik dari sinyal internal modul |
| | 17 | DirectFuzz (DAC 2021) | Fuzzing **terarah**: seed dari AST feature target |
| | 18 | Fuzzing Hardware Like Software (USENIX 2022) | Harness design-agnostic, FSM-aware seed scheduling |
| **F. Differential RTL & CDG** | 19 | DifuzzRTL (S&P 2021) | Differential fuzz per-module/CPU — oracle nilai sinyal vs golden |
| | 20 | Fine & Ziv (HLDVT 2003) | Coverage-Directed Test Generation (Bayesian) utk functional verification |

> Referensi pendukung (tidak masuk hitungan 20): Ioannides & Eder, *Coverage-Directed
> Test Generation Automated by Machine Learning — A Review*, ACM TODAES 17(1), 2012 —
> dipakai utk sintesis strategi CDG di oracle.

---

## 3. Referensi Lengkap (bibliografi terverifikasi)

### Pilar A — Fondasi fuzzing

1. **Miller, B. P., Fredriksen, L., So, B.** — *An Empirical Study of the Reliability of UNIX Utilities*. Communications of the ACM, 33(12), 1990. *(Paper pelopor random testing — dasar semua fuzzer.)*
2. **Liang, H., Pei, X., Jia, X., Shen, W., Zhang, J.** — *Fuzzing: State of the Art*. IEEE Transactions on Dependable and Secure Computing (TDSC), 15(3), 2018.
3. **Manès, V. J. M., Han, H., Han, C., Cha, S. K., Egele, M., Schwartz, E. J., Woo, M.** — *The Art, Science, and Engineering of Fuzzing: A Survey*. IEEE Symposium on Security and Privacy (S&P), 2019.

### Pilar B — Coverage-guided greybox fuzzing

4. **Bö hme, M., Pham, V.-T., Roychoudhury, A.** — *Coverage-based Greybox Fuzzing as Markov Chain* (AFLFast). ACM CCS, 2017.
5. **Lemieux, C., Sen, K.** — *FairFuzz: A Targeted Mutation Strategy for Increasing Greybox Fuzz Testing Coverage*. IEEE/ACM ASE, 2018.
6. **Gan, S., Zhang, C., Qin, X., Tu, X., Li, K., Pei, Z., Chen, Z.** — *CollAFL: Path Sensitive Fuzzing*. IEEE S&P, 2018.
7. **Rawat, S., Jain, V., Kumar, A., Cojocar, L., Giuffrida, C., Bos, H.** — *VUzzer: Application-aware Evolutionary Fuzzing*. NDSS, 2017.
8. **Yue, T., Wang, P., Tang, Y., Wang, E., Yu, B., Lu, K., Zhou, X.** — *EcoFuzz: Adaptive Energy Conservation for Efficient Coverage-guided Fuzzing*. USENIX Security, 2020.

### Pilar C — Grammar & struktur-aware fuzzing

9. **Aschermann, C., Frassetto, T., Holz, T., Jauernig, P., Sadeghi, A.-R., Teuchert, D.** — *NAUTILUS: Fishing for Deep Bugs with Grammars*. NDSS, 2019.
10. **Wang, J., Chen, B., Wei, L., Liu, Y.** — *Superion: Grammar-Aware Greybox Fuzzing*. ASPLOS, 2019.
11. **Hodován, R., Kiss, Á., Gyimóthy, T.** — *Grammarinator: A Grammar-Based Open Source Fuzzer*. A-TEST@ESEC/SIGSOFT FSE, 2018, pp. 45–48.
12. **Holler, C., Herzig, K., Zeller, A.** — *Fuzzing with Code Fragments*. USENIX Security, 2012.

### Pilar D — Differential & compiler testing

13. **Le, V., Afshari, M., Su, Z.** — *Compiler Validation via Equivalence Modulo Inputs* (EMI). ACM PLDI, 2014.
14. **Yang, X., Chen, Y., Eide, E., Regehr, J.** — *Finding and Understanding Bugs in C Compilers* (Csmith). ACM PLDI, 2011.
15. **Livinskii, V., Babokin, D., Regehr, J.** — *Random Testing for C and C++ Compilers with YARPGen*. ACM OOPSLA, 2020.

### Pilar E — Fuzzing RTL spesifik

16. **Laeufer, K., Koenig, J., Kim, D., Bachrach, J., Sen, K.** — *RFUZZ: Coverage-Directed Fuzz Testing of RTL on FPGAs*. IEEE/ACM ICCAD, 2018.
17. **Canakci, S., Delshadtehrani, L., Eris, F., Taylor, M. B., Egele, M., Joshi, A.** — *DirectFuzz: Automated Test Generation for RTL Designs Using Directed Graybox Fuzzing*. 58th ACM/IEEE DAC, 2021, pp. 529–534.
18. **Trippel, T., Shin, K. G., Chernyakhovsky, A., Kelly, G., Rizzo, D., Hicks, M.** — *Fuzzing Hardware Like Software*. 31st USENIX Security Symposium, 2022. (arXiv:2102.02308)

### Pilar F — Differential RTL & CDG

19. **Hur, J., Song, S., Kwon, D., Baek, E., Kim, J., Lee, B.** — *DifuzzRTL: Differential Fuzz Testing to Find CPU Bugs*. IEEE S&P, 2021.
20. **Fine, S., Ziv, A.** — *Coverage Directed Test Generation for Functional Verification Using Bayesian Networks*. IEEE HLDVT, 2003.

---

## 4. Peta Paper → Modul Kode

Rencana struktur `crates/maria-fuzz/src/` (1 file = 1 tanggung jawab, sesuai aturan project):

| Modul | Paper pendorong | Tanggung jawab |
|-------|-----------------|----------------|
| `gen.rs` | #1, #14, #15 | Generator `GenInput`: random murni, modul SV acak (Csmith-style), type-aware (YARPGen-style) |
| `guide.rs` | #4, #5, #8 | `CoverageGuide`: power schedule, rare-branch targeting, energi adaptif |
| `feature.rs` | #6, #7 | Feature map path-sensitive + fitur dataflow (op/lebar/outcome) |
| `grammar.rs` | #9, #11 | Grammar SV token-level: ekspansi produksi, generate & mutate |
| `ast_mutate.rs` | #10, #12 | Mutasi struct-aware via AST maria; corpus serpihan SV nyata |
| `differential.rs` | #13, #19 | Oracle EMI + oracle nilai sinyal per-module (DifuzzRTL-style) |
| `cdg.rs` | #20 | Coverage-directed test generation (fairness/utility scheduling) |
| `directed.rs` | #17 | Fuzzing terarah per-fitur/AST node target |
| `harness.rs` | #18 | Harness design-agnostic: seed scheduling, FSM awareness |
| `oracle.rs` | #2, #3 | Verdict: compile-oracle, sim-oracle, elab-oracle, property-oracle |
| `corpus.rs` | #12 | Seed corpus SV nyata (test/, opentitan/, cva6/) + minimizer |
| `lib.rs` | — | Orkestrasi, pipeline fuzzer, entry point library |
| `main.rs` (bin, feature `dev`) | — | CLI internal: `cargo run -p maria-fuzz --features dev` |

### Loop utama fuzzer (feedback-driven)

```
            ┌─────────────────────────────────────────────┐
            │                                             │
  corpus ──►│  pilih seed (energy schedule: #4 #5 #8)     │
            │      │                                      │
            │  mutate (grammar #9 #11, AST #10, frag #12) │
            │      │                                      │
            │  compile maria (parse→elab→sim, feature #7) │
            │      │                                      │
            │  orakel (#2 #3 #13 #19 #20) ──► verdict     │
            │      │                                      │
            │  update feature map (#6 #7) ──► bug? ──► lapor│
            │      │                                      │
            │  seed baru masuk corpus ─────────────────────┘
```

---

## 5. Strategi Oracle

1. **Compile-oracle** — input SV divalidasi: maria harus kompilasi atau gagal dgn
   diagnostic *terstruktur* (`DiagCode`), tidak panic/hang.
2. **Differential EMI (#13)** — ubah input (mutasi dead-code / equivalence-preserving
   pada level AST maria) → program **harus** menghasilkan output simulasi identik.
   Penyimpangan = bug semantik.
3. **Differential per-module (#19)** — instansiasi modul yang sama dgn dua konfigurasi
   parametrik ekivalen → sinyal output harus sama.
4. **CDG oracle (#20)** — nilai sinyal pada waktu tertentu dipatok sbg *coverage target*;
   fuzzer diarahkan memenuhi target tsb (bukan sekadar no-crash).
5. **Property-oracle** — seed menyisipkan `assert`/`cover`; fuzzer verifikasi assertion
   melanggar = bug engine.

## 6. Metrik & Kriteria Sukses

- **No crash** — 0 panic/hang/OOM utk N iterasi default (env `MARIA_FUZZ_N`).
- **Coverage feature map** — % operator, lebar bit, statement, region yang sudah diexercise
  (dilaporkan per-stage pipeline, CollAFL-inspired #6).
- **OSS-Fuzz-style minimizer** — setiap bug direduksi ke seed minimal sebelum dilaporkan.
- **Bug database** — hasil fuzz tersimpan utk regresi (dipakai kembali sbg seed, #18: FSM-aware).

## 7. Cara Pakai (dev-only)

```shell
# bangun & jalankan fuzzer internal (WAJIB feature dev)
cargo run -p maria-fuzz --features dev -- --iters 300 --seed 42

# jumlah iterasi / paralelisme lewat env
MARIA_FUZZ_N=1000 MARIA_FUZZ_WORKERS=4 cargo run -p maria-fuzz --features dev

# jalankan test suite fuzzer
cargo test -p maria-fuzz --features dev
```

> `maria-fuzz` **tidak** ikut `cargo build`/`cargo test` biasa — anggota workspace
> eksklusif feature `dev` + `required-features`.

## 8. Roadmap Implementasi

| Fase | Isi | Paper utama |
|------|-----|-------------|
| 1 | Scaffold crate: `lib.rs`, `main.rs`, orkestrasi loop, compile-oracle | #2, #3 |
| 2 | Generator dasar: `gen.rs` random + type-aware | #1, #15 |
| 3 | Mutasi grammar + AST | #9, #10, #11 |
| 4 | Coverage guide + feature map | #4, #5, #6, #7, #8 |
| 5 | Oracle differential EMI + per-module | #13, #19 |
| 6 | Directed & CDG & corpus nyata | #12, #17, #20 |
| 7 | Harness FSM-aware, minimizer, bug DB | #14, #16, #18 |

---

## 9. Catatan Akhir

- Dokumen ini ditulis **sebelum** implementasi kode — statusnya desain landasan.
  Setiap modul kode wajib merujuk nomor `Paper #N` di doc comment-nya.
- Bibliografi sudah diverifikasi via dblp/crossref (RFUZZ, DirectFuzz DAC 2021,
  Grammarinator A-TEST 2018, Ioannides & Eder TODAES 2012).
- Perubahan daftar paper harus lewat revisi dokumen ini (versi naik), bukan lewat
  perubahan kode diam-diam.

## 10. Status Implementasi

**v1 selesai (2026-09-06)** — seluruh modul kode di `crates/maria-fuzz/` ada,
41 unit test hijau (`cargo test -p maria-fuzz --features dev`), crate terdaftar
sebagai anggota workspace (lib kosong tanpa feature `dev`; bin memakai
`required-features = ["dev"]` sehingga tidak ikut build user).

**v2 selesai (2026-09-07)** — 55 unit test hijau. Ringkasannya di bawah;
detail bug + kampanye di bagian "v2" setelah daftar bug v1. Kode fuzzer
tersebar di `crates/maria-fuzz/src/` (14 file, 1 file = 1 tanggung jawab).

| Modul | Status | Catatan |
|-------|--------|---------|
| `lib.rs` (orkestrasi) | ✅ | loop fuzz + report + merge kampanye paralel |
| `gen.rs` | ✅ | module acak valid + syntax acak (#1/#14/#15) |
| `grammar.rs` | ✅ | keyword, decl/expr/body snippet (#9/#11) |
| `ast_mutate.rs` | ✅ | op-replace, literal-flip, splice corpus, insert grammar, dup line (#10/#12) |
| `corpus.rs` | ✅ | load `.sv` dari dir, fragment, minimizer baris (#12) |
| `feature.rs` | ✅ | feature map op/konstruk/lebar + stage + err-code (#6/#7) |
| `guide.rs` | ✅ | energy schedule AFLFast + rare boost + α adaptif (#4/#5/#8) |
| `oracle.rs` | ✅ | compile/sim verdict + fingerprint sinyal (#2/#3) |
| `harness.rs` | ✅ | isolasi thread, panic-catch, hang-timeout (#18) |
| `differential.rs` | ✅ | determinism + EMI dead-code + expr-swap, compare common (#13/#19) |
| `directed.rs` | ✅ | bias seed ke fitur target (#17) |
| `cdg.rs` | ✅ | target = fitur unreached; progress report (#20) |
| `bugdb.rs` | ✅ | bug DB persisten lintas-kampanye + re-seed parent (#18) |
| `main.rs` (bin) | ✅ | CLI --iters/--seed/--corpus-dir/--target/--emit-bugs + env |

### Hasil smoke campaign pertama (seed 42, 200 iterasi)

```
iters=200 compile_ok=63 compile_err=137 sim_ok=63 sim_err=0
panics=0 hangs=0 new_features=9 det_mismatch=0 emi_mismatch=3 covered=43
```

### Bug ditemukan fuzz → sudah diperbaiki (triage & fix di maria-simulator)

**Bug 1 — part-select OOB beda hasil serial vs paralel (EMI)**
`a[3:0]` pada `a` 2-bit: jalur paralel (SIM-28) mem-X-kan SELURUH hasil
(`xxxx`) sedangkan jalur serial memberi `xx01` — modul child dirubah nilai
hanya karena dead-code menambah proses comb (melewati ambang paralel).
*Fix:* `parallel.rs` RangeSelect — bit luar batas → X, bit dalam batas →
nilai asli (selaras LRM §11.5.1 & jalur serial).

**Bug 2 — semantik X/Z bitwise tidak konsisten jalur serial vs paralel**
`x & 0`: jalur serial packed (`eval_binary_packed`, tabel LRM) = 0;
jalur paralel (`eval_binary` pessimistic) = X. Dead-code EMI menaikkan
jumlah proses comb 3→4 → paralel aktif → hasil berubah (`0` vs `x`).
*Fix:* `parallel.rs` `with_packed_eval()` — flag scoped thread-local agar
evaluator paralel memakai semantik `use_packed_eval` yang sama dgn serial.

**Bug 3 — BitAnd/BitOr pessimistic `Z` salah vs LRM**
`0 & Z`/`1 | Z` dihitung X di mode pessimistic; tabel LRM 4-state memberi
0/1 (nilai dominan). `bitwise_op` fast-path meratakan Z→X sebelum closure.
*Fix:* `value.rs` — fast-path baca nilai bit asli dari `bits` (preservasi Z),
pessimistic menangani Z-dominan (`0&Z=0`, `1|Z=1`); X tetap pessimistic.
+2 test regresi (`test_xprop_pessimistic_bitand_z_dominated` / `_bitor_`).

**Verifikasi setelah fix:** 345 test maria-simulator + seluruh workspace
(899 + 378 + 307 + … semua hijau), kampanye ulang seed 42 × 300 iterasi:
`emi_mismatch=0 bugs=0`.

---

## v2 (2026-09-07) — responsif korpus, minimizer hang, hilangkan artefak oracle

### Perbaikan maria-fuzz (artefak fuzzer → bukan bug engine)

**Bug F1 — false-positive property-oracle dari minimizer**
Minimizer baris bisa menghapus deklarasi `wire [W-1:0]` temp mirror
(`_fz_rtA`/`_fz_rtB`) → temp jadi implicit net lebar default → `A !== B` = 1
walaupun ekspresi sama → viol=1 palsu yang BERKELANJUTAN. `duplicate_line`
juga bisa meng-drive temp dua kali (multi-driver → X → viol=1 palsu).
*Fix:* `lib.rs` `mirror_intact()`/`mirror_pair_intact()` — setiap blok mirror
WAJIB utuh (2 deklarasi lebar sama, tepat 1 driver per temp, rhs identik,
viol ter-deklarasi). Blok rusak di-skip. +6 unit test.

**Bug F2 — `sim_signal_check` anomali palsu utk sumber nondeterministik**
`$urandom`/`$random` berbeda lintas run secara SAH → fingerprint beda dilapor
anomali. *Fix:* `oracle.rs` skip bila `has_nondeterministic_src` (sama dgn
determinism/EMI). +1 unit test.

**Bug F3 — minimizer hang belum ada**
Kampanye Hang (parser opentitan kmac fragment) harus di-bisect manual.
*Fix:* `lib.rs` cabang `RunStatus::Hang` — minimasi baris sambil status tetap
Hang (`run_isolated` predicate, ambang ≤ 800 ms agar tiap kandidat yang
masih hang tidak membakar penuh `hang_ms`). Laporan bug Hang sekarang
membawa input terminimalkan.

**Bug F4 — re-seed bug-DB tidak efektif (#18)**
Bug lama masuk `corpus.seeds` (hanya sumber fragment) tapi TIDAK masuk
`guide` → tidak pernah jadi parent mutasi → regressi tidak benar-benar
diexercise antar kampanye. *Fix:* `lib.rs` — bug re-seed juga
`guide.add(src, feats, is_bug=true)` (energi tinggi, #20 bug-boosted).

**Lain:** `corpus.rs` auto-detect tambah `examples/`, `fuzz/`, `cva6/`;
`gen.rs` shape baru #4 (`always_comb` + `for` unroll + nested `if/else` +
part-select — stress statement-engine).

### Bug utama maria ditemukan fuzz → diperbaiki

**Bug M1 — Hang parser: `parse_clocking_block` loop infinite di EOF**
`crates/maria-parser/src/specify.rs` `parse_clocking_block` — arm catch-all
`_ => advance()` tanpa guard EOF. Input terpotong `clocking cb @(posedge
tck);` (tanpa `endclocking`) di module implisit level-unit → di EOF advance
macet (pos jebol), loop tidak pernah break; safety counter `parse_steps`/
`peek_count` tidak efektif karena kontrol tidak pernah kembali ke cek
level-atas. Hang >170 s RSS flat. *Fix:* arm `_` break bila `Token::Eof`.
Regresi: `test_parser_no_hang_on_truncated_clocking_fragment` (watchdog 10s).

**Bug M2 — panic elaborasi: concat part-select lebar negatif**
`{ sig[0+:(IDW-STIDW)], ... }` dgn `IDW = top_pkg::TL_AIW` (symbol package
unresolved → 0) dan `STIDW = $clog2(M)` (2) → `IDW-STIDW` = -2 →
`const_eval_params` i64 → `as usize` wrap ke 2^64-2 →
`ExprRangeSelect(hi=u64::MAX-1, lo=0)` → `expr_approx_width` concat `.sum()`
panic `attempt to add with overflow`. 6× ditemukan seed 42 (korpus opentitan).
*Fix dua lapis:* (1) `stmt.rs expr_approx_width` — Concat/Replicate saturasi
(lebar hanya perkiraan utk konteks sizing — tidak boleh panic);
(2) `expr.rs` RangeSelect/ExprRangeSelect — bound const `max(0)` sebelum
`as usize`, negatif = OOB (engine isi X per §11.5.1).
Regresi: `test_no_panic_concat_negative_range_select`.

### Hasil kampanye v2 (korpus aktif, 7 × 300 iterasi)

Sebelum fix M1/M2: seed 42 → `panics=6 hangs=0 bugs=6` (6× overflow panic,
semua dari concat same); bugdb Hang opentitan kmac → hang parser.
Setelah fix M1/M2: `panics=0 hangs=0 det=0 emi=0 sig=0 prop=0 bugs=0` untuk
7 seed (42, 1, 12345, 77, 31337, 2024, 999). Property-oracle artifact
sebelumnya (10+7 viol=1 palsu) kini 0.

### Roadmap lanjutan (belum dikerjakan)

- **Minimizer differential** ✅ sudah jalan (det/EMI/sim-sig/property) + **Hang** ✅ (v2).
- **Database bug persisten** ✅ (`bugdb.rs`, re-seed jadi parent mutasi).
- **Corpus paralel bersama antar worker** ✅ (`main.rs` `--workers`).
- Fuzzing **SVA/assertion + covergroup** (property-oracle #5) — mirror ada;
  `assert`/`cover` ala SVA belum (butuh evaluasi dukungan assertion engine).
- Fuzzing fitur mahal: **interface/class/UVM/DPI** via corpus opentitan/cva6 —
  sebagian tercakup korpus; masih butuh oracle khusus.

---

## v2.1 (2026-09-07) — assert-oracle, hang delta-storm, sim-err triage

### Mutasi baru: assert-oracle (Paper #14/#2/#3, oracle #5)
`ast_mutate.rs insert_assert_oracle` — tanam assertion yang WAJIB benar:
```systemverilog
wire [W-1:0] _fz_atA_N;  assign _fz_atA_N = (<rhs>);
wire [W-1:0] _fz_atB_N;  assign _fz_atB_N = (<rhs>);
initial #1 assert (_fz_atA_N === _fz_atB_N) else $fatal(0, "...");
```
Dua temp mengevaluasi ekspresi SAMA → engine konsisten = assertion selalu pass;
fail = bug evaluasi. Berbeda dgn mirror (`_fz_viol` net compare): di sini
`$fatal` membuat SIMULASI GAGAL (`RT7001`) — tak ada false-positive dari
minimizer/multi-driver. Main loop: sim_err pada source yang memuat
`has_assert_oracle` → property_violations + minisasi. Mutasi op #11.

### Hang delta-storm: bedakan hang sejati vs engine-settle (false positive massal)
`always @(posedge clk)` tanpa clock = delta-storm; engine SETTLE via delta-limit
(100k) → selesai normal (~10s build debug). Dengan `hang_ms=2000`, eksekusi ini
`>hang_ms` → fuzzer salah-klaim `Hang` (12–32×/kampanye seed 999/4567/8888),
padahal **bukan hang sejati** — engine menangguhkan dengan error `RT2001`, bukan
infinite.
*Fix:* default `hang_ms` naik 3000 → **12000** (di atas settle delta-storm
debug). Delta-storm kini selesai normal (bukan Hang); hanya hang SEJATI yang tak
pernah settle (parser/stack infinite, mis. M1 clocking, recursive elaboration)
yang >12s → Hang & di-minimize. Konfirmasi hang memakai window ≥15s sebelum
posting, agar kandidat yang engine-settle tidak salah masuk bug DB.

### Bug utama maria dari sim-err triage (MARIA_FUZZ_SIMERR dump)
Sweep sim-err seed 42 (release): 43× `RT2001` (delta-limit — input osilasi,
engine benar tolak) + 8× `RT0001`.

**Bug M3 — RT0001 nama sinyal KOSONG**
`error[RT0001]: hierarchical signal '' not found` — concat part-select member
`tl_h_i[i].a_source[0+:(IDW-STIDW)]` dengan `tl_h_i[i]` tak-resolved →
`build_hier_name` obj (Index/PartSelect) → `""` → fallback `Err(_)` emit
`HierRef(Symbol::intern(""))` → engine luntur `signal '' not found` (diagnostic
tak berguna).
*Fix:* `elaborator/expr.rs` MemberAccess fallback — kalau `hier_name` kosong,
pakai nama field (`a_source`). Kini `RT0001: hierarchical signal 'a_source'
not found`. Regresi `test_rt0001_reports_real_signal_name_not_empty`.

`RT2001` dinyatakan **bukan bug engine** (input malformed; engine tolak benar) —
tidak dilaporkan sebagai bug fuzz.

### Verifikasi v2.1
`cargo test` seluruh hijau (maria-tests 902, simulator 349, compiler 307,
elaboration 6, maria-fuzz 57). Kampanye release seed 999 120 iterasi:
`hangs=0 bugs=0` (sebelumnya 12 hang palsu). Seed 42 release 300 iterasi:
`hangs=0 bugs=0`. Run fuzzer disarankan **release** (`--release`) — build debug
delta-storm settle ~10s bikin kampanye lambat.
---

## v2.2 (2026-09-07) — interface-oracle, EMI artefak filter, assert-oracle proof

### Mutasi baru: interface-oracle (fitur mahal, Paper #10/#12)
`ast_mutate.rs insert_interface` (op #12) — tanam interface utuh + modport +
child yang memakai modport + instansiasi + koneksi lintas-modul (stress
elaborator interface/hierarki/modport). Self-contained (nama unik `fz_bus_`,
`fz_child_`, `fz_bif_`). `has_interface_oracle` utk deteksi. +2 unit test.

### Fix assert-oracle false positive (11 palso, seed 42)
Assert-oracle main-loop hanya cek `has_assert_oracle` (ada `_fz_at`) — tapi
sim_err bisa RT0001 lain (hier-signal tak-resolved dari source malformed),
bukan assertion fail. Minimizer menghapus deklarasi temp → 11 bug palsu
`assert-oracle: sim err RT0001`.
*Fix:* hanya sim code **RT7001** (dari `assert ... else $fatal`) + 
`has_assert_oracle_temps` (blok utuh: deklarasi wire temp lebar sama) →
+ `sim_err_isolated` harness (return sim error code utk predicate minimasi).

### Fix EMI artefak filter (2 palso, seed 31337)
EMI minimizer menghasilkan source malformed (implicit net / multi-driver, mis.
`fz_q1` di-drive 2×, `a,[11:11]` concat koma, interface data 9→8 trunc) →
dead-code mengubah net-topology → `z` vs `x` beda. Bukan bug engine.
*Fix dua lapis:* (1) minimizer determinism/EMI — kandidat wajib compile+sim ok
(`fingerprint_isolated().is_some()`) + tetap mismatch; (2) `compare_common` —
abaikan sinyal internal artefak fuzzer (`is_internal_artifact`: prefix `fz_`,
`_fz_`, `_fuzz_` — semua ditanam oracle). EMI bug sejati = dead-code mengubah
sinyal top nyata. +1 unit test (`internal_artifact_filter`).

### Verifikasi v2.2
`cargo test` hijau: maria-fuzz **61** (+4 oracle/artifact). Kampanye release
7 seed (42/1/12345/77/999/2024/31337) × 300 iterasi:
`panics=0 hangs=0 det=0 emi=0 sig=0 prop=0 bugs=0` (semua false-positive
yang muncul di v2.1: 11 assert-palsu + 2 EMI-palsu, kini 0).
