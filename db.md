Maria Incremental Compilation Database (MICD)
                Maria Compiler
                      │
          ┌───────────┼────────────┐
          │           │            │
      Lexer Cache Parser Cache Symbol Cache
          │           │            │
          └───────────┼────────────┘
                      │
             Dependency Graph
                      │
            Incremental Scheduler
                      │
          Verification Cache
                      │
              Binary Database
                 (*.mdb)

Bukan relational database.

Melainkan object database.

Struktur Folder
project/

    .maria/

        database/                       # database root (override: MARIA_MICD_DIR)

            VERSION                     # versi skema (SCHEMA_VERSION)

            registry.json               # pid → info project (root, sources, waktu)

            locks/

                <pid>.lock              # writer lock exclusive per project

            objects/                    # payload IMMUTABLE — content-addressed (CAS)

                <pid>/

                    <hex-hash>.ast      # Design terserialisasi per konten hash

                    <hex-hash>.preproc  # combined source per konten hash

            state/                      # index MUTABLE per project

                <pid>/

                    metadata.mdb        # per-file: hash, mtime, size, status, deps

                    graph.mdb           # dependency graph (CSR + reverse index)

                    symbol.mdb          # index simbol

                    types.mdb           # index tipe/signature module

                    verify.mdb          # verification cache

                    diagnostics.mdb     # diagnostic per file

                    stats.mdb           # profil build (mprof/mbench)

                    journal.mdb         # transaksi (crash recovery)

                    snapshots/

                        build-001

                        build-002

Semuanya binary.

Tidak ada SQL.

Layout mengikuti pola Git (`objects/` + `refs/`): AST & preprocessed source
adalah payload immutable yang disimpan per konten hash (dedup otomatis, dua
file dengan isi sama berbagi satu objek); index (metadata/graph/verify/symbol/
types/diag) adalah state mutable yang ditulis transaksional via journal.
Locks hidup di `locks/`, terpisah dari data. Layout lama
`projects/<pid>/<file>.mdb` di-migrasi otomatis ke `state/` + `objects/` saat
dibuka.

File Metadata
metadata.mdb

Isinya

FileID

Absolute Path

Hash

Timestamp

Compiler Version

Language Version

Flags

Dependencies

Status


Contoh

FileID : 0x001233

Hash :
5f347ab23...

Timestamp :
2026-08-01

Compiler Version :
Maria 0.9

Flags :
-DTOP

Dependencies :
32

Lookup O(1).

Lexer Cache
lexer_cache/

Setiap file menghasilkan

token stream
module.sv

↓

module.lex

Berisi

identifier

keyword

number

location

comments(optional)

Compile kedua

hash sama

↓

langsung load token

skip lexer
Parser Cache
module.ast

Berisi AST yang telah diserialisasi.

Module

Always

Assign

Generate

Typedef

Class

Package

Function

Tidak parse ulang.

Symbol Database
symbol.mdb

Berisi seluruh simbol.

Package

Module

Function

Task

Typedef

Class

Enum

Struct

Parameter

Macro

Disimpan menggunakan

SymbolID

bukan string.

Misal

logic

↓

ID 123


Semua lookup integer.

Type Database
types.mdb

Berisi

Resolved Type

Packed Width

Signed

Base Type

Array Info

Class Info

Misalnya

logic [31:0]

↓

TypeID 10

Semua node AST hanya menyimpan

TypeID
Dependency Graph

Ini inti incremental compile.

graph.mdb

Misalnya

uart.sv

↓

depends

↓

package_a

↓

defines.svh

↓

interface.sv

Disimpan seperti

Node

Edge

Reverse Edge

Misal

Node

123

↓

Edge

45

↓

Edge

67

Saat

defines.svh

berubah

langsung tahu

uart.sv

cpu.sv

dma.sv

perlu rebuild

Tanpa scan semua project.

AST Arena

Semua AST tidak disimpan pointer.

Tetapi

NodeID
ASTNode

ID

Kind

Parent

Child

Sibling

Contoh

ID

100

Kind

Always

Parent

50

Child

101

Portable.

Mudah di mmap.

Memory Mapped Database

Semua database memakai

mmap
metadata.mdb

↓

Memory Map

↓

langsung pointer

Tidak read()

Tidak deserialize.

Startup sangat cepat.

Diagnostics Database
diagnostics.mdb

Berisi

Error

Warning

Hint

Note

Fix-it

Terhubung ke

AST Node

SymbolID

FileID

Jadi IDE cukup query

Node 456

↓

diagnostic

Tidak compile ulang.

Verification Cache

OpenTitan compile besar.

Misalnya

alu.sv

sudah diverifikasi

Hasilnya

verify/

alu.verify

Berisi

Coverage

Lint

Dataflow

Race

Width Check

FSM Check

Jika

hash sama

langsung

reuse

