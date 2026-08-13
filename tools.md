11 CLI Tools Maria
1. minspect

Maria Inspect

Tool untuk menginspeksi isi project tanpa melakukan compile penuh.

Fungsi:

Menampilkan daftar module.
Menampilkan package.
Menampilkan interface.
Menampilkan class.
Menampilkan typedef.
Menampilkan dependency antar modul.
Menampilkan top module yang ditemukan.
Menampilkan statistik project.

Contoh:

maria minspect opentitan.f

Output:

Project Statistics
────────────────────────

Modules      : 3,972
Interfaces   : 84
Packages     : 119
Classes      : 162
Generate     : 1,438
Parameters   : 8,951

Top Modules
────────────
top_earlgrey
chip_earlgrey
...

Largest Module
──────────────
prim_ram_2p

Average LOC/module
──────────────────
241

Subcommand:

maria minspect stats
maria minspect modules
maria minspect hierarchy
maria minspect packages
maria minspect classes
maria minspect interfaces
maria minspect parameters
maria minspect deps
maria minspect cache

Subcommand `cache` membaca lapisan `cache/<pid>/` (db.md) tanpa compile —
menampilkan statistik per kategori (entries, bytes, hits, misses, hit-rate,
rebuilt) beserta ringkasan lapisan. Berguna untuk memeriksa apakah pipeline
cache (lexer/parser/semantic/type/dst.) benar-benar terisi dan di-reuse.

Kategori yang diisi otomatis saat compile: preprocess/, lexer/ (token
stream), parser/, macro/, include/, dependency/, resolve/, semantic/, type/,
constant/, hierarchy/, verify/, profile/. Setelah elaborasi, elaborate/
(instance + port binding + parameter override + proses + net resolution dari
IR) dan generate/ (blok if/for/case + instance hasil ekspansi) ikut terisi
(db.md "5. elaborate/", "16. generate/").

Cache yang sudah terisi dapat DIBACA ulang tanpa compile ulang:

