Saya akan menyebutnya MDB (Maria Database). Ini bukan pengganti SQL seperti MariaDB, melainkan database khusus untuk analisis HDL.

Tujuan
Incremental compile
Cache AST
Cache preprocessing
Symbol index
Cross-reference
Dependency graph
Error database
Wave metadata
Workspace database
Struktur
project.mdb
│
├── Header
├── File Table
├── Module Table
├── Package Table
├── Interface Table
├── Class Table
├── Symbol Table
├── Dependency Graph
├── AST Cache
├── Semantic Cache
├── Elaborated Design
├── Diagnostics
├── Benchmark
├── User Metadata
└── Free Space
Header
magic = MDB0

version = 1

compiler_version

created

modified

checksum

compression

endianness
File Table

Menyimpan seluruh file.

FileID

Path

Hash

Timestamp

Language

Size

Flags
Module Table
ModuleID

Name

FileID

LineStart

LineEnd

Visibility

ParameterCount

PortCount

Checksum
Package Table
PackageID

Imports

Exports

Hash
Symbol Table

Ini paling penting.

SymbolID

Name

Kind

Owner

File

Offset

Type

Flags

Kind:

module
class
interface
package
enum
typedef
variable
function
task
parameter
port
Dependency Graph
ModuleA

imports

PackageB

ModuleA

instantiate

ModuleC

ModuleD

include

HeaderE

Disimpan sebagai graph sehingga pencarian dependency hampir instan.

AST Cache

Daripada parse ulang.

NodeID

NodeKind

Parent

Children

Location

Hash

Jika file tidak berubah:

langsung load AST

Tidak perlu parse lagi.

Semantic Cache

Berisi hasil semantic analysis.

resolved type

constant value

scope

template expansion

typedef

parameter result
Elaborated Design

Setelah elaboration selesai.

Top

Instance Tree

Netlist

Resolved Parameters

Generated Blocks

Compile berikutnya tinggal membaca cache.

Diagnostics
ErrorID

Severity

File

Line

Column

Message

Code

Fix
Benchmark
Lex Time

Parse Time

Semantic Time

Elaboration Time

Optimization Time

Memory Peak

CPU Usage
User Metadata
Bookmarks

Recent Files

Editor State

Breakpoints

Layout

Project Settings
Penyimpanan
Page Size

16 KB

Semua data memakai sistem page seperti SQLite.

Page

↓

Record

↓

Offset

Tidak perlu membaca seluruh file.

Kompresi

Setiap section memakai:

Zstd
LZ4
Tanpa kompresi (debug)
Incremental Compile

Saat file berubah:

Hash berubah

↓

Reparse file tersebut

↓

Update AST

↓

Update Symbol

↓

Update Dependency

↓

Compile modul terdampak saja

Bukan seluruh proyek 3.970 file. Komputer akhirnya diberi kesempatan hidup beberapa detik lebih lama.

API Internal Rust
let db = Mdb::open("project.mdb")?;

db.add_file(...)?;
db.add_module(...)?;
db.update_symbol(...);

let ast = db.load_ast(file_id)?;
let deps = db.dependencies(module_id)?;
Format File
*.mdb

dengan beberapa file pendukung:

project.mdb        // Database utama
project.mdb-lock   // Lock file
project.mdb-wal    // Write-ahead log
project.mdb-cache  // Cache sementara
project.mdb-index  // Indeks cepat
Keunggulan desain
Sangat cepat untuk proyek besar (10.000+ modul).
Mendukung incremental compile sehingga hanya bagian yang berubah diproses ulang.
Cache AST, semantic, dan elaboration mengurangi parsing berulang.
Dependency graph mempercepat analisis dampak perubahan.
Dirancang khusus untuk SystemVerilog, Verilog, VHDL, dan bahasa HDL lain yang didukung Maria, sehingga lebih efisien dibanding memakai database SQL umum.
