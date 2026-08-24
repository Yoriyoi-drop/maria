//! Maria CPU Engine — EMULATOR.md §7.2.
//!
//! Empat mode eksekusi CPU: Interpreter (R2) → JIT (R3) → RTL-linked →
//! Hybrid. Fase ini: **Interpreter** (benar dulu, untuk bring-up/verifikasi),
//! dengan kontrak `CpuCore` yang sama untuk semua mode — JIT/RTL-linked
//! tinggal mengimplementasi trait yang sama.
//!
//! Interface ke dunia luar hanya 3 titik (EMULATOR.md §6.3): memori
//! (`MemoryPort`), interrupt (`raise_interrupt`), dan trap (`mcause`/CSR).

use crate::mem::MemoryPort;

/// ISA yang didukung CPU engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Isa {
    RiscV32,
    RiscV64,
    AArch64,
    X86_64,
}

impl Isa {
    pub fn label(&self) -> &'static str {
        match self {
            Isa::RiscV32 => "riscv32",
            Isa::RiscV64 => "riscv64",
            Isa::AArch64 => "aarch64",
            Isa::X86_64 => "x86_64",
        }
    }
}

/// Hasil satu langkah CPU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CpuStep {
    /// Satu instruksi dieksekusi (atau trap/interrupt ditangani).
    InstructionExecuted { cycles: u64 },
    /// Akses MMIO — jalur co-simulation: dispatcher meneruskan ke RTL
    /// (Tier A/B) lalu resume. Fase interpreter R2: RAM langsung via
    /// `MemoryPort`; MMIO trap aktif di R4.
    MmioAccess { addr: u64, write: bool, size: u8 },
    /// Trap keluar ke host (untuk JIT/co-sim). Interpreter R2 menangani
    /// trap INTERNAL (jump mtvec, set mepc/mcause) — varian ini untuk mode
    /// yang menyerahkan trap ke dispatcher.
    Trap { cause: u64, tval: u64 },
}

/// Kegagalan fatal CPU (bukan trap machine — kondisi tak bisa dilanjutkan).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuFault {
    pub pc: u64,
    pub reason: String,
}

impl std::fmt::Display for CpuFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cpu fault @0x{:x}: {}", self.pc, self.reason)
    }
}

/// Kontrak CPU core — semua mode (interpreter/JIT/RTL-linked) memenuhi ini.
pub trait CpuCore {
    /// Reset penuh: register = 0, PC = 0, CSR = 0.
    fn reset(&mut self);
    /// Eksekusi satu langkah (instruksi / trap / interrupt).
    fn step(&mut self, mem: &mut dyn MemoryPort) -> Result<CpuStep, CpuFault>;
    fn pc(&self) -> u64;
    fn set_pc(&mut self, addr: u64);
    /// Naikkan/turunkan garis interrupt `irq` (level-sensitive).
    fn raise_interrupt(&mut self, irq: u32, level: bool);
    /// Baca register `idx` (x0 → 0).
    fn read_reg(&self, idx: usize) -> u64;
    fn isa(&self) -> Isa;
    /// Byte output perangkat RTL (UART console) selama run. Default kosong;
    /// RTL-linked CPU dengan Direct RTL Device mengisi ini.
    fn console_output(&self) -> &[u8] {
        &[]
    }
}

pub mod riscv32;
pub mod rtl;
pub mod x86;

pub use riscv32::Rv32Cpu;
pub use rtl::RtlLinkedCpu;
pub use x86::{FileDisk, X86Cpu, X86Disk};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_isa_label() {
        assert_eq!(Isa::RiscV32.label(), "riscv32");
        assert_eq!(Isa::X86_64.label(), "x86_64");
    }

    #[test]
    fn test_cpu_fault_display() {
        let f = CpuFault {
            pc: 0x8000_0000,
            reason: "fetch".into(),
        };
        assert!(f.to_string().contains("0x80000000"));
        assert!(f.to_string().contains("fetch"));
    }
}
