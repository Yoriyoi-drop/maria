//! Machine Engine — EMULATOR.md §6 Jalur B.
//!
//! `Machine` menyatukan CPU (interpreter `.rs` atau **RTL-linked dari
//! `.sv`/`.v`**) + memori guest (`MemoryMap`) + loop eksekusi. CPU di-step
//! satu instruksi per iterasi; transaksi bus di-service lewat `MemoryPort`.
//! Selesai saat trap (ebreak/ecall/ilegal) atau `max_steps` tercapai.

use crate::cpu::{CpuCore, CpuFault, CpuStep};
use crate::mem::MemoryMap;

/// Hasil eksekusi mesin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineResult {
    /// Jumlah instruksi yang dieksekusi.
    pub steps: u64,
    /// Total cycle (RTL time unit) yang dikonsumsi CPU.
    pub cycles: u64,
    /// PC terakhir (instruksi terakhir yang selesai).
    pub pc: u64,
    /// true bila berhenti karena trap (ebreak/ecall/ilegal).
    pub halted: bool,
    /// Cause trap (mode machine; ecall = 11).
    pub cause: u64,
    /// Tval trap (alamat instruksi penyebab).
    pub tval: u64,
    /// Byte output Direct RTL Device (UART console RTL) selama run.
    pub console: Vec<u8>,
}

impl MachineResult {
    /// Ringkas untuk output CLI.
    pub fn summary(&self) -> String {
        let head = if self.halted {
            format!(
                "halted (trap cause={}, tval=0x{:08x}) after {} instr / {} cycles — pc=0x{:08x}",
                self.cause, self.tval, self.steps, self.cycles, self.pc
            )
        } else {
            format!(
                "max-steps reached after {} instr / {} cycles — pc=0x{:08x}",
                self.steps, self.cycles, self.pc
            )
        };
        if !self.console.is_empty() {
            let txt: String = self
                .console
                .iter()
                .map(|&b| {
                    if (32..=126).contains(&b) {
                        b as char
                    } else {
                        '?'
                    }
                })
                .collect();
            format!("{} — console: [{}] ({} bytes)", head, txt, self.console.len())
        } else {
            head
        }
    }
}

/// Mesin eksekusi: CPU + memori guest + batas langkah.
pub struct Machine {
    pub cpu: Box<dyn CpuCore>,
    pub mem: MemoryMap,
    pub max_steps: u64,
}

impl Machine {
    pub fn new(cpu: Box<dyn CpuCore>, mem: MemoryMap, max_steps: u64) -> Self {
        Self { cpu, mem, max_steps }
    }

    /// Jalankan sampai trap / max_steps. `MmioAccess` (R4, co-sim RTL device)
    /// diabaikan untuk saat ini — RAM/ROM di-service langsung oleh MemoryMap.
    pub fn run(&mut self) -> Result<MachineResult, CpuFault> {
        let mut steps = 0u64;
        let mut cycles = 0u64;
        while steps < self.max_steps {
            match self.cpu.step(&mut self.mem)? {
                CpuStep::InstructionExecuted { cycles: c } => {
                    steps += 1;
                    cycles += c;
                }
                CpuStep::MmioAccess { .. } => {
                    // R4: dispatch ke Direct RTL Device. Belum diimplementasikan.
                }
                CpuStep::Trap { cause, tval } => {
                    return Ok(MachineResult {
                        steps: steps + 1,
                        cycles,
                        pc: self.cpu.pc(),
                        halted: true,
                        cause,
                        tval,
                        console: self.cpu.console_output().to_vec(),
                    });
                }
            }
        }
        Ok(MachineResult {
            steps,
            cycles,
            pc: self.cpu.pc(),
            halted: false,
            cause: 0,
            tval: 0,
            console: self.cpu.console_output().to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::{Isa, Rv32Cpu};
    use crate::mem::{MemoryMap, MemoryPort, RamRegion, RegionKind};
    use maria_core::intern::Symbol;

    #[test]
    fn test_machine_summary_halted() {
        let r = MachineResult { steps: 7, cycles: 42, pc: 0x8000_001c, halted: true, cause: 11, tval: 0x8000_001c, console: Vec::new() };
        let s = r.summary();
        assert!(s.contains("halted"));
        assert!(s.contains("cause=11"));
        // Console RTL device ikut di-ringkas.
        let r2 = MachineResult { steps: 3, cycles: 12, pc: 0x1000, halted: true, cause: 11, tval: 0x1000, console: b"ABC".to_vec() };
        assert!(r2.summary().contains("console: [ABC] (3 bytes)"));
    }

    #[test]
    fn test_machine_max_steps_no_halt() {
        let mut mem = MemoryMap::new();
        mem.add(RamRegion::new(Symbol::intern("ram"), 0x0, 0x1000, RegionKind::Ram, false).unwrap()).unwrap();
        // Interpreter: loop tak berujung (jump ke diri sendiri) → max_steps.
        let mut cpu = Rv32Cpu::new();
        cpu.set_pc(0x0);
        // jal 0 (infinite loop) di 0x0
        mem.write(0x0, 4, 0x0000_006f).unwrap();
        let mut machine = Machine::new(Box::new(cpu), mem, 3);
        let r = machine.run().unwrap();
        assert!(!r.halted);
        assert_eq!(r.steps, 3);
        assert_eq!(machine.cpu.isa(), Isa::RiscV32);
    }
}