Tidak lint ulang.

Incremental Resolver

Saat compile

Changed Files

↓

Dependency Graph

↓

Dirty Node

↓

Type Resolver

↓

Verifier

↓

Done

Misalnya

4000 file

↓

ubah

uart.sv

↓

compile

4 file

bukan

4000 file
Snapshot Database
snapshots/

build001

build002

build003

Mirip Git commit.

Berisi

metadata

symbols

graph

types

Rollback sangat cepat.

Binary Layout

Semua file menggunakan format tetap.

Header

Magic

MDB1

Version

Checksum

Compression

Offset Table

Object Table

Payload

Misalnya

+----------------+
| Header         |
+----------------+
| Offset Table   |
+----------------+
| Objects        |
+----------------+
| String Pool    |
+----------------+
| Blob           |
+----------------+
Compiler Pipeline
Source

↓

Hash

↓

Lexer Cache

↓

Parser Cache

↓

Dependency Graph

↓

Type Resolver

↓

Semantic Cache

↓

Verification Cache

↓

Output

Jika hash identik:

Source

↓

Hash Match

↓

Load AST

↓

Skip Lexer

↓

Skip Parser

↓

Skip Semantic

↓

Skip Verification

↓

Generate Output

Cold compile pertama memang tetap memproses seluruh proyek. Namun setelah basis data terbangun, perubahan satu file hanya memicu rekalkulasi pada node yang benar-benar terdampak berdasarkan dependency graph, bukan seluruh proyek.

Struktur Internal yang Disarankan

Daripada satu berkas .mdb raksasa, lebih baik gunakan beberapa engine yang dioptimalkan untuk jenis data masing-masing:

Database	Fungsi	Struktur Data
metadata.mdb	Informasi file & hash	B+ Tree
symbols.mdb	Semua simbol	Adaptive Radix Tree (ART) + String Pool
types.mdb	Type resolution	Arena + HashMap
graph.mdb	Dependency graph	Compressed Sparse Row (CSR) + Reverse Index
ast.mdb	AST terserialisasi	Arena Allocation
diag.mdb	Diagnostics	Append-only Log
cache.mdb	Lexer/Parser/Semantic cache	Content-addressable storage (Blake3)
verify.mdb	Hasil verifikasi	Key-Value Binary Store
Fitur yang Akan Membedakan Maria

Dengan arsitektur ini, Maria dapat memiliki kemampuan yang biasanya hanya ditemukan pada compiler komersial:

File-level incremental compilation berbasis hash konten, bukan timestamp.
Structural incremental compilation, sehingga perubahan lokal pada satu modul tidak memaksa seluruh AST dibangun ulang.
Persistent type-resolution cache, sehingga simbol dan tipe yang tidak berubah tidak perlu diselesaikan ulang.
Memory-mapped object database untuk startup dan query yang sangat cepat.
Parallel-safe readers sehingga IDE, LSP, dan compiler dapat membaca basis data secara bersamaan.
Snapshot build untuk rollback dan perbandingan hasil kompilasi.
Verification cache yang menghindari menjalankan ulang analisis lint, width checking, dataflow, dan pemeriksaan lain pada modul yang identik.

Nilai Keseluruhan
Aspek	Nilai
Arsitektur	9.5/10
Skalabilitas	9/10
Maintainability	8.5/10
Incremental Design	10/10
Data Layout	9.5/10
Enterprise Readiness	8/10
Research Value	9.5/10
Production Ready	7.5/10
Kritik 1 (Critical)
Hash saja belum cukup

Saat ini pipeline:

Source
↓

Hash
↓

Reuse

Ini terlalu sederhana.

Compiler modern tidak hanya memakai file hash.

Mereka memiliki beberapa level.

Misalnya

Content Hash

AST Hash

Semantic Hash

Type Hash

IR Hash

Contoh

comment berubah

↓

content hash berubah

↓

AST identik

↓

semantic identik

↓

verification tetap reuse

Kalau hanya memakai file hash,

ubah komentar

↓

compile ulang

itu membuang performa.

Severity: Critical

Kritik 2 (Critical)

Dependency Graph masih terlalu kasar.

Saat ini

uart.sv

↓

depends

↓

package_a

Tetapi dependency sebaiknya sampai level simbol.

Misalnya

uart

↓

import pkg

↓

parameter WIDTH

↓

typedef DATA

↓

function crc


bukan hanya file.

Kalau package berubah sedikit,

tidak semua module perlu rebuild.

Kritik 3 (Critical)

Belum ada Versioned Schema.

Misalnya

Maria 0.9

↓

Maria 1.0


AST berubah.

Database lama rusak.

Harus ada

Schema Version

AST Version

Type Version

IR Version
Kritik 4 (Critical)

Belum ada Transaction System.

Misalnya compile gagal.

Database sudah setengah ditulis.

