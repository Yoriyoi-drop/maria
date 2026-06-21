# Emulator — Aurora SoC + Hermes GPU

Fast model untuk dua project RTL:

| Project | Path | Deskripsi |
|---------|------|-----------|
| **Aurora-172** | `/home/whale-d/aurora-172` | SoC heterogen: G-Core, H-Core, A-Core, NPU, memory fabric, interconnect, RT engine |
| **Hermes GPU** | `/home/whale-d/hermes` | GPU SIMT: 8 warp, 32x32 systolic array, FP16/BF16/INT8 |

Emulator bisa jalanin **Zenus OS** + **Hermes GPU driver** sebelum chip fisik ada.
Co-simulation dengan maria RTL untuk verifikasi.

## Target Skor

| Kategori | Nilai |
|----------|-------|
| CPU Architecture | 9.5/10 |
| Linux Readiness | 9.5/10 |
| SMP | 9/10 |
| Debugging | 9.5/10 |
| Co-Simulation | 9.5/10 |
| Extensibility | 9.5/10 |
| Performance | 9/10 |
| Production Readiness | 9/10 |
| Maintainability | 9.5/10 |

## Pipeline

```
RTL Design (.sv)                    Emulator (fast model)
─────────────────────               ─────────────────────
aurora-172/ + hermes/               program.elf / kernel.bin
    ↓                                   ↓
maria RTL Simulator                  ELF Loader
    ↓                                   ↓
VCD Output (cycle-accurate)          Core Multiplexer
    ↓                                   ↓
Co-Sim: match RTL vs Emulator ←──     ├── H-Core (RV64GC JIT + Interpreter)
                                       ├── G-Core (Custom ISA interpreter)
                                       ├── A-Core (Tensor ISA interpreter)
                                       ├── NPU (Inference ISA interpreter)
                                       ├── Hermes GPU (SIMT interpreter)
                                       │   ├── Warp scheduler
                                       │   ├── Tensor core (systolic array)
                                       │   ├── Vector unit
                                       │   └── Scalar unit
                                       ├── Memory fabric (L1/L2, MESI, NoC)
                                       ├── Interconnect (ring bus, NoC mesh)
                                       └── RT engine
```

## Struktur Direktori

