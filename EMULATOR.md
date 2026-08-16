# EMULATOR.md — Maria: Hardware-Software Emulator

> **Visi**: Maria bukan "QEMU yang ditulis ulang dalam Rust" — itu terlalu kecil.
> Maria adalah **Hardware-Software Emulator**: mesin virtual dibangun **langsung dari
> source HDL user** (RTL adalah machine model-nya), mampu boot dan menjalankan
> **OS nyata** (Linux/Windows) dari media yang **disediakan user** (ISO/disk/kernel).
> OS image tidak dibundel Maria.

Status: **Desain** (belum implementasi). Berlaku bersama DESIGN.md, ROADMAP.md,
SYNTHESIS.md, MARIA-HDL.md, dan AUDIT.md.

---

## 1. Ringkasan Eksekutif

| Pertanyaan | Jawaban |
|---|---|
| Mesin berasal dari mana? | **Source HDL user** (`.mv`/`.sv`/`.v`/`.svh`/`.vh`; VHDL/SystemC menyusul) |
| OS berasal dari mana? | **User** (ISO/raw image/kernel+initrd/ELF) — Maria tidak menyediakan OS |
| Engine eksekusi CPU? | **Full Rust** — Interpreter + JIT (Cranelift), tanpa dependensi QEMU |
| Struktur engine? | **Dua engine terpisah**: Maria RTL Engine + Maria Machine Engine, disatukan lewat **co-simulation** |
| Akurasi? | **Dual-mode**: `functional` (cepat) ↔ `cycle-accurate` (RTL asli), bisa dipilih per-device |
| Mode operasi CLI? | `rtl` · `sim` · `emu` · `hybrid` · `coemu` |
| Target ISA pertama? | **RISC-V** (sumber SoC di repo: `cva6/`, `openc910/`, `opentitan/`) |
| Pembeda utama vs emulator lain? | **Direct RTL Device** + **debugger lintas-lapisan** (OS → bus → RTL → signal → baris source) |
| ISO Windows? | Mungkin — bertahap via mesin x86-64 (UEFI/ACPI/APIC), setelah RISC-V/ARM terbukti |

Prinsip inti: **"Berikan RTL-nya. Maria membangun mesin virtual dari hardware tersebut."**

```
                 ┌──────────────────────────────────────┐
                 │              MARIA                    │
                 │ Hardware + Software Emulator          │
                 └──────────────────────────────────────┘
                                │
             ┌──────────────────┼──────────────────┐
             │                  │                  │
             ▼                  ▼                  ▼
       HDL Frontend        Hardware Model       OS Runtime
       .mv/.sv/.v          RTL / Netlist         Linux
       .svh/.vh            Device Model          Windows
             │                  │                  │
             └──────────────┬───┴──────────────────┘
                            ▼
                    Maria Machine Model
                            │
              ┌─────────────┼─────────────┐
              ▼             ▼             ▼
            CPU           Memory          Bus
              │             │             │
              └─────────────┼─────────────┘
                            ▼
                     Device Fabric
                            │
             ┌──────────────┼──────────────┐
             ▼              ▼              ▼
           PCIe           UART          Network
             │              │              │
             └──────────────┼──────────────┘
                            ▼
                       Guest OS
                   Linux / Windows
```

---

## 2. Perbedaan Fundamental dengan QEMU

**QEMU** mulai dari model hardware yang sudah ditulis sebagai software (device model
dalam C). **Maria** mulai dari RTL:

```
RTL (.sv/.v/.svh/.vh/.mv)
      │
      ▼
Maria HDL Compiler
      │
      ▼
Elaborated Hardware
      │
      ▼
Hardware IR (MHIR)
      │
      ▼
Maria Emulator
      │
      ▼
Real OS
```

Jika user punya `cpu.sv`, `cache.sv`, `axi.sv`, `uart.sv`, `plic.sv`, `clint.sv`,
`memory.sv`, `soc.sv` — Maria **tidak** berkata "saya punya model CPU virtual
bernama X". Maria berkata: **"Berikan RTL-nya. Saya bangun mesin virtual
berdasarkan hardware tersebut."**

Konsekuensi: bug di RTL muncul di Maria; di QEMU tidak akan pernah.

---

## 3. Prinsip Desain

1. **OS-agnostic**: Maria tidak membundel OS. Media boot (ISO/raw/kernel) dari user.
   Syarat hanya: ISA & machine cocok dengan media.
2. **HDL-native**: semua device boleh berasal dari RTL user. Device native
   (16550, PLIC, virtio) adalah fallback/percepatan, bukan keharusan.
3. **Full-Rust**: seluruh engine (termasuk JIT CPU) di Rust/Cranelift. Tidak ada
   dependensi eksekusi ke QEMU.
4. **Dua engine, jangan satu monster**: Maria RTL Engine (akurasi) dan Maria
   Machine Engine (kecepatan) adalah entitas terpisah, disatukan oleh
   co-simulation. Satu engine yang mencoba melakukan semuanya = resep monster
   compiler yang makan RAM.
5. **Dual-mode akurasi per-device**: `execution_mode = RTL | JIT | native`
   bisa berbeda untuk CPU, peripheral, dan glue logic dalam satu run.
6. **Sandbox**: OS tamu tidak pernah menyentuh host secara langsung — semua
   resource host lewat lapisan virtual + sandbox.
