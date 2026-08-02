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

        database/

            metadata.mdb

            symbol.mdb

            ast.mdb

            types.mdb

            graph.mdb

            diagnostics.mdb

            cache/

                lexer/

                parser/

                semantic/

                verify/

                optimize/

            snapshots/

                build-001

                build-002

Semuanya binary.

Tidak ada SQL.

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