```
src/emulator/
├── mod.rs                   # Entry, main loop, event loop
│
├── core/
│   ├── mod.rs               # Core trait + dispatch
│   ├── h_core/              # H-Core: RV64GC
│   │   ├── hart.rs
│   │   ├── decoder.rs
│   │   ├── jit.rs           # Cranelift
│   │   ├── interpreter.rs
│   │   ├── csr.rs
│   │   ├── trap.rs
│   │   ├── privilege.rs
│   │   ├── atomic.rs
│   │   └── fpu.rs
│   ├── g_core/              # G-Core: gaming ISA (opcodes 0x01-0x07)
│   │   ├── core.rs
│   │   ├── decoder.rs       # DRAW, TEXTURE, PHYSICS, COLLISION, RAYTRACE, FRAMEGEN, SHADING
│   │   ├── units/
│   │   │   ├── draw.rs
│   │   │   ├── texture.rs
│   │   │   ├── physics.rs
│   │   │   └── shading.rs
│   │   └── branch_predictor.rs
│   ├── a_core/              # A-Core: AI/tensor ISA (opcodes 0x20-0x25)
│   │   ├── core.rs
│   │   ├── decoder.rs       # MATMUL, ATTENTION, CONV2D, POOLING, ACTIVATION, NORMALIZE
│   │   └── tensor_unit.rs
│   └── npu/                 # NPU: inference ISA (opcodes 0x40-0x48)
│       ├── cluster.rs
│       ├── decoder.rs       # NOP, INFERENCE, CONV, POOL, RELU, etc.
│       └── systolic_array.rs
│
├── gpu/                     # Hermes GPU
│   ├── hermes.rs            # Top-level GPU state
│   ├── warp.rs              # Warp state (8 warps)
│   ├── warp_scheduler.rs    # Round-robin scheduler
│   ├── tensor_core.rs       # 32x32 systolic array (1024 MACs)
│   ├── vector_unit.rs       # VADD/VSUB/VMUL/VRELU/VSIGMOID/VTANH/VCONV
│   ├── scalar_unit.rs       # SADD/SSUB/SMUL/SMOV/SBRA
│   ├── decoder.rs           # 64-bit instruction decode
│   ├── register_file.rs     # 8 warps × 1024 regs × 32 lanes
│   ├── mem_hierarchy.rs     # Arbiter → TLB + shared mem + L1 → L2 → DRAM
│   └── renderer.rs          # Framebuffer output (SDL / dump)
│
├── memory/
│   ├── bus.rs               # Address routing + device dispatch
│   ├── ram.rs               # RAM backend
│   ├── rom.rs               # Boot ROM
│   ├── cache.rs             # L1/L2 cache model
│   ├── mesi.rs              # MESI coherency protocol
│   ├── dma.rs               # DMA engine
│   ├── prefetcher.rs        # Hardware prefetcher
│   ├── mmu.rs               # Sv39 + Sv48
│   ├── tlb.rs               # iTLB + dTLB, ASID
│   └── snapshot.rs          # Save/load state
│
├── interconnect/
│   ├── fabric.rs            # Aurora fabric
│   ├── ring_bus.rs          # Ring bus
│   ├── noc.rs               # NoC mesh / router
│   └── scheduler.rs         # MQ / SQ global scheduler
│
├── devices/
│   ├── mod.rs               # Device trait + event system
│   ├── uart.rs              # 16550 UART
│   ├── clint.rs             # CLINT timer
│   ├── plic.rs              # PLIC interrupt controller
│   ├── virtio_blk.rs        # VirtIO block
│   ├── virtio_net.rs        # VirtIO net
│   ├── rtc.rs               # Real-time clock
│   └── fdt.rs               # Flattened Device Tree
│
├── debug/
│   ├── gdb.rs               # GDB stub (tcp:1234, RISC-V only)
│   ├── trace.rs             # Instruction trace per core
│   ├── profiler.rs           # Hot block detection
│   ├── disasm.rs            # Disassembler per ISA
│   └── coverage.rs          # PC histogram
│
├── cosim/
│   ├── compare.rs           # State comparison
│   ├── rtl_bridge.rs        # Ambil signal dari maria engine
│   └── mismatch.rs          # Mismatch logging
│
├── loader/
│   ├── elf.rs               # ELF parser
│   ├── opensbi.rs           # OpenSBI firmware
│   └── linux.rs             # Linux boot protocol
│
└── platform/
    ├── aurora.rs            # Aurora SoC (semua core + fabric + devices)
    ├── hermes.rs            # Hermes GPU standalone
    └── config.rs            # Platform config builder
```

## Core Architecture

### H-Core (RISC-V RV64GC)

Sama seperti desain sebelumnya — **JIT Cranelift + Interpreter fallback**.

- Decoder RV64GC (I, M, A, F, D, C)
- Cranelift JIT per basic block
- Interpreter fallback untuk JIT bug / instruksi rare
- SMP hingga 32 hart
- Sv39/Sv48 MMU
- Trap & interrupt handling
- GDB stub

### G-Core (Gaming)

Custom ISA dengan opcode:

| Opcode | Instruksi | Fungsi |
|--------|-----------|--------|
| `0x01` | DRAW | Render primitive |
| `0x02` | TEXTURE | Texture mapping |
| `0x03` | PHYSICS | Physics simulation |
| `0x04` | COLLISION | Collision detection |
| `0x05` | RAYTRACE | Ray tracing |
| `0x06` | FRAMEGEN | Frame generation |
| `0x07` | SHADING | Shading |

Implementasi:
- **Interpreter-only** (4 core, jadi JIT overhead tidak sebanding)
- Branch predictor + uop cache
- CET anti-cheat emulation

### A-Core (AI/Tensor)

Custom ISA dengan opcode:

| Opcode | Instruksi | Fungsi |
|--------|-----------|--------|
| `0x20` | MATMUL | Matrix multiply |
| `0x21` | ATTENTION | Attention mechanism |
| `0x22` | CONV2D | 2D convolution |
| `0x23` | POOLING | Pooling layer |
| `0x24` | ACTIVATION | Activation function |
| `0x25` | NORMALIZE | Normalization |