7. **Deterministik**: seed tetap → eksekusi identik; replay trace untuk bug.
8. **Bertahap dari aset yang ada**: reuse cycle-fusion, DAG-parallel, JIT,
   maria-sir/netlist, parallel/distributed framework.

---

## 4. MHIR — Maria Hardware IR (Jantung Maria)

MHIR adalah bagian terpenting. Bukan sekadar netlist — MHIR adalah representasi
**hardware yang sudah diekstraksi** namun **tetap menunjuk balik ke RTL source**.

```
RTL → Parser → Elaboration → MHIR
                               ├── Module
                               ├── Port
                               ├── Signal
                               ├── Register
                               ├── Memory
                               ├── Process
                               ├── Clock
                               ├── Reset
                               ├── Bus
                               ├── Interrupt
                               ├── Address Map
                               └── Device
```

### 4.1 Contoh ekstraksi

```systemverilog
module uart (
    input  logic clk,
    input  logic rst,
    input  logic [7:0] data,
    output logic tx
);
```

menjadi:

```
Device: UART
  Clock:   clk
  Reset:   rst
  Register: DATA
  Output:   TX
  Width:    8-bit
```

### 4.2 Back-pointer ke source (jangan buang RTL semantics)

Setiap node MHIR membawa asal source:

```
MHIR node
   ├── source file
   ├── line
   ├── column
   ├── module
   └── process
```

Inilah yang memungkinkan debugger lintas-lapisan (lihat §16):

```
Guest OS → CPU instruction → MMIO → UART → RTL signal → uart.sv:143
```

### 4.3 Relasi dengan IR yang ada

| Lapisan | Struktur | Status |
|---|---|---|
| AST | `maria-ast` (Design) | ✅ ada |
| IR elaborasi | `maria-ir` (IrDesign) | ✅ ada |
| Sintesis/netlist | `maria-sir`, `maria-netlist` | ✅ ada |
| **MHIR** | `maria-emu::mhir` (baru) — ekstraksi register/device/address-map **+** back-pointer | 🆕 dibangun di atas IrDesign + netlist |

MHIR tidak menggantikan IrDesign — ia **meninggikan** abstraksinya: IrDesign
tetap dipakai RTL Engine; MHIR dipakai Machine Engine dan debugger.

---

## 5. Maria Machine Definition

Dari MHIR, Maria membangun **Machine Definition**:

```
Machine
├── CPU        → ISA, registers, privilege, interrupt
├── Memory     → RAM, ROM, MMIO
├── Bus        → AXI, APB, custom
├── Interrupt Controller
├── Timer
├── UART
├── DMA
├── PCIe
├── Network
├── Storage
└── Firmware
```

OS tamu melihatnya sebagai **komputer sungguhan**.

```rust
struct MachineDef {
    name: Symbol,
    cpu: CpuDesc,                // ISA, mode, reset vector
    memory_map: Vec<MemoryRegion>,
    bus: BusTopology,
    devices: Vec<DeviceInstance>, // dari RTL / software / native
    interrupts: InterruptTopology,
    firmware: Option<FirmwareDesc>,
    boot: BootDesc,
}
```

---

## 6. Dua Jalur Eksekusi — Dua Engine Terpisah

### Jalur A — RTL-accurate (Maria RTL Engine)

Untuk: debugging RTL, verification, waveform, assertion, timing, X/Z propagation,
signal tracing, cycle accuracy.

```
RTL → Elaboration → Hardware IR → RTL Simulator → Cycle / Event Simulation
CPU → ALU → Register File → Cache → AXI → DRAM   (cycle-by-cycle)
```

= engine event-driven/cycle-based yang sudah ada (`SimulationEngine`, Tier A/B).

### Jalur B — OS Emulation (Maria Machine Engine)

Untuk: Linux, Windows, bootloader, kernel, driver, filesystem, networking, aplikasi.

```
RTL → Elaboration → Hardware Extraction → Executable Hardware Model
      → Maria Machine → Guest OS
```

Maria **tidak** menjalankan setiap gate RTL untuk setiap instruksi CPU. RTL
`always_ff @(posedge clk)` diekstraksi menjadi model eksekusi yang jauh lebih cepat.

### 6.1 Co-Simulation

```
                 Maria
                   │
        ┌──────────┴──────────┐
        ▼                     ▼
   RTL Engine            Machine Engine
   cycle/event              JIT
        │                     │
        └──────────┬──────────┘
                   ▼
              Co-Simulation
```

Satu device bisa berkata `execution_mode = RTL`, CPU `execution_mode = JIT`,
peripheral lain `execution_mode = native` — dalam satu run yang sama.

---

## 7. Engine Detail

### 7.1 Maria RTL Engine

| Tier | Deskripsi | Status |
|---|---|---|
| **Tier A** | Event-driven, IEEE 1800 13-region scheduler | ✅ ada |
| **Tier B** | Cycle-based compiled 2-state per clock domain (reuse cycle-fusion + DAG + JIT eval) | 🆕 |

### 7.2 Maria Machine Engine — CPU 4 mode

