//! JIT CPU Engine — EMULATOR.md §7.2 R3.
//!
//! Translates guest RISC-V instructions to native code via Cranelift.
//! Target: RV64IMAC + Zicsr + S-mode (Linux boot).
//! Uses basic-block translation with inline caching for hot paths.

#[cfg(feature = "jit")]
use crate::cpu::{CpuCore, CpuFault, CpuStep, Isa};
#[cfg(feature = "jit")]
use crate::mem::MemoryPort;
#[cfg(feature = "jit")]
use maria_simulator::simulator::jit_cranelift::{CraneliftCompiledFn, CraneliftEngine, JitOp};
#[cfg(feature = "jit")]
use std::collections::HashMap;
#[cfg(feature = "jit")]
use std::sync::Arc;

/// JIT-compiled RISC-V CPU core.
///
/// Implements `CpuCore` trait for RISC-V64 with Cranelift backend.
/// Uses per-basic-block JIT compilation with simple caching.
#[cfg(feature = "jit")]
pub struct Rv64JitCpu {
    /// General-purpose registers x0-x31
    regs: [u64; 32],
    /// Program counter
    pc: u64,
    /// Machine-mode CSRs (minimal set for Linux boot)
    csrs: CsrFile,
    /// Pending interrupt (level-sensitive)
    irq_pending: u64,
    /// JIT engine for code generation
    jit: Option<CraneliftEngine>,
    /// Compiled basic block cache: pc -> compiled function
    bb_cache: HashMap<u64, JitBasicBlock>,
    /// Execution statistics
    stats: JitStats,
    /// Max instructions per basic block before forced exit
    max_bb_instrs: usize,
}

/// Machine-mode CSR file (minimal for Linux/OpenSBI)
#[cfg(feature = "jit")]
#[derive(Clone, Default)]
struct CsrFile {
    mstatus: u64,
    mtvec: u64,
    mepc: u64,
    mcause: u64,
    mtval: u64,
    mie: u64,
    mip: u64,
    satp: u64,
    mscratch: u64,
    medeleg: u64,
    mideleg: u64,
    sstatus: u64,
    stvec: u64,
    sepc: u64,
    scause: u64,
    stval: u64,
    sip: u64,
    sie: u64,
}

/// A JIT-compiled basic block
#[cfg(feature = "jit")]
#[derive(Clone)]
struct JitBasicBlock {
    /// Entry PC
    entry_pc: u64,
    /// Compiled function pointer (takes &mut Rv64JitCpu, returns CpuStep)
    code_ptr: *const u8,
    /// Number of instructions in this block
    instr_count: usize,
    /// Fallthrough PC (if no branch taken)
    fallthrough_pc: u64,
    /// Target PC if branch taken (for conditional branches)
    branch_target: Option<u64>,
}

/// JIT execution statistics
#[cfg(feature = "jit")]
#[derive(Default, Clone, Debug)]
pub struct JitStats {
    pub total_instructions: u64,
    pub jit_compiled_blocks: usize,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub interpreter_fallbacks: u64,
}

#[cfg(feature = "jit")]
impl Rv64JitCpu {
    /// Create a new RV64 JIT CPU
    pub fn new() -> Self {
        let jit = CraneliftEngine::new();
        Rv64JitCpu {
            regs: [0; 32],
            pc: 0,
            csrs: CsrFile::default(),
            irq_pending: 0,
            jit,
            bb_cache: HashMap::new(),
            stats: JitStats::default(),
            max_bb_instrs: 32,
        }
    }

    /// Create with existing JIT engine (for sharing cache)
    pub fn with_jit(jit: CraneliftEngine) -> Self {
        let mut cpu = Self::new();
        cpu.jit = Some(jit);
        cpu
    }

    /// Reset CPU state
    pub fn reset(&mut self) {
        self.regs = [0; 32];
        self.pc = 0;
        self.csrs = CsrFile::default();
        self.irq_pending = 0;
        self.bb_cache.clear();
        self.stats = JitStats::default();
    }

    /// Get JIT statistics
    pub fn stats(&self) -> &JitStats {
        &self.stats
    }

    /// Set maximum instructions per basic block
    pub fn set_max_bb_instrs(&mut self, max: usize) {
        self.max_bb_instrs = max;
    }

    /// Read a CSR
    fn read_csr(&self, addr: u16) -> u64 {
        match addr {
            0x300 => self.csrs.mstatus,  // mstatus
            0x305 => self.csrs.mtvec,    // mtvec
            0x341 => self.csrs.mepc,     // mepc
            0x342 => self.csrs.mcause,   // mcause
            0x343 => self.csrs.mtval,    // mtval
            0x304 => self.csrs.mie,      // mie
            0x344 => self.csrs.mip,      // mip
            0x180 => self.csrs.satp,     // satp
            0x340 => self.csrs.mscratch, // mscratch
            0x3aa => self.csrs.medeleg,  // medeleg
            0x3ab => self.csrs.mideleg,  // mideleg
            0x100 => self.csrs.sstatus,  // sstatus
            0x105 => self.csrs.stvec,    // stvec
            0x141 => self.csrs.sepc,     // sepc
            0x142 => self.csrs.scause,   // scause
            0x143 => self.csrs.stval,    // stval
            0x144 => self.csrs.sip,      // sip
            0x104 => self.csrs.sie,      // sie
            _ => 0,
        }
    }