metadata ✔

symbol ✔

graph ✖


Database korup.

Harus ada

Begin Transaction

↓

Temporary Pages

↓

Commit

↓

Atomic Rename
Kritik 5 (Critical)

Belum ada Crash Recovery.

Misalnya listrik mati.

Harus ada

Journal

atau

Write Ahead Log

Kalau tidak,

database rusak.

Kritik 6 (High)

Belum ada Garbage Collection.

Cache akan terus bertambah.

Misalnya

200 compile

↓

20GB cache

↓

50GB cache

↓

120GB cache

Harus ada

LRU

TTL

Reference Count

Compaction
Kritik 7 (High)

Belum ada Concurrency Model.

Misalnya

Compiler

IDE

LSP

GUI

Verifier

akses

symbol.mdb

bersamaan.

Siapa yang lock?

Siapa reader?

Siapa writer?

Kalau salah,

deadlock.

Kritik 8 (High)

Belum ada Parallel Scheduler.

Sekarang

Changed

↓

Compile

Padahal

Changed

↓

Dependency Analysis

↓

Task Graph

↓

Worker Queue

↓

Work Stealing

↓

Compile

lebih scalable.

Kritik 9 (High)

Verification Cache terlalu sederhana.

Harus dipisah.

Lint

Coverage

Formal

CDC

RDC

Width

FSM

Timing


Jangan satu blob.

Kritik 10 (High)

AST Arena bagus.

Tetapi

NodeID

Parent

Child

Sibling


kurang.

Tambahkan

Span

TypeID

Flags

SymbolID

Source Range


agar IDE tidak perlu lookup berkali-kali.

Kritik 11 (Medium)

String Pool.

Harus

interning

dedup

readonly

mmap

Kalau tidak,

memory besar.

Kritik 12 (Medium)

Diagnostics

Perlu

Fix-it

Related Info

Code Action

Primary Span

Secondary Span

bukan hanya Error.

Kritik 13 (Medium)

Snapshot

Saat ini

build001

build002

Lebih baik

DAG

bukan linear.


Mirip Git.

Kritik 14 (Medium)

Belum ada Statistics Database.

Idealnya

compile time

memory

cache hit

cache miss

parallelism

worker idle

verification time

AST size

symbol count


Semuanya dicatat.

Kritik 15 (Low)

Belum ada Compression Strategy.

Misalnya

AST

LZ4

Graph

Zstd

Blob

No Compression

Tidak semua data cocok memakai algoritma yang sama.

Hal yang Sangat Bagus

Beberapa keputusan desain menunjukkan pemahaman yang baik tentang compiler modern:

Menggunakan object database daripada SQL sangat masuk akal untuk workload compiler.
Memisahkan metadata, symbols, types, graph, dan diagnostics menghindari bottleneck satu berkas besar.
mmap untuk startup cepat merupakan pilihan yang tepat.
NodeID menggantikan pointer membuat data lebih portabel dan ramah untuk memory mapping.
Verification cache dan dependency graph adalah fondasi penting untuk incremental compilation skala besar.
Fitur yang Masih Hilang agar Setara Compiler Industri

Jika targetmu benar-benar kelas Synopsys VCS, Cadence Xcelium, atau compiler modern lain, saya akan menambahkan:

Content-addressable object store (berbasis Blake3 atau serupa).
Multi-level hash (content, AST, semantic, IR).
Fine-grained dependency graph hingga level simbol.
MVCC atau model transaksi untuk akses paralel.
Write-ahead log dan crash recovery.
Background cache compaction dan garbage collection.
Distributed build cache untuk berbagi hasil antar mesin.
Telemetri performa internal untuk analisis bottleneck.
Binary schema evolution agar format database dapat berubah tanpa memaksa rebuild total.
API publik yang stabil sehingga GUI, LSP, debugger, dan tool lain dapat mengakses MICD tanpa bergantung langsung pada implementasi internal.
Verdict

Saat ini: 8.8/10.

Arsitektur dasarnya sudah berada di jalur yang benar dan jauh lebih dekat ke desain compiler modern daripada sekadar "database cache". Namun masih ada beberapa komponen yang di industri dianggap wajib, terutama transaksi, pemulihan dari crash, konkurensi, evolusi skema, dan dependency graph yang lebih halus. Tanpa itu, MICD akan bekerja sangat baik untuk proyek besar, tetapi mulai menunjukkan batasnya ketika dipakai oleh banyak engineer secara bersamaan atau ketika ukuran proyek dan frekuensi perubahan terus meningkat.
Saya akan mendesain cache/ seperti ini.