```
Maria CPU Engine
│
├── Interpreter   — paling lambat, paling mudah di-debug
│                    instruction → decode → execute → memory → interrupt
│                    untuk: debugging CPU, bring-up, verification
├── JIT           — untuk Linux/Windows
│                    guest instructions → Maria Decoder → IR → Native Code
│                    (x86-64/ARM host; Cranelift)
├── RTL-linked    — CPU itu sendiri berasal dari RTL (cycle-accurate, Tier A)
└── Hybrid        — JIT untuk komputasi, RTL-linked saat masuk hardware tertentu
```

**Hybrid adalah senjata utama**:

```
              Maria
                 │
       ┌─────────┴─────────┐
       │                   │
   Fast Path           Accurate Path
       │                   │
      JIT                 RTL
       │                   │
       ▼                   ▼
    CPU code          Peripheral
       │                   │
       └─────────┬─────────┘
                 ▼
             OS state
```

OS menjalankan kernel + aplikasi (JIT). Saat masuk MMIO → UART RTL, Maria
**berpindah ke model RTL** untuk device tersebut, lalu kembali ke JIT.

#### RTL-linked — implementasi saat ini (mode 3, `maria emu --rtl-cpu`)

Mesin (CPU) dibangun **murni dari RTL user (.sv/.v)** — bukan model software
Rust ala QEMU. Register file, ALU, dan kontrol dieksekusi oleh Maria RTL
Engine (Tier A); sisi Rust hanya menyediakan memori + orkestrasi bus.

- `RtlLinkedCpu` (`crates/maria-emu/src/cpu/rtl.rs`) — implementasi `CpuCore`
  yang mengkompilasi file RTL CPU (parser + elaborator), meresolusi port bus,
  dan menggerakkan clock. Kontrak bus wajib (picorv32-style):
  `clk, resetn, mem_valid, mem_instr, mem_addr[31:0], mem_wdata[31:0],
  mem_wstrb[3:0], mem_ready, mem_rdata[31:0], trap`.
- `Machine` (`crates/maria-emu/src/machine.rs`) — loop eksekusi: step CPU RTL
  + layani transaksi bus (`mem_valid`/`mem_ready`, strobe per-byte untuk
  store) sampai trap (ebreak/ecall/ilegal) atau `max_steps`.
- Driver clock: clk ditulis **di dalam time step** via event terjadwal
  (`SdfDelayedWrite`) — tulis langsung sebelum `step_cycle()` tidak pernah
  terdeteksi sebagai edge (engine mengambil snapshot Preponed di awal time
  step, transisi 0→1 sudah "settled" → posedge tidak membangunkan proses
  Sequential).
- Konstruksi CPU RTL: `RtlLinkedCpu::from_files(&[wrapper, core], top)`;
  reset (`resetn` rendah beberapa cycle) → boot di `PROGADDR_RESET`.
- Parameter override instance (`#(.PROGADDR_RESET(32'h8000_0000))`) di-
  elaborasi dan di-fold ke IR (`reg_pc <= 0x80000000`).

Contoh nyata (picorv32.v dari GitHub, `examples/rtl/`):

```shell
maria emu examples/rtl/rv32_bus_wrapper.sv examples/rtl/picorv32.v \
  --config emu_ram.meu \
  --rtl-cpu examples/rtl/rv32_bus_wrapper.sv --rtl-cpu examples/rtl/picorv32.v \
  --rtl-cpu-top rv32_bus_wrapper --run --max-steps 200
# → halted (trap cause=11) after N instr / M cycles — pc=0x...
```

`emu_ram.meu` (region RAM host, field di root TOML):

```toml
ram = { base = 0x80000000, size = 0x10000 }
```

E2E terverifikasi: program bare-metal (ADDI/LUI/SW/LW/ADD/SW/ebreak) di-
jalankan oleh picorv32 RTL — hasil komputasi (42+42=84) tersimpan di RAM oleh
RTL, bukan host (`test_rtl_cpu_runs_elf_program`).

**Direct RTL Device (R4)**: top `rv32_soc.sv` menginstansiasi picorv32 +
`uart_console.sv` + decoder MMIO (`0x1000_0000`, decode DI RTL).

- **MMIO write**: store CPU ke MMIO di-latch UART RTL (`tx_byte`/`tx_done`);
  host TIDAK menjawab txn MMIO (`serve()` mengecek `mmio_sel`) — decoder RTL
  yang memberi ack (OR internal `cpu_mem_ready`). Byte UART ditangkap host
  (`uart_tx_done` pulse) → `MachineResult.console` → summary CLI
  `— console: [ABC] (3 bytes)`. Rust hanya membaca sinyal output — logika
  UART murni RTL.
- **MMIO read**: `cpu_mem_rdata` = mux RTL — MMIO → register UART
  (`UART_BASE+0` = tx_byte, `UART_BASE+4` = `tx_count`), non-MMIO → rdata
  host (RAM). Host tidak men-drive `mem_rdata` saat MMIO. Status register
  monotonik (`tx_count` naik tiap tulis) → verifikasi deterministik.

Verifikasi e2e: `test_rtl_cpu_mmio_uart_console` (CPU tulis 'A','B','C' ke
0x10000000 → console "ABC"; store RAM biasa tetap bekerja) dan
`test_rtl_cpu_mmio_read_status` (CPU baca `tx_count` 0→1→2 → simpan ke RAM →
diverifikasi host).

### 7.3 Protokol co-sim MMIO (transaksi bus)