    /// Write a CSR
    fn write_csr(&mut self, addr: u16, val: u64) {
        match addr {
            0x300 => self.csrs.mstatus = val,
            0x305 => self.csrs.mtvec = val,
            0x341 => self.csrs.mepc = val,
            0x342 => self.csrs.mcause = val,
            0x343 => self.csrs.mtval = val,
            0x304 => self.csrs.mie = val,
            0x344 => self.csrs.mip = val,
            0x180 => self.csrs.satp = val,
            0x340 => self.csrs.mscratch = val,
            0x3aa => self.csrs.medeleg = val,
            0x3ab => self.csrs.mideleg = val,
            0x100 => self.csrs.sstatus = val,
            0x105 => self.csrs.stvec = val,
            0x141 => self.csrs.sepc = val,
            0x142 => self.csrs.scause = val,
            0x143 => self.csrs.stval = val,
            0x144 => self.csrs.sip = val,
            0x104 => self.csrs.sie = val,
            _ => {}
        }
    }

    /// Check and handle pending interrupts
    fn check_interrupts(&mut self) -> Option<CpuStep> {
        // M-mode external interrupt (MEIP) = bit 11
        let meip = (self.csrs.mip >> 11) & 1;
        let mie_meie = (self.csrs.mie >> 11) & 1;
        let mstatus_mie = (self.csrs.mstatus >> 3) & 1;

        if meip == 1 && mie_meie == 1 && mstatus_mie == 1 {
            // Take interrupt
            self.csrs.mepc = self.pc;
            self.csrs.mcause = 0x8000_0000_0000_000Bu64; // MEI
                                                         // MPIE = MIE SEBELUM clear, lalu MIE=0 (urutan penting).
            self.csrs.mstatus |= ((self.csrs.mstatus >> 3) & 1) << 7;
            self.csrs.mstatus &= !(1 << 3);
            self.pc = self.csrs.mtvec;
            self.irq_pending &= !(1 << 11);
            self.csrs.mip &= !(1 << 11);
            return Some(CpuStep::InstructionExecuted { cycles: 1 });
        }

        // Timer interrupt (MTIP) = bit 7
        let mtip = (self.csrs.mip >> 7) & 1;
        let mie_mtie = (self.csrs.mie >> 7) & 1;
        if mtip == 1 && mie_mtie == 1 && mstatus_mie == 1 {
            self.csrs.mepc = self.pc;
            self.csrs.mcause = 0x8000_0000_0000_0007u64; // MTI
            self.csrs.mstatus |= ((self.csrs.mstatus >> 3) & 1) << 7;
            self.csrs.mstatus &= !(1 << 3);
            self.pc = self.csrs.mtvec;
            self.irq_pending &= !(1 << 7);
            self.csrs.mip &= !(1 << 7);
            return Some(CpuStep::InstructionExecuted { cycles: 1 });
        }

        // Software interrupt (MSIP) = bit 3
        let msip = (self.csrs.mip >> 3) & 1;
        let mie_msie = (self.csrs.mie >> 3) & 1;
        if msip == 1 && mie_msie == 1 && mstatus_mie == 1 {
            self.csrs.mepc = self.pc;
            self.csrs.mcause = 0x8000_0000_0000_0003u64; // MSI
            self.csrs.mstatus |= ((self.csrs.mstatus >> 3) & 1) << 7;
            self.csrs.mstatus &= !(1 << 3);
            self.pc = self.csrs.mtvec;
            self.irq_pending &= !(1 << 3);
            self.csrs.mip &= !(1 << 3);
            return Some(CpuStep::InstructionExecuted { cycles: 1 });
        }

        None
    }

    /// Fetch 32-bit instruction from memory
    fn fetch_instr(&mut self, mem: &mut dyn MemoryPort, pc: u64) -> Result<u32, CpuFault> {
        match mem.read(pc, 4) {
            Ok(val) => Ok(val as u32),
            Err(_) => Err(CpuFault {
                pc,
                reason: "instruction fetch fault".into(),
            }),
        }
    }

    /// Execute a single instruction in interpreter mode (fallback)
    fn execute_interpreter(
        &mut self,
        mem: &mut dyn MemoryPort,
        instr: u32,
    ) -> Result<CpuStep, CpuFault> {
        // Delegate to the interpreter implementation
        // For now, return a trap to indicate unimplemented
        // In practice, we'd call the interpreter's decode/execute
        Err(CpuFault {
            pc: self.pc,
            reason: format!("instruction 0x{:08x} not yet JIT-compiled", instr),
        })
    }