Implementasi:
- **Interpreter** dengan native BLAS acceleration untuk MATMUL
- 64 core, jadi perlu efisien

### NPU

Custom ISA dengan opcode:

| Opcode | Instruksi | Fungsi |
|--------|-----------|--------|
| `0x40` | NOP | No operation |
| `0x41` | INFERENCE | Run inference |
| `0x42` | CONV | Convolution |
| `0x43` | POOL | Pooling |
| `0x44` | RELU | ReLU activation |
| `0x45` | SOFTMAX | Softmax |
| `0x46` | LOAD_W | Load weight |
| `0x47` | STORE_R | Store result |
| `0x48` | SYNC | Synchronize |

Implementasi:
- **Interpreter** dengan native matrix ops
- 8 cluster, masing-masing dengan systolic array sendiri

## Hermes GPU

### Instruction Format (64-bit)

```
[63:59] opcode | [58:57] fmt | [56] pred | [55:51] rd
[50:46] rs1    | [45:41] rs2 | [40:39] wgpr_sel | [31:0] imm
```

Instructions packed 8 per 512-bit DRAM word. PC increments by 8.

### Opcodes

| Opcode | Instruksi |
|--------|-----------|
| `OP_MMA` | Matrix multiply-accumulate |
| `OP_VADD` | Vector add |
| `OP_VSUB` | Vector subtract |
| `OP_VMUL` | Vector multiply |
| `OP_VRELU` | Vector ReLU |
| `OP_VSIGMOID` | Vector sigmoid |
| `OP_VTANH` | Vector tanh |
| `OP_VCONV` | Vector convolution |
| `OP_SADD` | Scalar add |
| `OP_SSUB` | Scalar subtract |
| `OP_SMUL` | Scalar multiply |
| `OP_SMOV` | Scalar move |
| `OP_SBRA` | Scalar branch |
| `OP_LD` | Load (DRAM → register) |
| `OP_ST` | Store (register → DRAM) |
| `OP_LDS` | Load shared memory |
| `OP_STS` | Store shared memory |
| `OP_BAR` | Barrier sync |
| `OP_EXIT` | Exit warp |
| `OP_NOP` | No operation |

### Pipeline Eksekusi

```
Warp Scheduler (round-robin, 8 warps)
    │
    ├── Instruction Decode
    │       │
    │       ├── Tensor: → Tensor Core (32x32 systolic array)
    │       ├── Vector: → Vector Unit (32 lanes)
    │       ├── Scalar: → Scalar Unit (1 lane)
    │       ├── Memory: → Mem Hierarchy (arbiter → TLB → L1 → L2 → DRAM)
    │       └── Sync:   → Barrier/Warp control
    │
    └── Writeback → Register File (8 warps × 1024 regs × 32 lanes)
```

### Memory Hierarchy

```
Warp request → Arbiter
    → [TLB | Shared Memory | L1$ (16KB 4-way)]
    → L2$ (128KB 8-way)
    → DRAM (AXI 512-bit)
```

## Memory Fabric (Aurora)

```
┌─────────────────────────────────────────────────────┐
│                    Interconnect                       │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐           │
│  │ Ring Bus │  │  NoC     │  │ Aurora   │           │
│  │          │  │  Mesh    │  │ Fabric   │           │
│  └──────────┘  └──────────┘  └──────────┘           │
│                                                       │
│  ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐ ┌───────────┐ │
│  │L1$   │ │L1$   │ │L1$   │ │L1$   │ │  L2$      │ │
│  │G-Core│ │H-Core│ │A-Core│ │NPU   │ │  (shared) │ │
│  └──────┘ └──────┘ └──────┘ └──────┘ └───────────┘ │
│                          │                            │
│                   ┌──────┴──────┐                     │
│                   │   MESI      │                     │
│                   │  Coherency  │                     │
│                   └──────┬──────┘                     │
│                          │                            │
│                   ┌──────┴──────┐                     │
│                   │    DMA      │                     │
│                   └─────────────┘                     │
└─────────────────────────────────────────────────────┘
```

### MESI Protocol