```
CPU (JIT)                Dispatcher              RTL Engine (Tier A/B)
    │  MMIO write 0x10000000   │                       │
    ├─────────────────────────►│  transaction mulai     │
    │                          ├───────────────────────►│
    │                          │  jalankan RTL sampai   │
    │                          │  bus transaction       │
    │                          │  selesai / respon      │
    │                          │◄───────────────────────┤
    │◄─────────────────────────┤  selesai + data        │
    │  resume translation      │                       │
```

- Mode functional: latensi transaksi konfigurable (default 0 / dari model bus).
- Mode cycle-accurate: RTL maju per cycle; transaksi menunggu handshake bus asli.

---

## 8. Mode Operasi CLI

| Mode | Tujuan | Engine aktif |
|---|---|---|
| `rtl` | Akurasi RTL murni | RTL Engine (Tier A) |
| `sim` | Event/cycle simulation | RTL Engine (Tier A/B) |
| `emu` | OS emulation (CPU JIT/interpreter) | Machine Engine |
| `hybrid` | JIT + RTL (per-device) | Keduanya (co-sim) |
| `coemu` | Hardware + OS co-emulation penuh | Keduanya (co-sim penuh) |

```shell
maria run --mode rtl design.mv
maria run --mode hybrid soc.mv --disk linux.img
maria run --mode coemu --rtl soc.sv --firmware opensbi.bin --disk rootfs.img
```

---

## 9. Device ABI

OS tidak peduli hardware berasal dari SystemVerilog — OS hanya melihat CPU,
Memory, PCI, UART, Storage, Network, Interrupt, Timer. Maka Maria mendefinisikan
**kontrak device**:

```
Device
├── identity    — nama, vendor, versi, jenis
├── MMIO        — region alamat, read/write callback
├── IRQ         — line, trigger level/edge
├── DMA         — master port ke memory
├── reset       — perilaku saat reset
├── clock       — domain, frekuensi
├── state       — akses state internal (debug)
├── snapshot    — serialize/deserialize state
└── migration   — pindah antar host/thread
```

### 9.1 Tiga sumber device

```
UART
 ├── RTL implementation   — uart.sv, via Direct RTL Device
 ├── Maria software model — implementasi Rust native (fallback)
 └── host terminal        — terhubung ke console host (pty/stdio/socket)
```

```rust
pub trait Device: Send {
    fn identity(&self) -> &DeviceIdentity;
    fn mmio(&mut self, addr: u64, write: bool, size: u8, val: u64) -> MmioResult;
    fn irq(&self) -> &[IrqLine];
    fn reset(&mut self);
    fn clock_domain(&self) -> ClockDomainId;
    fn snapshot(&self) -> Vec<u8>;
    fn restore(&mut self, data: &[u8]);
}
```

---

## 10. Direct RTL Device — Pembeda Utama Maria

Fitur yang menjadi pembeda utama. User punya `uart.sv`, Maria mendeteksi
`module uart`, user mendefinisikan:

```
MMIO: 0x10000000
IRQ:  5
```

Maria membangun:

```
Guest CPU
   │  store 0x10000000
   ▼
AXI
   │
   ▼
UART RTL
   │
   ▼
TX
   │
   ▼
Host Terminal
```

Bukan sekadar "UART emulator" — tetapi:

```
OS → virtual bus → actual RTL-derived device
```

Implementasi: anotasi RTL `(* maria_region = "mmio", base = "0x10000000",
size = "0x1000" *)` + `(* maria_irq = "5" *)`, atau bagian `[emu]` di project
file (lihat §18).

---

## 11. Memory Subsystem

### 11.1 Guest memory berlapis

```
Guest Memory
├── RAM
├── ROM
├── MMIO
├── Shared Memory
└── DMA Memory
```

### 11.2 Backend

```
Anonymous memory
mmap
Huge pages
File-backed memory
Shared memory
```

### 11.3 Alur akses

```
Guest Physical Address → Maria MMU → Memory Map → RAM / Device
```

- RAM → host mmap (zero-copy, byte-addressable, ukuran konfigurable).
- MMU walk (page table) dilakukan **fungsional di software** (softmmu/TLB),
  bukan di RTL — page-table walk di RTL = pembunuh performa #1.
- Koherensi DMA: master DMA di RTL menulis ke **backing store yang sama**
  (mmap) → koherensi by construction; notify untuk invalidasi TLB CPU.

---

## 12. OS Services & Boot Flow

### 12.1 Linux (target pertama)

```
Maria
 ├── RISC-V CPU RTL (cva6/C910) — atau picorv32/Ibex untuk bare-metal
 ├── AXI
 ├── CLINT
 ├── PLIC
 ├── UART
 ├── RAM
 └── VirtIO
       │
       ▼
    OpenSBI
       │
       ▼
     Linux
       │
       ▼
   userspace
```

Setelah boot: `uname -a`, `ls`, `ip addr`, `cat /proc/cpuinfo`, `mount`,
`dmesg` — semua harus bekerja jika device model + kernel support benar.

### 12.2 Windows (bertahap, jauh lebih sulit)

Bukan karena Windows sakral, tetapi kebutuhan device/firmware jauh lebih kompleks:

```
UEFI → ACPI → CPU → PCIe → APIC → HPET/Timer → RAM → Storage → Network
     → Display → USB → Interrupt
```

Phase bertahap (jangan langsung "jalankan seluruh Windows"):

