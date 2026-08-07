Desain 5. Enterprise Context Architecture (Paling Standar Industri)

Setiap domain memiliki context sendiri sehingga tidak ada satu Env yang menjadi terlalu besar.

env/
├── mod.rs
│
├── global/
│   ├── global_env.rs
│   ├── version.rs
│   ├── build.rs
│   └── startup.rs
│
├── workspace/
│   ├── workspace.rs
│   ├── filelist.rs
│   ├── project.rs
│   └── include.rs
│
├── runtime/
│   ├── runtime.rs
│   ├── cpu.rs
│   ├── memory.rs
│   ├── threadpool.rs
│   └── scheduler.rs
│
├── compiler/
│   ├── preprocess.rs
│   ├── lexer.rs
│   ├── parser.rs
│   ├── ast.rs
│   ├── hir.rs
│   ├── elaboration.rs
│   └── optimize.rs
│
├── verification/
│   ├── lint.rs
│   ├── semantic.rs
│   ├── typecheck.rs
│   ├── xprop.rs
│   └── assertions.rs
│
├── simulation/
│   ├── kernel.rs
│   ├── event_queue.rs
│   ├── waveform.rs
│   ├── dpi.rs
│   └── coverage.rs
│
├── cache/
│   ├── cache.rs
│   ├── incremental.rs
│   ├── artifact.rs
│   └── fingerprint.rs
│
├── database/
│   ├── micd.rs
│   ├── symbol_db.rs
│   ├── graph_db.rs
│   ├── diagnostics_db.rs
│   └── metadata_db.rs
│
├── diagnostics/
│   ├── emitter.rs
│   ├── formatter.rs
│   ├── warning.rs
│   ├── error.rs
│   └── statistics.rs
│
├── telemetry/
│   ├── profiler.rs
│   ├── tracing.rs
│   ├── metrics.rs
│   └── performance.rs
│
├── plugins/
│   ├── manager.rs
│   ├── registry.rs
│   └── sandbox.rs
│
└── security/
    ├── permissions.rs
    ├── sandbox.rs
    └── validation.rs
Hubungan antar context
GlobalEnv
│
├── WorkspaceContext
├── RuntimeContext
├── CompilerContext
├── VerificationContext
├── SimulationContext
├── CacheContext
├── DatabaseContext
├── DiagnosticsContext
├── TelemetryContext
├── PluginContext
└── SecurityContext
Kelebihan desain 5
Tidak ada MariaEnv berukuran raksasa dengan ratusan field.
Setiap tahap pipeline memiliki context yang jelas.
Mendukung incremental compilation melalui MICD.
Mudah diparalelkan karena context dapat dipisahkan per pekerjaan.
Sangat cocok untuk proyek besar seperti OpenTitan, XiangShan, CVA6, atau SoC dengan puluhan ribu modul.
Mudah menambahkan fitur baru tanpa mengubah struktur inti.


Filosofi Arsitektur

Alih-alih seperti ini:

pub struct MariaEnv {
    pub config: Config,
    pub cache: Cache,
    pub parser: Parser,
    pub ast: Ast,
    pub symbols: Symbols,
    pub diagnostics: Diagnostics,
    pub scheduler: Scheduler,
    pub thread_pool: ThreadPool,
    pub telemetry: Telemetry,
    pub profiler: Profiler,
    pub simulation: Simulation,
    pub dpi: Dpi,
    pub xprop: XProp,
    // ...
    // 300 field...
}

diganti menjadi

Maria
 │
 └── GlobalEnv
      │
      ├── WorkspaceContext
      ├── RuntimeContext
      ├── ConfigContext
      ├── CompilerContext
      ├── VerificationContext
      ├── SimulationContext
      ├── CacheContext
      ├── DatabaseContext
      ├── DiagnosticsContext
      ├── TelemetryContext
      ├── PluginContext
      └── SecurityContext

Setiap Context adalah subsystem.

Struktur Lengkap
env/

├── mod.rs
│
├── global/
│
├── config/
│
├── workspace/
│
├── runtime/
│
├── compiler/
│
├── verification/
│
├── simulation/
│
├── cache/
│
├── database/
│
├── diagnostics/
│
├── telemetry/
│
├── plugins/
│
└── security/

Masing-masing memiliki tugas sendiri.

1. Global Context
global/

global_env.rs
startup.rs
shutdown.rs
build.rs
version.rs

Ini adalah root object.

Contoh

pub struct GlobalEnv {

    pub config: Arc<ConfigContext>,

    pub workspace: Arc<WorkspaceContext>,

    pub runtime: Arc<RuntimeContext>,

    pub compiler: Arc<CompilerContext>,

    pub verification: Arc<VerificationContext>,

    pub simulation: Arc<SimulationContext>,

    pub cache: Arc<CacheContext>,

    pub database: Arc<DatabaseContext>,

    pub diagnostics: Arc<DiagnosticsContext>,

    pub telemetry: Arc<TelemetryContext>,

    pub plugins: Arc<PluginContext>,

    pub security: Arc<SecurityContext>,
}

GlobalEnv tidak memiliki logika compiler.

Ia hanya menyimpan service.

2. Config Context
config/

config.rs

defaults.rs

loader.rs