```rust
enum MesiState {
    Modified,
    Exclusive,
    Shared,
    Invalid,
}

struct CacheLine {
    state: MesiState,
    tag: u64,
    data: [u8; 64],
    dirty: bool,
}
```

### Interconnect Scheduler

MQ (Multi-Queue) dan SQ (Single-Queue) — dua implementasi untuk A/B comparison, sesuai RTL.

## Alur Eksekusi

```
Emulator::run()
│
├── 1. Load ELF / firmware
├── 2. Setup platform (Aurora / Hermes standalone)
├── 3. Init cores + devices
├── 4. Main loop:
│       loop {
│           // Advance all cores
│           for each core in platform.cores {
│               core.tick()
│           }
│           // Advance GPU (if present)
│           if platform.gpu { gpu.tick() }
//         // Advance interconnect + memory fabric
│           fabric.tick()
│           // Advance devices
│           process_events()
│           // Co-simulation step
│           co_sim.step()
│       }
│
└── 5. Dump state / VCD
```

### Core Trait

```rust
trait Core {
    fn id(&self) -> CoreId;
    fn tick(&mut self) -> CoreResult;
    fn reset(&mut self);
    fn save(&self) -> Vec<u8>;
    fn load(&mut self, data: &[u8]);
    fn attach_bus(&mut self, bus: &mut Bus);
}

enum CoreId {
    HCore(u32),    // 0..31
    GCore(u32),    // 0..3
    ACore(u32),    // 0..63
    NpuCluster(u32), // 0..7
}

struct CoreResult {
    done: bool,
    stalled: bool,
    mem_access: Vec<MemAccess>,
    interrupts: Vec<Interrupt>,
}
```

## Memory Map (Aurora SoC)

| Range | Device |
|-------|--------|
| `0x00000000 – 0x7FFFFFFF` | RAM (2 GB) |
| `0x10000000 – 0x10000FFF` | UART 16550 |
| `0x20000000 – 0x20000FFF` | CLINT |
| `0x20040000 – 0x2004FFFF` | PLIC |
| `0x30000000 – 0x3FFFFFFF` | PCIe MMIO |
| `0x40000000 – 0x4FFFFFFF` | Hermes GPU MMIO |
| `0x50000000 – 0x5FFFFFFF` | NPU MMIO |
| `0x60000000 – 0x6FFFFFFF` | G-Core MMIO |
| `0x70000000 – 0x7FFFFFFF` | A-Core MMIO |
| `0x80000000 – 0x8FFFFFFF` | PCIe ECAM |
| `0xBFE00000` | FDT |

## Event System

```rust
enum Event {
    TimerInterrupt { hart: u64 },
    UartRx,
    DmaDone { channel: u8 },
    GpuVsync,
    GpuKernelDone { warp: u8 },
    ExternalIrq { irq: u32 },
    FabricDeadlock,
}

struct EventQueue {
    events: BinaryHeap<EventEntry>,  // priority queue by time
}

impl EventQueue {
    fn schedule(&mut self, delay_ns: u64, event: Event);
    fn process_pending(&mut self, now: u64) -> Vec<Event>;
}
```

## Co-Simulation

```rust
struct CoSim {
    enabled: bool,
    rtl_signals: SignalMap,
    emu_state: EmuSnapshot,
    mismatches: Vec<Mismatch>,
}

struct EmuSnapshot {
    // Per-core
    h_core: Vec<CoreSnapshot>,
    g_core: Vec<CoreSnapshot>,
    a_core: Vec<CoreSnapshot>,
    npu: Vec<CoreSnapshot>,
    // GPU
    gpu: GpuSnapshot,
    // Memory
    cache_states: Vec<CacheLineSnapshot>,
    fabric_queues: Vec<QueueSnapshot>,
}

struct Mismatch {
    cycle: u64,
    component: String,
    field: String,
    emu_value: String,
    rtl_value: String,
}
```

## GDB Stub

TCP :1234, protocol GDB Remote Serial Protocol.

Khusus H-Core (RISC-V). Support:
- `g` / `G` — read/write registers
- `m` / `M` — memory
- `c` / `s` — continue / step
- `Z0` / `z0` — breakpoint
- Thread list per hart
- Target: `riscv64-linux-gnu-gdb`