| Phase | Target |
|---|---|
| Phase 1 | Windows bootloader |
| Phase 2 | Windows kernel |
| Phase 3 | Basic device initialization |
| Phase 4 | Safe Mode |
| Phase 5 | Normal desktop |

---

## 13. Keamanan & Sandbox

**OS nyata tidak boleh langsung menyentuh host.** Ini prinsip keamanan wajib.

```
Jangan:  Windows guest → Host filesystem langsung
```

```
Guest
 ↓
Virtual Device
 ↓
Maria Sandbox
 ↓
Host Resource
```

Contoh filesystem:

```
Guest NTFS → Virtual Disk → Maria Storage Backend → qcow-like / raw / sparse image
```

Host tetap terlindungi: guest hanya melihat device virtual; akses host
(terminal, file, network) selalu lewat lapisan sandbox + izin eksplisit user.

---

## 14. Snapshot Engine

```
Machine
├── CPU state
├── RAM
├── Device state
├── Interrupt state
├── DMA state
├── Timer
└── RTL state
```

```shell
maria snapshot create
maria snapshot restore
```

Snapshot juga alat debugging RTL:

```
Boot Linux → Crash → Restore snapshot → Change RTL → Replay
```

Sangat berguna untuk hardware development: ubah RTL, replay dari titik crash
tanpa boot ulang.

---

## 15. Deterministic Execution & Replay

```
Deterministic Mode
  Seed = 12345
  CPU → Device → Interrupt → DMA → Timer
  → eksekusi IDENTIK setiap run
```

```shell
maria replay trace.bin
```

Trace berisi: instruksi, MMIO, interrupt, DMA, timer events. Replay mengulang
bug secara deterministik — bernilai tinggi untuk hardware verification.

---

## 16. Time Engine

Maria tidak boleh bergantung hanya pada wall-clock host. Gunakan
**Maria Virtual Time**:

```
0, 1 ns, 2 ns, 3 ns, ...
```

CPU execution bisa memakai **instruction count**; semuanya disinkronkan:

```
Virtual Time
   ├── CPU
   ├── Timer
   ├── DMA
   ├── UART
   └── RTL
```

- Mode functional: CPU maju per instruksi, RTL maju saat transaksi MMIO.
- Mode cycle-accurate: semua maju per cycle clock domain (virtual time = time
  sim RTL).

---

## 17. Cross-Layer Debugger — Identitas Maria

Fitur paling "Maria": hubungan debugging end-to-end:

```
Linux application → syscall → driver → MMIO → PCI/AXI → RTL module
→ always_ff → signal
```

Debugger Maria menunjukkan:

```
Guest:    PID 421
Instruction: 0x80203410
Memory:      0x10000000
Device:      UART
Bus:         AXI4
RTL:         uart.sv:143
Signal:      tx_valid = 1
```

Developer hardware melihat **software → bus → RTL → signal** dalam satu
debugger. Ini didukung oleh back-pointer MHIR (§4.2) + breakpoint di semua
lapisan (instruksi CPU, MMIO, bus transaction, RTL signal).

---

## 18. Arsitektur Software Maria

```
maria/
│
├── compiler/      lexer, parser, elaborator, resolver, optimizer
├── hdl/           sv, verilog, systemverilog, maria_hdl
├── ir/            hwir (MHIR), rtl-ir, machine-ir
├── emulator/      cpu, memory, bus, interrupt, timer, scheduler
├── devices/       uart, virtio, pci, storage, network, usb, display
├── rtl_runtime/   process, signal, clock, reset, event
├── os/            linux, windows, firmware
├── jit/           decoder, optimizer, backend
├── debug/         debugger, waveform, trace, replay
└── snapshot/
```

### 18.1 Pemetaan ke workspace crates yang ada

| Konsep desain | Crate/area yang ada | Status |
|---|---|---|
| compiler + hdl | `maria-parser`, `maria-compiler`, `maria-elaboration` | ✅ |
| ir → hwir (MHIR) | `maria-ir` + **baru** `maria-emu::mhir` | 🆕 |
| ir → rtl-ir | `maria-sir`, `maria-netlist` | ✅ |
| emulator (rtl) | `maria-simulator` (engine, scheduler) | ✅ |
| emulator (machine) | **baru** `maria-emu` (cpu, memory, bus, devices) | 🆕 |
| jit | Cranelift (`jit` feature) | ✅ (perlu decoder ISA) |
| rtl_runtime | `maria-simulator` (state, value, event) | ✅ |
| debug | `maria-simulator::debugger` | ✅ (perlu lapisan lintas) |
| snapshot | checkpoint (`SIM-17/18`) + **baru** machine snapshot | 🆕 |

Aturan 1 file = 1 tanggung jawab tetap berlaku; `maria-emu` adalah crate baru
yang memakai API `maria-api`/`maria-simulator`/`maria-sir`.

---

## 19. Antarmuka Antar-Lapisan (API ringkas)

