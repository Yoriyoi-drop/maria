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
| `differential.rs` | ✅ | determinism + EMI dead-code, compare sinyal common (#13/#19) |
| `directed.rs` | ✅ | bias seed ke fitur target (#17) |
| `cdg.rs` | ✅ | target = fitur unreached; progress report (#20) |
| `main.rs` (bin) | ✅ | CLI --iters/--seed/--corpus-dir/--target/--emit-bugs + env |

### Hasil smoke campaign pertama (seed 42, 200 iterasi)

```
iters=200 compile_ok=63 compile_err=137 sim_ok=63 sim_err=0
panics=0 hangs=0 new_features=9 det_mismatch=0 emi_mismatch=3 covered=43
```

**Temuan terkonfirmasi (bug EMI nyata):** menambah `wire [7:0] _fuzz_dn;
assign _fuzz_dn = 8'h00;` (dead-code) sebelum `endmodule` modul hierarki yang
memakai part-select out-of-range (`a[3:0]` pada `a` 2-bit) mengubah hasil
simulasi modul child: `y=10 → xx`, `__port_u_child_x=xx01 → xxxx`.
Tereproduksi via CLI (`maria --print-state`). Dugaan: init/indexing sinyal
port atau part-select terpengaruh keberadaan sinyal tambahan → **perlu triage
di simulator** (bukan di fuzzer).

### Roadmap lanjutan (belum dikerjakan)

- Minimizer untuk bug differential (saat ini hanya panic yang di-minimize).
- Database bug persisten (re-seed lintas kampanye, #18 FSM-aware).
- Corpus paralel bersama antar worker (saat ini tiap kampanye punya corpus sendiri).
- Fuzzing SVA/assertion + covergroup (property-oracle).
- Fuzzing fitur mahal: interface/class/UVM/DPI melalui corpus opentitan/cva6.