## Snapshot

```rust
struct Snapshot {
    timestamp: u64,
    cores: HashMap<CoreId, Vec<u8>>,
    ram: Vec<u8>,
    devices: HashMap<String, Vec<u8>>,
    gpu: Option<Vec<u8>>,
}
```

Format: binary file per komponen. Boot sekali → snapshot → debug tanpa reboot.

## Build Plan (Fase)

### Fase 1 — H-Core + RAM + UART
- Decoder RV64GC (integer + compressed)
- Cranelift JIT (basic block)
- Interpreter fallback
- ELF loader
- RAM backend + Bus
- UART 16550
- **Target:** Hello World bare-metal RISC-V

### Fase 2 — H-Core Trap + MMU + Interrupt
- Trap handling (ecall, page fault, illegal)
- CLINT + PLIC
- Sv39 MMU + TLB
- Privilege M/S/U
- OpenSBI boot
- **Target:** Boot OpenSBI

### Fase 3 — SMP + Atomic + FPU + Snapshot
- Multi-hart H-Core (2/4/8/16/32)
- Atomic (LR/SC, AMO)
- FPU (F/D)
- Snapshot
- **Target:** SMP OpenSBI

### Fase 4 — G-Core + A-Core + NPU
- G-Core interpreter (DRAW, TEXTURE, PHYSICS, SHADING)
- A-Core interpreter (MATMUL, CONV2D, ATTENTION)
- NPU interpreter (INFERENCE, CONV, POOL)
- Memory fabric (L1/L2 cache, MESI)
- Interconnect (ring bus, NoC)
- **Target:** Semua core aktif di Aurora

### Fase 5 — Hermes GPU
- Warp scheduler (8 warps, round-robin)
- Decoder 64-bit instruction
- Tensor core (32x32 systolic)
- Vector unit + Scalar unit
- Register file + mem hierarchy
- Renderer (SDL / dump)
- **Target:** Hermes GPU standalone test

### Fase 6 — PCIe + VirtIO + Linux
- PCIe root complex
- VirtIO block + net
- Linux boot protocol
- FDT generator
- **Target:** Boot Linux di H-Core

### Fase 7 — GDB + Debug + Co-Simulation
- GDB stub (tcp:1234)
- Trace, profiler, coverage
- Co-simulation engine
- riscv-tests
- **Target:** 100% riscv-tests, co-sim dengan RTL

### Fase 8 — GPU + Aurora Integrasi + Zenus OS
- Integrasi Hermes GPU ke Aurora SoC
- Zenus OS boot protocol
- Hermes GPU driver jalan
- **Target:** Zenus OS + Hermes driver di Aurora emulator

### Fase 9 — Production Polish
- Memory poisoning + leak detection
- Crash dump
- Assertions + watchdog
- Performance optimization
- **Target:** Skor 9/10 semua kategori

## Dependensi Cargo

```toml
[dependencies]
cranelift = "0.117"
cranelift-module = "0.117"
cranelift-jit = "0.117"
target-lexicon = "0.13"
goblin = "0.9"                # ELF parser
```

## Testing

```
tests/
├── riscv-tests/         # RISC-V architectural tests (H-Core)
│   ├── rv64ui/          # User-level integer
│   ├── rv64um/          # Multiply
│   ├── rv64ua/          # Atomic
│   ├── rv64uc/          # Compressed
│   └── rv64uf/          # Float
├── aurora/              # Aurora SoC tests
│   ├── g_core/          # G-Core instruction tests
│   ├── a_core/          # A-Core instruction tests
│   ├── npu/             # NPU tests
│   └── fabric/          # Interconnect + MESI tests
├── hermes/              # Hermes GPU tests
│   ├── warp/            # Single warp test
│   ├── tensor/          # Systolic array test
│   └── kernel/          # Full kernel test
├── linux-boot/          # Boot Linux via H-Core
├── opensbi/             # Boot OpenSBI
└── cosim/               # Co-simulation dengan RTL
    ├── counter           # vs test/counter.sv
    ├── picorv32          # vs PicoRV32
    ├── aurora            # vs Aurora RTL
    └── hermes            # vs Hermes RTL
```