```rust
// crate baru: crates/maria-emu/
pub mod mhir;      // Maria Hardware IR: ekstraksi + back-pointer
pub mod machine;   // MachineDef, builder dari MHIR
pub mod mem;       // MemoryPort, RamRegion, MmioBackend, softmmu/TLB
pub mod cpu;       // CpuCore trait, Interpreter, JIT (Cranelift), RtlLinked
pub mod devices;   // Device trait + 16550, PLIC, CLINT, virtio-mmio, virtio-blk/net
pub mod cosim;     // co-simulation dispatcher (MMIO trap, DMA notify)
pub mod time;      // Maria Virtual Time
pub mod sandbox;   // sandbox resource access
pub mod snapshot;  // machine snapshot create/restore
pub mod replay;    // trace + deterministic replay
pub mod debug;     // cross-layer debugger

pub trait CpuCore {
    fn reset(&mut self);
    fn step(&mut self, mem: &mut dyn MemoryPort) -> Result<CpuStep, CpuFault>;
    fn pc(&self) -> u64;
    fn set_pc(&mut self, addr: u64);
    fn raise_interrupt(&mut self, irq: u32, level: bool);
    fn read_reg(&self, idx: usize) -> u64;
    fn isa(&self) -> Isa;
}

pub trait MemoryPort {
    fn read(&self, addr: u64, size: u8) -> Result<u64, AccessFault>;
    fn write(&mut self, addr: u64, size: u8, val: u64) -> Result<(), AccessFault>;
    fn region_of(&self, addr: u64) -> Option<RegionRef>; // RAM | ROM | Mmio | Unmapped
}

pub enum ExecutionMode { Rtl, Jit, Native, Hybrid }   // per-device

pub struct Emulator {
    machine: MachineDef,
    mem: Arc<dyn MemoryPort>,
    cpu: Box<dyn CpuCore>,
    rtl: RtlBackend,
    devices: DeviceTable,       // masing-masing dgn execution_mode
    time: VirtualTime,
    mode: RunMode,              // rtl | sim | emu | hybrid | coemu
}
```

---

## 20. CLI & Project File

```shell
maria run --mode rtl design.mv
maria run --mode sim soc.sv -T 1_000_000
maria run --mode emu --soc cva6 --kernel vmlinux --initrd rootfs.cpio
maria run --mode hybrid soc.mv --disk linux.img
maria run --mode coemu --rtl soc.sv --firmware opensbi.bin --disk rootfs.img

maria emu --dump-memory-map chip.maria
maria emu --dump-dtb chip.maria
maria snapshot create --tag booted
maria snapshot restore --tag booted
maria replay trace.bin
```

Konfigurasi emulator = **file TOML terpisah** (default ekstensi `.meu`),
dimuat via `--config` — **BUKAN** section di project file `.maria`
(ekstensi/direktori `.maria` dipakai MICD dan file list — tidak boleh bentrok):

```toml
# soc.meu
top = "ariane_soc"
mode = "coemu"                 # rtl | sim | emu | hybrid | coemu
accuracy = "functional"        # functional | cycle-accurate
cpu = "riscv64"                # auto-detect bila kosong
console = "pty"                # stdio | pty | socket:<path>
block = ["rootfs.img"]
iso  = "debian-riscv64.iso"    # opsional, sebagai CD-ROM
firmware = "opensbi.bin"       # opsional; default boot flow bila kosong
dtb = "board.dts"              # opsional; auto-generate bila kosong
seed = 12345                   # deterministic mode

[ram]
base = 0x80000000             # TOML integer hex didukung
size = 0x40000000             # 1GB

[[devices]]                    # Direct RTL Device
name = "u_uart"
rtl = "uart.sv"
mmio = 0x10000000
size = 0x1000
irq = 5
```

```shell
maria emu --config soc.meu rtl/ ...

# Direct RTL CPU (mode 3) — mesin dari RTL .sv/.v, bukan interpreter:
maria emu wrapper.sv picorv32.v --config emu_ram.meu \
  --rtl-cpu wrapper.sv --rtl-cpu picorv32.v --rtl-cpu-top rv32_bus_wrapper \
  --run --max-steps 10000
```

---

## 20.5 Status Implementasi (2026-08-16)

**R0 — SEBAGIAN SELESAI ✅** (crate `maria-emu`, CLI `maria emu`):

| Item R0 | Status |
|---|---|
| `maria-emu` crate (mhir: types/backptr/extract + dump) | ✅ 18 unit test |
| Ekstraksi clock/reset/register (FF inference)/memory/device | ✅ |
| Back-pointer instance (line/col) + signal (scan source) | ✅ |
| `apply_address_map` (`--addr NAME=BASE:SIZE`, match instance/module) | ✅ |
| CLI `maria emu --dump-mhir / --dump-memory-map` | ✅ |
| Config emulator file TOML terpisah (`.meu`, `--config`) — top/ram/devices/seed | ✅ 7 unit test |
| Memory subsystem: `MemoryPort` + `RamRegion` (mmap) + `MemoryMap` decode | ✅ 8 unit test |
| ELF loader (ELF32/64 LE, PT_LOAD + bss) ke MemoryPort | ✅ 6 unit test |
| CLI `--load-elf` + `--dump-memory` (hex dump) + `--config` | ✅ |
| Anotasi `(* maria_region *)` / `(* maria_irq *)` dari AST | ⏳ R0.5 |
| CPU interpreter RISC-V32 (R2) — `cpu/riscv32.rs`: RV32IM + Zicsr, trap/
  interrupt berprioritas/mret, 13+ test end-to-end | ✅ |