- `maria melab file.sv --from-cache` — hierarki/instance/port binding/
  parameter override/proses/net dari cache elaborate/ + generate/, tanpa
  menjalankan elaborator (db.md "1000 instance generate tidak perlu
  dielaborasi ulang").
- `maria mprof file.sv --cached` — profil + bottleneck + rekomendasi dari
  build terakhir (db.md "20. profile/" — Maria mengetahui bottleneck sendiri),
  tanpa menjalankan pipeline.

Kategori yang belum diisi otomatis (optimize/expression/simulation/waveform/
coverage/lint) dapat dipopulasi tool via cache layer API.

Contoh output `maria minspect cache test/counter.sv`:

── Pipeline Cache ──
  root                       .maria/database
  project id                 029877faa3cede04
  files                      1

  Category      Entries        Bytes    Hits  Misses    Hit%
  lexer/              1        479 B       0       0      0%
  parser/             1         40 B       0       0      0%
  semantic/           1        117 B       0       0      0%
  ...

── Cache Summary ──
  categories                 21
  entries                    12
  hit rate                   0%

Kenapa lebih berguna?

Saat menghadapi proyek besar seperti OpenTitan, sering kali Anda hanya ingin mengetahui struktur proyek atau memastikan modul tertentu memang terdeteksi, tanpa membuang waktu menjalankan elaboration atau simulation. minspect memberikan "X-ray" terhadap proyek dengan cepat, memanfaatkan indeks internal Maria yang sudah ada.
2. mlint

Static RTL Linter.

Fitur

RTL lint
FSM check
Combinational loop
Latch detection
Width mismatch
Unused signal
maria mlint rtl/
3. melab

Standalone Elaborator.

Hanya melakukan

parameter resolve
generate
hierarchy
maria melab top.sv
4. msim

Simulator.

maria msim

Support

VCD
Wave
Assertion
Coverage
5. mcov

Coverage Analyzer.

Menghasilkan

coverage.html
coverage.json

Jenis

Line
Toggle
FSM
Branch
Assertion
6. mwave

Wave Utility.

Bukan viewer.

Digunakan

mwave merge
mwave export
mwave filter
7. mfmt

Formatter.

Mirip

cargo fmt

Tetapi untuk

Verilog
SystemVerilog
Maria HDL
8. mprof

Performance Profiler.

Output

Lexer

Parser

Elaboration

Optimization

Simulation

Bottleneck langsung terlihat.

9. mcheck

Project Health Checker.

Memeriksa

Missing file
Circular include
Dependency
Version
Config
10. mbench

Benchmark Tool.

mbench opentitan

Output

Compile speed

Memory

CPU

Cache hit

Parser throughput
11. synth

Synthesis Tool (SYNTHESIS.md — flow ala Vivado). Nama lama: `msynth` (alias).

Fungsi

Synthesizability check (SYN-1..9)
Lowering RTL → SIR (node-based, `maria-sir`)
Pass manager optimizer (const fold, arith, mux, CSE, DCE)
SIR → generic netlist (`maria-netlist` — 1-driver/N-load DAG)
Netlist `.mvnet` / `netlist.v` / `netlist.json`
Utilization report

Contoh

maria synth rtl/counter.sv --top counter --emit-mvnet
maria synth rtl/ --check-only
maria synth rtl/counter.sv --dump-sir
maria synth rtl/alu.sv --dump-sir-opt --preset generic
maria synth rtl/counter.sv --top counter --dump-netlist
maria synth rtl/counter.sv --top counter --emit-netlist

Output

Skor sintesizability
Dump SIR sebelum & setelah optimasi (--dump-sir / --dump-sir-opt)
Dump netlist generik (--dump-netlist)
Emit netlist ke file: `counter.netlist.v` / `.mvnet` / `.json` (--emit-netlist)
Pass manager: const fold, arith, mux, CSE, DCE (--preset generic|fpga|asic|custom)
FF / LUT / CARRY4 / BRAM / DSP
Netlist gate-level — bisa disimulasikan engine Maria (hasil = sim RTL)
10 GUI Tools Maria
1. Project Explorer

Panel kiri.

Isi

RTL

Testbench

Package

Interface

Library

IP
2. Hierarchy Explorer

Menampilkan

Top

Module

Instance

Generate Block

Tree interaktif.

3. Signal Browser

Semua

wire

logic

reg

interface

parameter

Bisa search realtime.

4. Wave Studio

Viewer bawaan.

Fitur

Zoom
Marker
Trigger
Compare
Bookmark
5. Diagnostics Center

Semua

Warning

Error

Note

Hint

Klik langsung menuju source code.

6. Performance Dashboard

Panel performa compile.

Grafik

Lexer

Parser

Elaboration

Optimization

Simulation

Realtime.

7. Coverage Studio

Visualisasi

Heatmap coverage
Branch
Toggle
FSM
Assertion

Modul dengan coverage rendah langsung disorot.

8. Dependency Graph

Visual graph.

Menampilkan

Module A

↓

Module B

↓

Package C

Memudahkan memahami proyek OpenTitan yang memiliki ribuan modul, tanpa harus membaca include dan instansiasi satu per satu seperti sedang memecahkan teka-teki buatan orang yang membenci dokumentasi.

9. Memory & Cache Monitor

Khusus Maria.

Menampilkan

Cache hit

Cache miss

Arena allocator

AST

HIR

IR

Database

Memory

Realtime.

10. Compile Timeline

Timeline horizontal seperti profiler modern.

Discover

████

Preprocess

██████

Lexer

██████

Parser

██████████

Elaboration

████████████████

Optimization

████

Simulation

██

Klik salah satu blok akan membuka detail waktu, penggunaan CPU, memori, jumlah modul, dan statistik internal. Ini sangat membantu menemukan bottleneck pada proyek besar seperti OpenTitan tanpa harus mengandalkan log terminal yang panjangnya bisa menyaingi novel.

Arsitektur yang disarankan

Seluruh tool CLI dan GUI sebaiknya memakai backend yang sama agar tidak ada duplikasi logika:

                 Maria Core
                      │
      ┌───────────────┼───────────────┐
      │               │               │
   Parser         Elaborator      Simulator
      │               │               │
      ├───────────────┼───────────────┤
      │         MICD Database         │
      └───────────────┼───────────────┘
                      │
         ┌────────────┴────────────┐
         │                         │
      CLI Tools                GUI Panels
 (mlint, melab, ...)      (Wave, Timeline,
                           Coverage, dll.)