.maria/
└── database/
    └── cache/
        ├── lexer/
        ├── parser/
        ├── semantic/
        ├── elaborate/
        ├── optimize/
        ├── verify/
        ├── preprocess/
        ├── macro/
        ├── include/
        ├── dependency/
        ├── resolve/
        ├── constant/
        ├── generate/
        ├── expression/
        ├── type/
        ├── hierarchy/
        ├── simulation/
        ├── waveform/
        ├── coverage/
        ├── lint/
        └── profile/
1. preprocess/

Cache hasil preprocessing.

source

↓

`define
`ifdef
`include

↓

preprocessed text

↓

preprocess cache

Disimpan:

Expanded source
Include list
Define table
Conditional branch
Hash

Jika hanya file lain berubah, hasil preprocess yang identik bisa langsung digunakan.

2. lexer/

Bukan hanya token.

module.lex

Berisi

TokenID

Kind

Location

Trivia

Whitespace

Comment

Macro Expansion

Line Mapping

IDE dapat membaca token tanpa menjalankan lexer.

3. parser/

Jangan hanya AST.

Simpan juga

Parse Tree

AST

Syntax Error

Recovery Point

Node Offset

Recovery point mempercepat reparsing saat ada error.

4. semantic/

Ini cache terbesar.

Berisi

Resolved Symbol

Resolved Type

Scope

Visibility

Width

Constant Value

Evaluation Result

Diagnostic

Jika semantic hash sama

langsung reuse.

5. elaborate/

Ini sangat penting untuk Maria.

Berisi

Generate Expansion

Parameter Override

Module Instance

Hierarchy

Port Binding

Net Resolution

Always Expansion

Misalnya

generate

for

1000 instance

tidak perlu dielaborasi ulang.

6. optimize/

Berisi

Constant Folding

Dead Code

Unused Wire

Propagation

Flatten Result

Loop Unroll

Expression Simplification

Kalau hasil optimize identik

skip.

7. verify/

Pisahkan menjadi

verify/

    lint/

    width/

    race/

    xprop/

    cdc/

    fsm/

    dataflow/

    timing/

    coverage/

    assertion/

Jangan satu file besar.

8. dependency/

Berisi

Forward Edge

Reverse Edge

Import

Include

Parameter Dependency

Generate Dependency

Supaya scheduler tidak membuka graph utama.

9. resolve/

Cache resolver.

logic

↓

TypeID

WIDTH

↓

ConstantID

pkg::abc

↓

SymbolID

Lookup O(1).

10. expression/

Cache evaluasi expression.

4+5

↓

9
WIDTH*8

↓

256

Compiler modern melakukan ini jutaan kali.

11. constant/

Semua hasil

Constant Folding

Parameter

Localparam

Enum Value

disimpan.

12. type/
logic

logic[7:0]

struct

union

enum

packed

unpacked

class

interface

Semua punya TypeID.

13. macro/

Cache

define

undef

macro body

argument

expansion

Macro sangat mahal.

14. include/
include tree

↓

dependency

↓

hash

Kalau include tidak berubah

skip scan.

15. hierarchy/
Top

↓

CPU

↓

ALU

↓

Adder


Tidak perlu dibangun ulang.

16. generate/

Cache

Generate If

Generate For

Generate Case

Ini akan sangat mengurangi bottleneck elaboration.

17. simulation/

Cache

Initial State

Resolved Event

Compiled Scheduler

Timewheel

Sensitivity List

Jika hanya RTL kecil berubah

scheduler bisa reuse sebagian.

18. waveform/
Signal Index

Hierarchy

Variable Metadata

Alias

Type

Sehingga VCD/FST lebih cepat dibuka.

19. coverage/
Branch

Toggle

FSM

Statement

Condition

Expression

Coverage identik

reuse.

20. profile/

Ini yang sering dilupakan.

Setiap compile simpan

Compile Time

Lexer Time

Parser Time

Semantic Time

Elaboration Time

Optimization Time

Verification Time

Memory

CPU

Worker Utilization

Cache Hit

Cache Miss

Rebuild Count

Dirty Node Count

Dari sini Maria bisa mengetahui sendiri bottleneck dan bahkan memberikan rekomendasi optimasi.

Saran arsitektur cache

Saya juga menyarankan setiap cache memiliki struktur internal yang seragam agar mudah dikelola:

cache/
└── parser/
    ├── objects/
    ├── index/
    ├── blobs/
    ├── temp/
    ├── journal/
    ├── stats/
    ├── lock/
    └── manifest.mdb
objects/: menyimpan objek cache berdasarkan content hash (mis. Blake3).
index/: memetakan FileID atau NodeID ke objek cache.
blobs/: data besar seperti AST atau hasil elaborasi.
temp/: cache yang sedang dibangun sebelum commit.
journal/: log transaksi untuk recovery jika proses terhenti.
stats/: statistik hit/miss, ukuran, dan umur cache.
lock/: koordinasi reader/writer untuk akses paralel.
manifest.mdb: metadata cache, versi skema, checksum, dan konfigurasi