| **Direct RTL CPU (mode 3, §7.2)** — `cpu/rtl.rs` `RtlLinkedCpu` + `machine.rs`
  `Machine`: mesin dijalankan dari RTL .sv/.v (picorv32.v dari GitHub,
  `examples/rtl/`), bukan interpreter. CLI `--rtl-cpu/--rtl-cpu-top/--run/
  --max-steps`; e2e program bare-metal → hasil di RAM oleh RTL | ✅ |
| **Direct RTL Device (R4, sebagian besar)** — `rv32_soc.sv` +
  `uart_console.sv` (examples/rtl/): decode MMIO (0x10000000) DI RTL, UART
  TX RTL → byte ditangkap host (`mmio_sel`/`uart_tx_done`/`uart_tx_byte`),
  console host di `MachineResult.console`/`--run` summary. **MMIO write**:
  CPU tulis 'ABC' → console "ABC" (test `test_rtl_cpu_mmio_uart_console`).
  **MMIO read**: status register `tx_count` (UART_BASE+4, mux `cpu_mem_rdata`
  DI RTL) — CPU baca 0→1→2 disimpan ke RAM, diverifikasi (test
  `test_rtl_cpu_mmio_read_status`). Rust hanya membaca sinyal output UART —
  logika UART murni RTL | ✅ store+read |
| Interrupt device RTL (IRQ → CPU) + co-sim bus cycle-accurate | ⏳ |

**Bug fix maria utama (global)**:
1. `flatten_instances` mengonsumsi `top.sub_instances` tanpa mengembalikan →
   `IrDesign.top.sub_instances` selalu kosong → hierarchy tree (melab `--tree`,
   debugger, GUI outline) kosong dan distributed partitioner selalu
   single-partition. Fix: clone daftar instance sebelum flatten, kembalikan
   setelah selesai (flatten.rs).
2. **Const-fold `case` dengan label sinyal** (elaborator stmt.rs): `case
   (1'b1)` / `case (KONST)` dengan label SINYAL (idiom `(* parallel_case *)
   case (1'b1) sel: ...`) di-const-fold saat elaborasi → label sinyal gagal
   const_eval → jatuh ke `default` secara statis → cabang sinyal TIDAK pernah
   dieksekusi (picorv32 `decoded_imm` selalu X → CPU tidak mengeksekusi
   instruksi apa pun). Fix: fold hanya bila case expr KONSTAN dan SEMUA label
   KONSTAN; selain itu case dievaluasi runtime.
3. **`Expr::Paren` hilang dari `collect_sensitivity`** (util/signal_analysis.rs):
   ekspresi `(sig)` membuat sensitivity `assign`/`always_comb` kosong → proses
   tidak re-trigger saat sinyal dalam kurung berubah (bus picorv32 `mem_addr`
   tidak pernah update). Fix: `Expr::Paren(inner)` → sensitif ke inner; sama
   untuk `resolve_expr_signal` (lvalue).
4. Engine step/debugger: `run()` memanggil `initialize_time_zero()` setiap
   re-run (setelah kompaksi event, `push_event(0)` underflow) → guard hanya di
   time 0; `StepMode::StepCycle` break SEBELUM `state.time += 1` → step tidak
   pernah maju waktu → break setelah increment (core.rs).
5. Interpreter `Rv32Cpu`: `csrrw` (op=1) menulis `zimm` (field rs1 sebagai
   angka) bukan nilai register `regs[rs1]` → mtvec salah → trap melompat ke
   alamat salah (riscv32.rs).

Verifikasi: `cargo test --workspace` **2007 test pass, 0 fail** (termasuk
59 test `maria-emu`: e2e picorv32 RTL + e2e MMIO UART console store+read).

---

## 21. Roadmap — Urutan Implementasi

Urutan paling masuk akal (dari desain user):

```
MHIR → Memory/Bus → CPU interpreter → device model → Linux boot
     → JIT → RTL device bridge → deterministic replay
     → Windows/UEFI → full hybrid co-emulation
```

| Fase | Isi | Milestone / bukti sukses |
|---|---|---|
| **R0** | **MHIR** (`maria-emu::mhir`): ekstraksi register/device/address-map dari IrDesign+netlist + back-pointer source; anotasi `(* maria_region *)`; `[emu]` parse | `--dump-memory-map` benar; bare-metal ELF jalan di cva6 lewat Tier A |
| **R1** | **Memory/Bus**: `mem::RamRegion` (mmap), `MemoryPort`, decode bus; ELF loader | Bare-metal ELF jalan; akses RAM zero-copy |
| **R2** | **CPU interpreter** (RISC-V32/64, benar dulu) + `CpuCore` trait; CLINT/PLIC + UART native; boot flow OpenSBI; DTB builder; virtio-mmio + virtio-blk | ✅ RV32IM+Zicsr interpreter (riscv32.rs); ✅ RTL-linked CPU (mode 3, picorv32 boot bare-metal); ⏳ RV64 + vmlinux/initrd boot di cva6 |
| **R3** | **JIT** (Tier C): basic-block translation via Cranelift + softmmu/TLB + MMIO trap + interrupt delivery | Boot Linux < 30 s; `$` shell; `uname -a`; ISO Linux RISC-V boot |
| **R4** | **RTL device bridge** (Direct RTL Device end-to-end): UART RTL via bus co-sim; Tier B (cycle-based compiled) untuk peripheral RTL | ✅ store+read `0x10000000` → `uart_console.sv` RTL → host console + status register (`rv32_soc`, 2 test e2e); ⏳ interrupt device + mode `hybrid` |
| **R5** | **Deterministic replay** + snapshot penuh (machine state); Virtual Time lengkap | `maria replay trace.bin` reproduksi bug; snapshot boot < 1 s |
| **R6** | **Windows/UEFI**: mesin x86-64 (translation ISA ketiga) + chipset (PIC/APIC/ACPI minimal) → phase bootloader → kernel → device init → Safe Mode | Windows bootloader + kernel (functional); desktop = R6 lanjutan |
| **R7** | **Full hybrid co-emulation**: multi-core SMP, per-region accuracy, distribusi lintas host, VHDL/SystemC frontend | SMP Linux boot; mode `coemu` penuh; SoC VHDL boot |