validator.rs

cli.rs

environment.rs

Tugasnya

membaca CLI
membaca TOML
membaca YAML
membaca JSON
membaca ENV

misalnya

Maria.toml

↓

ConfigContext

↓

CompilerContext

Compiler tidak membaca file.

Compiler cukup bertanya

config.max_threads

config.incremental

config.sim_timeout
3. Workspace Context
workspace/

workspace.rs

project.rs

include.rs

search.rs

filelist.rs

Mengelola

workspace/

src/

rtl/

tb/

ip/

third_party/

juga

-filelist

+incdir+

library

root path

output path
4. Runtime Context

Ini mengatur seluruh resource.

runtime/

cpu.rs

memory.rs

threadpool.rs

scheduler.rs

allocator.rs

gpu.rs

Misalnya

CPU

RAM

Thread

NUMA

Hugepage

GPU

Temp Directory

Compiler cukup meminta

runtime.thread_pool.spawn(...)

tanpa tahu implementasi thread.

5. Compiler Context

Inilah pipeline compiler.

compiler/

preprocess.rs

lexer.rs

parser.rs

ast.rs

hir.rs

symbol.rs

type.rs

elaboration.rs

optimizer.rs

Diagram

Source

↓

Preprocessor

↓

Lexer

↓

Parser

↓

AST

↓

HIR

↓

Symbol

↓

Type

↓

Elaboration

↓

Optimizer

Compiler tidak tahu database.

Compiler tidak tahu GUI.

Compiler tidak tahu logger.

Semua lewat Context.

6. Verification Context
verification/

lint.rs

semantic.rs

assertions.rs

formal.rs

coverage.rs

xprop.rs

Berisi

Lint

Semantic

Assertion

Coverage

Formal

CDC

RDC

X propagation

Semua checker ada di sini.

7. Simulation Context
simulation/

kernel.rs

scheduler.rs

waveform.rs

dpi.rs

coverage.rs

timewheel.rs

event_queue.rs

Diagram

Simulator

↓

Kernel

↓

Event Queue

↓

Time Wheel

↓

Waveform

↓

Coverage

↓

DPI
8. Cache Context
cache/

fingerprint.rs

artifact.rs

incremental.rs

storage.rs

eviction.rs

Ini mengatur

Token Cache

AST Cache

HIR Cache

Type Cache

Dependency Cache

Object Cache

semuanya.

9. Database Context

Khusus MICD.

database/

micd.rs

ast_db.rs

symbol_db.rs

graph_db.rs

metadata_db.rs

diagnostics_db.rs

Semua persistent storage ada di sini.

Compiler tidak pernah membuka file database secara langsung.

10. Diagnostics Context
diagnostics/

error.rs

warning.rs

formatter.rs

emitter.rs

statistics.rs

Diagram

Compiler

↓

Diagnostic Builder

↓

Formatter

↓

Reporter

↓

CLI

GUI

JSON
11. Telemetry Context
telemetry/

profiler.rs

metrics.rs

trace.rs

performance.rs

timeline.rs

Contoh

Lexer

23 ms

Parser

311 ms

Elaboration

721 ms

Simulation

9.2 s

semuanya otomatis tercatat.

12. Plugin Context
plugins/

manager.rs

registry.rs

loader.rs

sandbox.rs

Untuk

Plugin Lint

Plugin GUI

Plugin AI

Plugin Coverage

Plugin Export
13. Security Context
security/

permission.rs

sandbox.rs

validation.rs

signature.rs

Mengatur

Permission

Sandbox

Trusted Plugin

Hash

Signature
Dependency Rule (Sangat Penting)

Aturan yang sebaiknya diterapkan adalah dependency satu arah agar tidak terjadi circular dependency.

Config
    │
    ▼
Workspace
    │
    ▼
Runtime
    │
    ▼
Compiler
    │
    ├────────► Cache
    │
    ├────────► Database
    │
    ├────────► Diagnostics
    │
    ├────────► Telemetry
    │
    ▼
Verification
    │
    ▼
Simulation

Sebaliknya:

Cache tidak boleh memanggil Compiler.
Database tidak boleh mengetahui Parser.
Diagnostics hanya menerima data hasil proses, bukan menjalankan parser.
Telemetry hanya mengamati, tidak mengubah perilaku sistem.
Lifecycle

Urutan hidup sistem sebaiknya konsisten:

Startup
   │
   ▼
Load Config
   │
   ▼
Open Workspace
   │
   ▼
Initialize Runtime
   │
   ▼
Open Database (MICD)
   │
   ▼
Initialize Cache
   │
   ▼
Load Plugins
   │
   ▼
Compile
   │
   ▼
Verify
   │
   ▼
Simulate
   │
   ▼
Flush Diagnostics
   │
   ▼
Write Metrics
   │
   ▼
Shutdown
Mengapa cocok untuk Maria?

Berdasarkan diskusi kita sebelumnya tentang Maria, Anda memiliki target seperti:

menangani ribuan hingga puluhan ribu modul RTL,
memiliki MICD sebagai database incremental,
mendukung elaboration, verification, dan simulation,
menyediakan GUI dan CLI,
melakukan profiling bottleneck,
serta berkembang menjadi tool industri.