    /// Try to JIT-compile a basic block starting at `pc`
    fn try_compile_block(&mut self, mem: &mut dyn MemoryPort, pc: u64) -> Option<JitBasicBlock> {
        let jit = self.jit.as_mut()?;
        let mut instrs = Vec::new();
        let mut current_pc = pc;

        // Fetch instructions up to max_bb_instrs or until control flow
        for _ in 0..self.max_bb_instrs {
            let instr = match self.fetch_instr(mem, current_pc) {
                Ok(i) => i,
                Err(_) => break,
            };
            instrs.push((current_pc, instr));

            // Check if this instruction ends the basic block
            let opcode = instr & 0x7F;
            let is_branch = matches!(opcode, 0x63 | 0x6F | 0x67); // BRANCH, JAL, JALR
            let is_system = matches!(opcode, 0x73); // SYSTEM (ECALL, EBREAK, CSR)

            current_pc = current_pc.wrapping_add(4);

            if is_branch || is_system {
                break;
            }
        }

        if instrs.is_empty() {
            return None;
        }

        // For now, we don't have full JIT compilation of RV64 instructions
        // This is a stub that marks the block for future compilation
        // Real implementation would:
        // 1. Decode each instruction to Cranelift IR
        // 2. Build a function that executes the block
        // 3. Handle register allocation, memory access, CSR access
        // 4. Return compiled function pointer

        None // Not yet implemented
    }

    /// Execute using interpreter (fallback when JIT not ready)
    fn execute_interpreter_fallback(
        &mut self,
        mem: &mut dyn MemoryPort,
    ) -> Result<CpuStep, CpuFault> {
        self.stats.interpreter_fallbacks += 1;
        let instr = self.fetch_instr(mem, self.pc)?;
        self.execute_interpreter(mem, instr)
    }
}

#[cfg(feature = "jit")]
impl CpuCore for Rv64JitCpu {
    fn reset(&mut self) {
        self.reset();
    }

    fn step(&mut self, mem: &mut dyn MemoryPort) -> Result<CpuStep, CpuFault> {
        // Check for pending interrupts first
        if let Some(step) = self.check_interrupts() {
            self.stats.total_instructions += 1;
            return Ok(step);
        }

        // Try to get or compile a basic block
        let pc = self.pc;
        if let Some(bb) = self.bb_cache.get(&pc).cloned() {
            self.stats.cache_hits += 1;
            // Execute compiled block
            // Safety: we trust our own compiled code
            // In reality, we'd call the function pointer here
            // For now, fall back to interpreter
            self.execute_interpreter_fallback(mem)
        } else {
            self.stats.cache_misses += 1;
            // Try to compile
            if let Some(bb) = self.try_compile_block(mem, pc) {
                self.stats.jit_compiled_blocks += 1;
                self.bb_cache.insert(pc, bb.clone());
                // Execute compiled block
                self.execute_interpreter_fallback(mem)
            } else {
                // Fall back to interpreter
                self.execute_interpreter_fallback(mem)
            }
        }
    }

    fn pc(&self) -> u64 {
        self.pc
    }

    fn set_pc(&mut self, addr: u64) {
        self.pc = addr;
    }

    fn raise_interrupt(&mut self, irq: u32, level: bool) {
        if level {
            self.irq_pending |= 1 << irq;
            self.csrs.mip |= 1 << irq;
        } else {
            self.irq_pending &= !(1 << irq);
            self.csrs.mip &= !(1 << irq);
        }
    }

    fn read_reg(&self, idx: usize) -> u64 {
        if idx < 32 {
            self.regs[idx]
        } else {
            0
        }
    }

    fn isa(&self) -> Isa {
        Isa::RiscV64
    }
}

// ─── Stub for non-JIT builds ───

#[cfg(not(feature = "jit"))]
pub struct Rv64JitCpu;

#[cfg(not(feature = "jit"))]
impl Rv64JitCpu {
    pub fn new() -> Self {
        panic!("Rv64JitCpu requires 'jit' feature")
    }
}

#[cfg(not(feature = "jit"))]
impl crate::cpu::CpuCore for Rv64JitCpu {
    fn reset(&mut self) {}
    fn step(
        &mut self,
        _mem: &mut dyn crate::mem::MemoryPort,
    ) -> Result<crate::cpu::CpuStep, crate::cpu::CpuFault> {
        Err(crate::cpu::CpuFault {
            pc: 0,
            reason: "JIT not enabled".into(),
        })
    }
    fn pc(&self) -> u64 {
        0
    }
    fn set_pc(&mut self, _addr: u64) {}
    fn raise_interrupt(&mut self, _irq: u32, _level: bool) {}
    fn read_reg(&self, _idx: usize) -> u64 {
        0
    }
    fn isa(&self) -> crate::cpu::Isa {
        crate::cpu::Isa::RiscV64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "jit")]
    fn test_jit_cpu_create() {
        let cpu = Rv64JitCpu::new();
        assert_eq!(cpu.isa(), Isa::RiscV64);
        assert_eq!(cpu.pc(), 0);
    }

    #[test]
    #[cfg(feature = "jit")]
    fn test_jit_cpu_reset() {
        let mut cpu = Rv64JitCpu::new();
        cpu.set_pc(0x8000_0000);
        cpu.regs[10] = 42;
        cpu.reset();
        assert_eq!(cpu.pc(), 0);
        assert_eq!(cpu.regs[10], 0);
    }
}