**Catatan scope**: R0–R4 adalah jalur kritis (Linux RISC-V + Direct RTL Device —
identitas Maria). R5–R7 pararel opsional. Windows (**R6**) = investasi terbesar,
baru realistis setelah JIT terbukti di RISC-V dan ARM64.

---

## 22. Risiko & Mitigasi

| Risiko | Dampak | Mitigasi |
|---|---|---|
| MHIR ekstraksi tidak lengkap (register/device tak terdeteksi) | Machine salah | Anotasi eksplisit + deteksi struktural; fallback ke native device |
| Performa JIT < target | Boot lambat | Codegen flat, 2-state, domain-parallel; ukur di R3 |
| Translation CPU sulit (trap, privilege, atomics) | R3 molor | Interpreter benar dulu, translate per-block bertahap; test differential vs Tier A |
| Interrupt/timing OS sensitif | Hang/gagal boot | CLINT tick berbasis instruksi; verifikasi bertahap: bare-metal → initramfs → full Linux |
| SoC RTL kompleks (cva6) tak ter-elaborasi penuh | R0 tersendat | Target bergantian: picorv32 (✅) → Ibex → cva6; subset yang dibutuhkan boot |
| Sandbox bocor (guest akses host langsung) | Keamanan | Semua akses host lewat `maria-emu::sandbox` + izin eksplisit; audit |
| Non-determinisme (wall-clock, host timer) | Replay gagal | Virtual Time murni + seed; host hanya untuk I/O |
| ISO x86 = scope besar | Windows molor | Eksplisit di R6; jalur alternatif: Linux RISC-V + bare-metal menutup mayoritas verifikasi produk |

---

## 23. Benchmark Target

| Benchmark | Target |
|---|---|
| Tier B eval rate (cva6-scale) | ≥ 100 MHz cycle (single-thread) |
| JIT IPC vs interpreter | ≥ 5–10× |
| Boot Linux (vmlinux+initrd, cva6) | < 30 s (functional) |
| Boot dari ISO Linux RISC-V | < 60 s |
| Snapshot save/restore | < 1 s |
| Replay determinism | bit-identik antar run (seed sama) |

---

## 24. Kaitannya dengan Fitur Maria yang Ada

| Aset maria | Dipakai untuk |
|---|---|
| `SimulationEngine` (13-region scheduler) | RTL Engine (Tier A), backend MMIO co-sim |
| `ClockDomainAnalysis` + cycle fusion | Tier B (per-domain eval) |
| `SimulationDag` + parallel eval | Topological order + paralelisme Tier B |
| Cranelift JIT (`jit` feature) | JIT CPU + JIT eval Tier B |
| `maria-sir` / netlist (FF inference) | MHIR extraction, deteksi core, memory map |
| Distributed sim (master/slave) | R7: SMP lintas host |
| Foreign loader (VHPI/PLI/DPI, dlopen) | SystemC bridge (R7) |
| `picorv32` compile+sim ✅, `cva6/`, `openc910/` di repo | Target uji CPU / boot Linux |
| Checkpoint (`SIM-17/18`) | Dasar snapshot engine (R5) |
| Coverage, SDF, UPF | Mode cycle-accurate: regression + timing + power |

---

## 25. Keputusan yang Sudah Diambil

1. **Maria = Hardware-Software Emulator**, bukan "QEMU dalam Rust".
2. **OS tidak dibundel** — media boot (ISO/raw/kernel) sepenuhnya dari user.
3. **Dua engine terpisah** (RTL Engine + Machine Engine) + co-simulation;
   `execution_mode` per-device (`RTL | JIT | native`).
4. **MHIR adalah jantung** — ekstraksi hardware + back-pointer source.
5. **Direct RTL Device** dan **cross-layer debugger** adalah pembeda utama.
6. **Engine full-Rust** — Interpreter + JIT (Cranelift), tanpa QEMU.
7. **5 mode operasi**: `rtl` · `sim` · `emu` · `hybrid` · `coemu`.
8. **Dual-mode akurasi**: functional ↔ cycle-accurate, per-device.
9. **Sandbox**: OS tamu tidak pernah menyentuh host langsung.
10. **Deterministik**: seed + Virtual Time + `maria replay trace.bin`.
11. **Multi-ISA bertahap**: RISC-V (R0–R4) → ARM64 → x86-64 (R6, jalur Windows).
12. **Urutan implementasi**: MHIR → Memory/Bus → CPU interpreter → device model
    → Linux boot → JIT → RTL device bridge → deterministic replay → Windows/UEFI
    → full hybrid co-emulation.
