//! Interpreter RISC-V RV32IM + Zicsr — mode Interpreter (R2).
//!
//! Set instruksi: RV32I (LUI/AUIPC/JAL/JALR/branch/load/store/OP-IMM/OP/
//! FENCE/ECALL/EBREAK/MRET) + M (mul/div/rem) + Zicsr (csrrw/csrrs/csrrc +
//! varian immediate) + interrupt machine (mie/mip, MSIE/MTIE/MEIE).
//!
//! Perilaku machine (bukan ISS yang menyerahkan trap):
//! - ECALL/EBREAK/instruksi ilegal/akses fault → trap INTERNAL: set
//!   mepc/mcause/mtval, MPIE=MIE, MIE=0, MPP=3, PC = mtvec (direct mode).
//! - Interrupt: jika mstatus.MIE && (mie & mip) → mcause = 0x80000000|irq.
//! - MRET: PC = mepc, MIE = MPIE, MPIE = 1, MPP = 0.
//! - x0 selalu 0; tulis ke x0 dibuang. Load/store tidak misaligned (byte-wise
//!   MemoryPort — RAM menangani offset apa pun).

use crate::cpu::{CpuCore, CpuFault, CpuStep, Isa};
use crate::mem::MemoryPort;

// ── mstatus bits ──
const MSTATUS_MIE: u32 = 0x8;
const MSTATUS_MPIE: u32 = 0x80;
const MSTATUS_MPP: u32 = 0x1800;

// ── CSR address ──
const CSR_MSTATUS: u64 = 0x300;
const CSR_MIE: u64 = 0x304;
const CSR_MTVEC: u64 = 0x305;
const CSR_MSCRATCH: u64 = 0x340;
const CSR_MEPC: u64 = 0x341;
const CSR_MCAUSE: u64 = 0x342;
const CSR_MTVAL: u64 = 0x343;
const CSR_MIP: u64 = 0x344;

// ── Trap cause (exception) ──
const CAUSE_INST_ACCESS_FAULT: u64 = 1;
const CAUSE_ILLEGAL_INSTRUCTION: u64 = 2;
const CAUSE_BREAKPOINT: u64 = 3;
const CAUSE_LOAD_ACCESS_FAULT: u64 = 5;
const CAUSE_STORE_ACCESS_FAULT: u64 = 7;
const CAUSE_ECALL_M: u64 = 11;

// ── Interrupt pending (mip/mie) ──
const IRQ_MSI: u32 = 3;
const IRQ_MTI: u32 = 7;
const IRQ_MEI: u32 = 11;

/// CPU RISC-V 32-bit (RV32IM + Zicsr).
#[derive(Debug, Clone)]
pub struct Rv32Cpu {
    regs: [u64; 32],
    pc: u64,
    // CSR
    mstatus: u32,
    mtvec: u64,
    mepc: u64,
    mcause: u64,
    mtval: u64,
    mscratch: u64,
    mie: u32,
    mip: u32,
    /// Garis interrupt level-sensitive dari `raise_interrupt` (16 line).
    irq_level: [bool; 16],
    /// Jumlah instruksi/step dieksekusi.
    cycles: u64,
}

impl Rv32Cpu {
    pub fn new() -> Self {
        let mut cpu = Self {
            regs: [0; 32],
            pc: 0,
            mstatus: 0,
            mtvec: 0,
            mepc: 0,
            mcause: 0,
            mtval: 0,
            mscratch: 0,
            mie: 0,
            mip: 0,
            irq_level: [false; 16],
            cycles: 0,
        };
        cpu.reset();
        cpu
    }

    /// Baca CSR umum (dipakai test). CSR tak dikenal → None.
    pub fn csr(&self, addr: u64) -> Option<u64> {
        match addr {
            CSR_MSTATUS => Some(self.mstatus as u64),
            CSR_MIE => Some(self.mie as u64),
            CSR_MTVEC => Some(self.mtvec),
            CSR_MSCRATCH => Some(self.mscratch),
            CSR_MEPC => Some(self.mepc),
            CSR_MCAUSE => Some(self.mcause),
            CSR_MTVAL => Some(self.mtval),
            CSR_MIP => Some(self.mip as u64),
            _ => None,
        }
    }

    pub fn cycles(&self) -> u64 {
        self.cycles
    }

    /// Eksekusi `n` langkah; berhenti lebih awal bila trap ECALL/EBREAK
    /// diterima (cause >= 3 dan cause bukan access-fault loop). Return jumlah
    /// langkah aktual. Untuk test/bring-up.
    pub fn run_steps(&mut self, n: u64, mem: &mut dyn MemoryPort) -> Result<u64, CpuFault> {
        let mut done = 0;
        for _ in 0..n {
            let before = self.mcause;
            let pc_before = self.pc;
            self.step(mem)?;
            done += 1;
            // Berhenti bila step ini menghasilkan trap (mcause berubah) dan
            // PC tidak berubah (mtvec tak ter-set / loop trap).
            if self.mcause != before && self.pc == pc_before {
                break;
            }
        }
        Ok(done)
    }

    /// Jalankan trap internal: mepc = inst_pc, mcause, mtval, MPIE=MIE,
    /// MIE=0, MPP=3, PC = mtvec (direct mode, mask bit [1:0]).
    fn trap(&mut self, cause: u64, tval: u64, inst_pc: u64) {
        self.mcause = cause;
        self.mtval = tval;
        self.mepc = inst_pc;
        let mpie = (self.mstatus & MSTATUS_MIE) << 4; // MIE → MPIE
        self.mstatus =
            (self.mstatus & !(MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_MPP)) | mpie | (3 << 11);
        self.pc = self.mtvec & !3;
    }

    fn mret(&mut self) {
        self.pc = self.mepc;
        let mie = ((self.mstatus & MSTATUS_MPIE) >> 4) & MSTATUS_MIE;
        self.mstatus = (self.mstatus & !(MSTATUS_MIE | MSTATUS_MPP)) | mie | MSTATUS_MPIE;
    }

    /// Interrupt tertunda berprioritas tertinggi (MEI > MTI > MSI).
    fn pending_interrupt(&self) -> Option<u32> {
        let pending = self.mie & self.mip;
        for irq in [IRQ_MEI, IRQ_MTI, IRQ_MSI] {
            if pending & (1 << irq) != 0 {
                return Some(irq);
            }
        }
        None
    }

    fn csr_read(&self, addr: u64) -> Option<u64> {
        self.csr(addr)
    }

    /// Tulis CSR dengan mask writable. Return false bila CSR tak dikenal.
    fn csr_write(&mut self, addr: u64, val: u64) -> bool {
        match addr {
            CSR_MSTATUS => {
                self.mstatus = (val as u32) & (MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_MPP);
                true
            }
            CSR_MIE => {
                self.mie = (val as u32) & 0x888; // MSIE|MTIE|MEIE
                true
            }
            CSR_MTVEC => {
                self.mtvec = val & !3;
                true
            }
            CSR_MSCRATCH => {
                self.mscratch = val;
                true
            }
            CSR_MEPC => {
                self.mepc = val & !1;
                true
            }
            CSR_MCAUSE => {
                self.mcause = val;
                true
            }
            CSR_MTVAL => {
                self.mtval = val;
                true
            }
            CSR_MIP => {
                self.mip = (val as u32) & 0x888;
                true
            }
            _ => false,
        }
    }

    fn write_rd(&mut self, rd: u8, val: u64) {
        if rd != 0 {
            self.regs[rd as usize] = val;
        }
    }
}

impl Default for Rv32Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl CpuCore for Rv32Cpu {
    fn reset(&mut self) {
        self.regs = [0; 32];
        self.pc = 0;
        self.mstatus = 0;
        self.mtvec = 0;
        self.mepc = 0;
        self.mcause = 0;
        self.mtval = 0;
        self.mscratch = 0;
        self.mie = 0;
        self.mip = 0;
        self.irq_level = [false; 16];
        self.cycles = 0;
    }

    fn step(&mut self, mem: &mut dyn MemoryPort) -> Result<CpuStep, CpuFault> {
        self.cycles += 1;

        // ── Interrupt check (sebelum fetch) ──
        if self.mstatus & MSTATUS_MIE != 0 {
            if let Some(irq) = self.pending_interrupt() {
                self.trap(0x8000_0000 | irq as u64, 0, self.pc);
                return Ok(CpuStep::InstructionExecuted { cycles: 1 });
            }
        }

        let pc = self.pc;
        let raw = match mem.read(pc, 4) {
            Ok(v) => v as u32,
            Err(_) => {
                // Instruksi access fault → trap internal.
                self.trap(CAUSE_INST_ACCESS_FAULT, pc, pc);
                return Ok(CpuStep::InstructionExecuted { cycles: 1 });
            }
        };

        match decode(raw) {
            Decoded::Illegal => self.trap(CAUSE_ILLEGAL_INSTRUCTION, raw as u64, pc),
            Decoded::Ecall => self.trap(CAUSE_ECALL_M, 0, pc),
            Decoded::Ebreak => self.trap(CAUSE_BREAKPOINT, 0, pc),
            Decoded::Mret => self.mret(),
            Decoded::Instr(instr) => match self.execute(pc, instr, mem) {
                Ok(()) => {}
                Err(ExecErr::LoadFault(addr)) => {
                    self.trap(CAUSE_LOAD_ACCESS_FAULT, addr, pc)
                }
                Err(ExecErr::StoreFault(addr)) => {
                    self.trap(CAUSE_STORE_ACCESS_FAULT, addr, pc)
                }
                Err(ExecErr::Illegal(raw)) => {
                    self.trap(CAUSE_ILLEGAL_INSTRUCTION, raw as u64, pc)
                }
            },
        }
        Ok(CpuStep::InstructionExecuted { cycles: 1 })
    }

    fn pc(&self) -> u64 {
        self.pc
    }

    fn set_pc(&mut self, addr: u64) {
        self.pc = addr & !1;
    }

    fn raise_interrupt(&mut self, irq: u32, level: bool) {
        if irq >= 16 {
            return;
        }
        self.irq_level[irq as usize] = level;
        let mask = 1u32 << irq;
        if level {
            self.mip |= mask;
        } else {
            self.mip &= !mask;
        }
    }

    fn read_reg(&self, idx: usize) -> u64 {
        if idx == 0 {
            0
        } else {
            self.regs[idx]
        }
    }

    fn isa(&self) -> Isa {
        Isa::RiscV32
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Decode
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decoded {
    Instr(Instr),
    Ecall,
    Ebreak,
    Mret,
    Illegal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Instr {
    Lui { rd: u8, imm: u64 },
    Auipc { rd: u8, imm: u64 },
    Jal { rd: u8, imm: i64 },
    Jalr { rd: u8, rs1: u8, imm: i64 },
    Branch { op: u8, rs1: u8, rs2: u8, imm: i64 },
    Load { op: u8, rd: u8, rs1: u8, imm: i64 },
    Store { op: u8, rs1: u8, rs2: u8, imm: i64 },
    OpImm { op: u8, rd: u8, rs1: u8, imm: i64 },
    Op { op: u8, rd: u8, rs1: u8, rs2: u8 },
    Fence,
    Csr { op: u8, rd: u8, rs1: u8, csr: u64, zimm: u64 },
}

fn sext(v: u64, bits: u32) -> i64 {
    let shift = 64 - bits;
    ((v << shift) as i64) >> shift
}

fn decode(raw: u32) -> Decoded {
    let opcode = raw & 0x7f;
    let rd = ((raw >> 7) & 0x1f) as u8;
    let funct3 = ((raw >> 12) & 0x7) as u8;
    let rs1 = ((raw >> 15) & 0x1f) as u8;
    let rs2 = ((raw >> 20) & 0x1f) as u8;
    let funct7 = (raw >> 25) as u8;
    match opcode {
        0x37 => Decoded::Instr(Instr::Lui { rd, imm: ((raw >> 12) as u64) << 12 }),
        0x17 => Decoded::Instr(Instr::Auipc { rd, imm: ((raw >> 12) as u64) << 12 }),
        0x6f => {
            let imm = ((raw >> 31) & 1) << 20
                | ((raw >> 12) & 0xff) << 12
                | ((raw >> 20) & 1) << 11
                | ((raw >> 21) & 0x3ff) << 1;
            Decoded::Instr(Instr::Jal { rd, imm: sext(imm as u64, 21) })
        }
        0x67 => {
            let imm = raw >> 20;
            Decoded::Instr(Instr::Jalr { rd, rs1, imm: sext(imm as u64, 12) })
        }
        0x63 => {
            let imm = ((raw >> 31) & 1) << 12
                | ((raw >> 7) & 1) << 11
                | ((raw >> 25) & 0x3f) << 5
                | ((raw >> 8) & 0xf) << 1;
            Decoded::Instr(Instr::Branch { op: funct3, rs1, rs2, imm: sext(imm as u64, 13) })
        }
        0x03 => {
            let imm = raw >> 20;
            Decoded::Instr(Instr::Load { op: funct3, rd, rs1, imm: sext(imm as u64, 12) })
        }
        0x23 => {
            let imm = ((raw >> 25) & 0x7f) << 5 | ((raw >> 7) & 0x1f);
            Decoded::Instr(Instr::Store { op: funct3, rs1, rs2, imm: sext(imm as u64, 12) })
        }
        0x13 => {
            let imm = raw >> 20;
            Decoded::Instr(Instr::OpImm { op: funct3, rd, rs1, imm: sext(imm as u64, 12) })
        }
        0x33 => Decoded::Instr(Instr::Op { op: funct3 | (funct7 << 3), rd, rs1, rs2 }),
        0x0f => Decoded::Instr(Instr::Fence), // FENCE / FENCE.I
        0x73 => {
            if raw == 0x0000_0073 {
                Decoded::Ecall
            } else if raw == 0x0010_0073 {
                Decoded::Ebreak
            } else if raw == 0x3020_0073 {
                Decoded::Mret
            } else {
                let csr = (raw >> 20) as u64;
                let zimm = rs1 as u64;
                Decoded::Instr(Instr::Csr { op: funct3, rd, rs1, csr, zimm })
            }
        }
        _ => Decoded::Illegal,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Execute
// ═══════════════════════════════════════════════════════════════════════════

enum ExecErr {
    LoadFault(u64),
    StoreFault(u64),
    Illegal(u32),
}

impl Rv32Cpu {
    fn execute(
        &mut self,
        base_pc: u64,
        instr: Instr,
        mem: &mut dyn MemoryPort,
    ) -> Result<(), ExecErr> {
        match instr {
            Instr::Lui { rd, imm } => {
                self.write_rd(rd, imm as u64);
                self.pc = base_pc + 4;
            }
            Instr::Auipc { rd, imm } => {
                self.write_rd(rd, (base_pc as i64).wrapping_add(imm as i64) as u64);
                self.pc = base_pc + 4;
            }
            Instr::Jal { rd, imm } => {
                self.write_rd(rd, base_pc + 4);
                self.pc = (base_pc as i64).wrapping_add(imm) as u64;
            }
            Instr::Jalr { rd, rs1, imm } => {
                let target = (self.regs[rs1 as usize] as i64).wrapping_add(imm) as u64 & !1;
                self.write_rd(rd, base_pc + 4);
                self.pc = target;
            }
            Instr::Branch { op, rs1, rs2, imm } => {
                let a = self.regs[rs1 as usize] as i32;
                let b = self.regs[rs2 as usize] as i32;
                let taken = match op {
                    0 => a == b,                                    // BEQ
                    1 => a != b,                                    // BNE
                    4 => a < b,                                     // BLT
                    5 => a >= b,                                    // BGE
                    6 => (a as u32) < (b as u32),                   // BLTU
                    7 => (a as u32) >= (b as u32),                  // BGEU
                    _ => return Err(ExecErr::Illegal(0)),
                };
                self.pc = if taken {
                    (base_pc as i64).wrapping_add(imm) as u64
                } else {
                    base_pc + 4
                };
            }
            Instr::Load { op, rd, rs1, imm } => {
                let addr = (self.regs[rs1 as usize] as i64).wrapping_add(imm) as u64;
                let size = match op {
                    0 => 1, // LB
                    1 => 2, // LH
                    2 => 4, // LW
                    4 => 1, // LBU
                    5 => 2, // LHU
                    _ => return Err(ExecErr::Illegal(0)),
                };
                let v = mem.read(addr, size).map_err(|_| ExecErr::LoadFault(addr))?;
                let val = match op {
                    0 => sext(v, (size as u32) * 8) as u64,
                    1 => sext(v, (size as u32) * 8) as u64,
                    _ => v,
                };
                self.write_rd(rd, val);
                self.pc = base_pc + 4;
            }
            Instr::Store { op, rs1, rs2, imm } => {
                let addr = (self.regs[rs1 as usize] as i64).wrapping_add(imm) as u64;
                let size = match op {
                    0 => 1,
                    1 => 2,
                    2 => 4,
                    _ => return Err(ExecErr::Illegal(0)),
                };
                mem.write(addr, size, self.regs[rs2 as usize])
                    .map_err(|_| ExecErr::StoreFault(addr))?;
                self.pc = base_pc + 4;
            }
            Instr::OpImm { op, rd, rs1, imm } => {
                let a = self.regs[rs1 as usize] as u32;
                let imm32 = imm as u32;
                let val = match op {
                    0 => a.wrapping_add(imm32),                     // ADDI
                    1 => a.wrapping_shl(imm32 & 0x1f),              // SLLI
                    2 => ((a as i32) < (imm as i32)) as u32,        // SLTI
                    3 => (a < imm32) as u32,                        // SLTIU
                    4 => a ^ imm32,                                 // XORI
                    5 => {
                        // SRLI (imm[11:5]=0) / SRAI (imm[11:5]=0x20 → bit 10).
                        let shamt = imm32 & 0x1f;
                        if imm & 0x400 != 0 {
                            ((a as i32) >> shamt) as u32
                        } else {
                            a >> shamt
                        }
                    }
                    6 => a | imm32,                                 // ORI
                    7 => a & imm32,                                 // ANDI
                    _ => return Err(ExecErr::Illegal(0)),
                };
                self.write_rd(rd, val as u64);
                self.pc = base_pc + 4;
            }
            Instr::Op { op, rd, rs1, rs2 } => {
                let a = self.regs[rs1 as usize] as u32;
                let b = self.regs[rs2 as usize] as u32;
                let val = match op {
                    // funct7=0 (bit 3 = 0): integer ops
                    0x0 => a.wrapping_add(b),                       // ADD
                    0x20 => a.wrapping_sub(b),                      // SUB
                    0x1 => a.wrapping_shl(b & 0x1f),                // SLL
                    0x2 => ((a as i32) < (b as i32)) as u32,        // SLT
                    0x3 => (a < b) as u32,                          // SLTU
                    0x4 => a ^ b,                                   // XOR
                    0x5 => a >> (b & 0x1f),                         // SRL
                    0x25 => ((a as i32) >> (b & 0x1f)) as u32,      // SRA
                    0x6 => a | b,                                   // OR
                    0x7 => a & b,                                   // AND
                    // funct7=1 (bit 3 = 1): M extension
                    0x8 => (a as u32).wrapping_mul(b as u32),       // MUL (low)
                    0x9 => {
                        // MULH: high dari (a_sign * b_sign) 64-bit
                        (((a as i64) * (b as i64)) >> 32) as u32
                    }
                    0xa => {
                        // MULHSU
                        (((a as i64 as i128) * (b as u32 as u64 as i128)) >> 32) as u32
                    }
                    0xb => {
                        // MULHU
                        (((a as u64) * (b as u64)) >> 32) as u32
                    }
                    0xc => {
                        // DIV
                        if b == 0 {
                            0xffff_ffff
                        } else if a == 0x8000_0000 && b == 0xffff_ffff {
                            a
                        } else {
                            ((a as i32) / (b as i32)) as u32
                        }
                    }
                    0xd => {
                        // DIVU
                        if b == 0 {
                            0xffff_ffff
                        } else {
                            a / b
                        }
                    }
                    0xe => {
                        // REM
                        if b == 0 {
                            a
                        } else if a == 0x8000_0000 && b == 0xffff_ffff {
                            0
                        } else {
                            ((a as i32) % (b as i32)) as u32
                        }
                    }
                    0xf => {
                        // REMU
                        if b == 0 {
                            a
                        } else {
                            a % b
                        }
                    }
                    _ => return Err(ExecErr::Illegal(0)),
                };
                self.write_rd(rd, val as u64);
                self.pc = base_pc + 4;
            }
            Instr::Fence => {
                self.pc = base_pc + 4; // nop (single-core)
            }
            Instr::Csr { op, rd, rs1, csr, zimm } => {
                let old = self.csr_read(csr).ok_or(ExecErr::Illegal(0))?;
                let (new, _wval) = match op {
                    // BUG FIX: CSRRW (op=1) menulis nilai REGISTER rs1 —
                    // sebelumnya menulis `zimm` (= field rs1 sebagai angka),
                    // sehingga `csrrw x0, mtvec, a0` (a0=0x800) menyetel mtvec=10
                    // bukan 0x800 → trap melompat ke alamat salah.
                    1 => {
                        let w = self.regs[rs1 as usize];
                        (Some(w), w)
                    }
                    2 => {
                        // CSRRS: set bit
                        let w = if rs1 != 0 { old | self.regs[rs1 as usize] } else { old };
                        (if rs1 != 0 { Some(w) } else { None }, w)
                    }
                    3 => {
                        // CSRRC: clear bit
                        let w = if rs1 != 0 { old & !self.regs[rs1 as usize] } else { old };
                        (if rs1 != 0 { Some(w) } else { None }, w)
                    }
                    5 => (Some(zimm), zimm),                              // CSRRWI
                    6 => {
                        let w = if zimm != 0 { old | zimm } else { old };
                        (if zimm != 0 { Some(w) } else { None }, w)
                    }
                    7 => {
                        let w = if zimm != 0 { old & !zimm } else { old };
                        (if zimm != 0 { Some(w) } else { None }, w)
                    }
                    _ => return Err(ExecErr::Illegal(0)),
                };
                if let Some(w) = new {
                    if !self.csr_write(csr, w) {
                        return Err(ExecErr::Illegal(0));
                    }
                }
                self.write_rd(rd, old);
                self.pc = base_pc + 4;
            }
        }
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::{MemoryMap, RamRegion, RegionKind};
    use maria_core::intern::Symbol;

    fn map() -> MemoryMap {
        let mut m = MemoryMap::new();
        // Kode di 0x0..0x100000 (program ELF/test di 0x100-0x700), data di
        // 0x8000_0000 (store/load — alamat memori konvensional bare-metal).
        m.add(RamRegion::new(Symbol::intern("code"), 0x0, 0x10_0000, RegionKind::Ram, false).unwrap()).unwrap();
        m.add(RamRegion::new(Symbol::intern("data"), 0x8000_0000, 0x10_0000, RegionKind::Ram, false).unwrap()).unwrap();
        m
    }

    /// Helper encoding RV32I.
    fn r(f7: u8, rs2: u8, rs1: u8, f3: u8, rd: u8, op: u32) -> u32 {
        ((f7 as u32) << 25) | ((rs2 as u32) << 20) | ((rs1 as u32) << 15) | ((f3 as u32) << 12) | ((rd as u32) << 7) | op
    }

    fn i(imm: i32, rs1: u8, f3: u8, rd: u8, op: u32) -> u32 {
        ((imm as u32 & 0xfff) << 20) | ((rs1 as u32) << 15) | ((f3 as u32) << 12) | ((rd as u32) << 7) | op
    }

    fn s(imm: i32, rs2: u8, rs1: u8, f3: u8, op: u32) -> u32 {
        let imm = imm as u32 & 0xfff;
        ((imm >> 5) << 25) | ((rs2 as u32) << 20) | ((rs1 as u32) << 15) | ((f3 as u32) << 12) | ((imm & 0x1f) << 7) | op
    }

    fn b(imm: i32, rs2: u8, rs1: u8, f3: u8, op: u32) -> u32 {
        let imm = imm as u32 & 0x1fff;
        ((imm >> 12) << 31) | (((imm >> 5) & 0x3f) << 25) | ((rs2 as u32) << 20) | ((rs1 as u32) << 15) | ((f3 as u32) << 12) | (((imm >> 1) & 0xf) << 8) | (((imm >> 11) & 1) << 7) | op
    }

    const ADDI: u32 = 0x13;
    const LUI: u32 = 0x37;
    const SW: u32 = 0x23;
    const LW: u32 = 0x03;
    const OP: u32 = 0x33;
    const JALR: u32 = 0x67;
    const BR: u32 = 0x63;
    const SYSTEM: u32 = 0x73;
    const CSRRW: u32 = 0x73;

    /// Encoding JAL: imm[20|10:1|11|19:12], rd.
    fn jal(imm: i32, rd: u8) -> u32 {
        let imm = imm as u32 & 0x1f_ffff;
        ((imm >> 20) & 1) << 31
            | (((imm >> 1) & 0x3ff) << 21)
            | (((imm >> 11) & 1) << 20)
            | (((imm >> 12) & 0xff) << 12)
            | ((rd as u32) << 7)
            | 0x6f
    }

    fn load_code(cpu: &mut Rv32Cpu, mem: &mut MemoryMap, base: u64, words: &[u32]) {
        cpu.set_pc(base);
        for (i, w) in words.iter().enumerate() {
            mem.write(base + (i as u64) * 4, 4, *w as u64).unwrap();
        }
    }

    fn run(cpu: &mut Rv32Cpu, mem: &mut MemoryMap, n: u64) {
        cpu.run_steps(n, mem).unwrap();
    }

    #[test]
    fn test_addi_lui_sw_lw() {
        let mut m = map();
        let mut cpu = Rv32Cpu::new();
        let code = [
            i(42, 0, 0, 5, ADDI),                      // t0 = 42
            (0x80000u32 << 12) | (10u32 << 7) | LUI,   // a0 = 0x80000000
            s(0, 5, 10, 2, SW),                        // mem[a0] = t0
            i(0, 10, 2, 11, LW),                     // a1 = mem[a0]
            r(0, 5, 11, 0, 12, OP),                  // a2 = a1 + t0
        ];
        load_code(&mut cpu, &mut m, 0x100, &code);
        run(&mut cpu, &mut m, 5);
        assert_eq!(cpu.read_reg(5), 42);
        assert_eq!(cpu.read_reg(10), 0x8000_0000);
        assert_eq!(cpu.read_reg(11), 42);
        assert_eq!(cpu.read_reg(12), 84);
        assert_eq!(m.read(0x8000_0000, 4).unwrap(), 42);
    }

    #[test]
    fn test_branches() {
        let mut m = map();
        let mut cpu = Rv32Cpu::new();
        // t0=1, t1=2; beq t0,t1,+12 (skip); addi t2,zero,1; jal +8; addi t2,zero,2
        let code = [
            i(1, 0, 0, 5, ADDI),   // t0 = 1
            i(2, 0, 0, 6, ADDI),   // t1 = 2
            b(12, 6, 5, 0, BR),    // beq t0,t1,+12 → not taken
            i(1, 0, 0, 7, ADDI),   // t2 = 1
            jal(8, 0),             // jal x0, +8 → skip t2=2
            i(2, 0, 0, 7, ADDI),   // t2 = 2
            b(-8, 6, 5, 0, BR),    // beq t0,t1,-8 → not taken
        ];
        load_code(&mut cpu, &mut m, 0x100, &code);
        run(&mut cpu, &mut m, 6);
        assert_eq!(cpu.read_reg(7), 1, "beq tak diambil; jal lompat; t2=1");
        // BLT: t0=1 < t1=2 → diambil
        let mut cpu2 = Rv32Cpu::new();
        let code2 = [
            i(1, 0, 0, 5, ADDI),   // t0=1
            i(2, 0, 0, 6, ADDI),   // t1=2
            b(8, 6, 5, 4, BR),     // blt t0,t1,+8 → diambil
            i(9, 0, 0, 7, ADDI),   // t2=9 (skip)
            i(5, 0, 0, 7, ADDI),   // t2=5
        ];
        load_code(&mut cpu2, &mut m, 0x200, &code2);
        run(&mut cpu2, &mut m, 4);
        assert_eq!(cpu2.read_reg(7), 5);
    }

    #[test]
    fn test_jal_jalr_auipc() {
        let mut m = map();
        let mut cpu = Rv32Cpu::new();
        // auipc t0, 0 → t0 = pc; jalr ra, t0, 8 (call ke +8); addi t1,zero,1; addi t1,zero,2
        let base = 0x300u64;
        let code = [
            r(0, 0, 0, 0, 5, 0x17),  // auipc t0, 0
            i(8, 5, 0, 1, JALR),     // jalr ra, t0, 8 → target = pc+8
            i(1, 0, 0, 6, ADDI),     // t1=1
            i(2, 0, 0, 6, ADDI),     // t1=2
        ];
        load_code(&mut cpu, &mut m, base, &code);
        run(&mut cpu, &mut m, 4);
        assert_eq!(cpu.read_reg(5), base, "auipc → pc");
        // jalr di base+4 → ra = pc+4 = base+8.
        assert_eq!(cpu.read_reg(1), base + 8, "ra = pc+4 dari jalr");
        assert_eq!(cpu.pc(), base + 16);
        assert_eq!(cpu.read_reg(6), 2, "jalr melompat ke +8 (instruksi ke-3)");
    }

    #[test]
    fn test_mul_div_rem() {
        let mut m = map();
        let mut cpu = Rv32Cpu::new();
        // t0 = -3 (0xFFFFFFFD), t1 = 7
        let code = [
            i(-3, 0, 0, 5, ADDI),
            i(7, 0, 0, 6, ADDI),
            r(1, 6, 5, 0, 7, OP),  // t2 = t0 * t1 (low) = -21 = 0xFFFFFFEB
            r(1, 6, 5, 4, 8, OP),  // t3 = t0 / t1 (div) = 0
            r(1, 6, 5, 6, 9, OP),  // t4 = t0 % t1 (rem) = -3
        ];
        load_code(&mut cpu, &mut m, 0x400, &code);
        run(&mut cpu, &mut m, 5);
        assert_eq!(cpu.read_reg(7) as u32, 0xffff_ffeb);
        assert_eq!(cpu.read_reg(8), 0);
        assert_eq!(cpu.read_reg(9) as u32, 0xffff_fffd);
    }

    #[test]
    fn test_div_by_zero() {
        let mut m = map();
        let mut cpu = Rv32Cpu::new();
        // t0=5, t1=0; div → -1; divu → 0xFFFFFFFF; rem → 5; remu → 5
        let code = [
            i(5, 0, 0, 5, ADDI),
            i(0, 0, 0, 6, ADDI),
            r(1, 6, 5, 4, 7, OP), // DIV t2 = t0/t1
            r(1, 6, 5, 5, 8, OP), // DIVU t3
            r(1, 6, 5, 6, 9, OP), // REM t4
            r(1, 6, 5, 7, 10, OP), // REMU t5
        ];
        load_code(&mut cpu, &mut m, 0x500, &code);
        run(&mut cpu, &mut m, 6);
        assert_eq!(cpu.read_reg(7) as u32, 0xffff_ffff);
        assert_eq!(cpu.read_reg(8) as u32, 0xffff_ffff);
        assert_eq!(cpu.read_reg(9), 5);
        assert_eq!(cpu.read_reg(10), 5);
    }

    #[test]
    fn test_shifts() {
        let mut m = map();
        let mut cpu = Rv32Cpu::new();
        // t0 = 0x80000001 (lui 0x80000 + addi 1); slli → 2; srli → 0x40000000;
        // srai t0,31 → -1.
        let code = [
            (0x80000u32 << 12) | (5u32 << 7) | LUI, // t0 = 0x80000000
            i(1, 5, 0, 5, ADDI),                    // t0 = 0x80000001
            i(1, 5, 1, 6, ADDI),                    // slli t1,t0,1
            i(1, 5, 5, 7, ADDI),                    // srli t2,t0,1
            i(0x41f, 5, 5, 8, ADDI),                // srai t3,t0,31 (imm=0x400|31)
        ];
        let _ = OP;
        load_code(&mut cpu, &mut m, 0x600, &code);
        run(&mut cpu, &mut m, 5);
        assert_eq!(cpu.read_reg(6), 2);
        assert_eq!(cpu.read_reg(7), 0x4000_0000);
        assert_eq!(cpu.read_reg(8) as u32, 0xffff_ffff);
    }

    #[test]
    fn test_csr_roundtrip_and_mret() {
        let mut m = map();
        let mut cpu = Rv32Cpu::new();
        // a0 = 0x100; csrrw t0, mtvec, a0; csrr t1, mtvec (csrrs x0); csrrw t2, mtvec, x0
        let code = [
            i(0x100, 0, 0, 10, ADDI),  // a0 = 0x100
            i(0x305, 10, 1, 5, CSRRW), // csrrw t0, mtvec, a0
            i(0x305, 0, 2, 6, CSRRW),  // csrrs t1, mtvec, x0 → t1 = mtvec
            i(0x305, 0, 1, 7, CSRRW),  // csrrw t2, mtvec, x0
        ];
        load_code(&mut cpu, &mut m, 0x700, &code);
        run(&mut cpu, &mut m, 4);
        assert_eq!(cpu.read_reg(5), 0, "csrrw t0 membaca mtvec lama (0)");
        assert_eq!(cpu.read_reg(6), 0x100, "csrrs t1 membaca mtvec baru");
        assert_eq!(cpu.read_reg(7), 0x100, "csrrw t2 membaca mtvec, lalu tulis 0");
        assert_eq!(cpu.csr(CSR_MTVEC), Some(0));
    }

    #[test]
    fn test_ecall_trap_and_mret() {
        let mut m = map();
        let mut cpu = Rv32Cpu::new();
        // Handler di 0x700 (isi mret). ecall → trap → handler → mret.
        // (0x800 tak bisa di-encode imm ADDI 12-bit — 0x800 = -2048 sign-
        // extended — jadi pakai 0x700 yang masih dalam range positif.)
        let base = 0x1000u64;
        let handler = 0x700u64;
        let code = [
            i(0x700, 0, 0, 10, ADDI),   // a0 = 0x700
            i(0x305, 10, 1, 0, CSRRW),  // csrrw x0, mtvec, a0
            i(0x300, 8, 5, 0, SYSTEM),  // csrrwi x0, mstatus, 8 → MIE=1
            0x0000_0073,                // ecall
        ];
        load_code(&mut cpu, &mut m, base, &code);
        // Tulis handler (mret) di 0x700.
        m.write(handler, 4, 0x3020_0073u32 as u64).unwrap();
        run(&mut cpu, &mut m, 5);
        assert_eq!(cpu.csr(CSR_MCAUSE), Some(CAUSE_ECALL_M));
        assert_eq!(cpu.csr(CSR_MEPC), Some(base + 12), "mepc = pc instruksi ecall");
        assert_eq!(cpu.pc(), base + 12, "mret kembali ke mepc (instruksi ecall)");
        assert_eq!(cpu.csr(CSR_MSTATUS).unwrap() & MSTATUS_MIE as u64, MSTATUS_MIE as u64, "MIE dipulihkan oleh mret");
    }

    #[test]
    fn test_load_fault_trap() {
        let mut m = map();
        let mut cpu = Rv32Cpu::new();
        // mtvec = 0x200 (mret di sana), lalu lw a0, 0(a1) dengan a1 = 0xDEAD0000 (unmapped)
        let code = [
            i(0x200, 0, 0, 10, ADDI),   // a0 = 0x200
            i(0x305, 10, 1, 0, CSRRW),  // mtvec = a0
            i(0xdead, 0, 0, 11, ADDI),  // a1 = 0xDEAD (imm 12-bit: 0xDEAD & 0xfff = 0xead)
        ];
        // a1 = 0xDEAD0000 via lui+addi:
        let code = [
            i(0x200, 0, 0, 10, ADDI),
            i(0x305, 10, 1, 0, CSRRW),
            (0xdead0u32 << 12) | (11u32 << 7) | LUI, // a1 = 0xDEAD0000
            i(0, 11, 2, 12, LW),                     // a2 = lw 0(a1) → fault
        ];
        load_code(&mut cpu, &mut m, 0x1000, &code);
        m.write(0x200, 4, 0x3020_0073u32 as u64).unwrap(); // mret di handler
        run(&mut cpu, &mut m, 5);
        assert_eq!(cpu.csr(CSR_MCAUSE), Some(CAUSE_LOAD_ACCESS_FAULT));
        assert_eq!(cpu.csr(CSR_MTVAL), Some(0xdead_0000));
    }

    #[test]
    fn test_illegal_instruction_trap() {
        let mut m = map();
        let mut cpu = Rv32Cpu::new();
        // 0xFFFFFFFF = instruksi ilegal; mtvec = 0x300 (mret).
        let code = [
            i(0x300, 0, 0, 10, ADDI),
            i(0x305, 10, 1, 0, CSRRW),
            0xffff_ffff,
        ];
        load_code(&mut cpu, &mut m, 0x1000, &code);
        m.write(0x300, 4, 0x3020_0073u32 as u64).unwrap();
        run(&mut cpu, &mut m, 4);
        assert_eq!(cpu.csr(CSR_MCAUSE), Some(CAUSE_ILLEGAL_INSTRUCTION));
        assert_eq!(cpu.csr(CSR_MTVAL), Some(0xffff_ffff));
    }

    #[test]
    fn test_external_interrupt() {
        let mut m = map();
        let mut cpu = Rv32Cpu::new();
        // mstatus.MIE=1 + mie.MSIE=1 via csrrwi (zimm 5-bit = 8), mtvec=0x500,
        // lalu raise irq 3 → trap mcause=0x80000003.
        let code = [
            i(0x500, 0, 0, 10, ADDI),  // a0 = 0x500 (mtvec)
            i(0x305, 10, 1, 0, CSRRW), // mtvec = a0
            i(0x300, 8, 5, 0, SYSTEM), // csrrwi x0, mstatus, 8 (MIE)
            i(0x304, 8, 5, 0, SYSTEM), // csrrwi x0, mie, 8 (MSIE)
        ];
        load_code(&mut cpu, &mut m, 0x1000, &code);
        m.write(0x500, 4, 0x3020_0073u32 as u64).unwrap(); // mret handler
        run(&mut cpu, &mut m, 4);
        assert_eq!(cpu.pc(), 0x1010);
        cpu.raise_interrupt(3, true);
        cpu.step(&mut m).unwrap();
        assert_eq!(cpu.csr(CSR_MCAUSE), Some(0x8000_0000 | 3));
        assert_eq!(cpu.csr(CSR_MEPC), Some(0x1010), "mepc = pc saat interrupt");
        assert_eq!(cpu.pc(), 0x500, "pc = mtvec (handler)");
        cpu.raise_interrupt(3, false);
    }

    #[test]
    fn test_x0_always_zero() {
        let mut m = map();
        let mut cpu = Rv32Cpu::new();
        // coba tulis x0: lui x0, 0x12345 → x0 tetap 0
        let code = [(0x12345u32 << 12) | (0u32 << 7) | LUI];
        load_code(&mut cpu, &mut m, 0x1000, &code);
        run(&mut cpu, &mut m, 1);
        assert_eq!(cpu.read_reg(0), 0);
        assert_eq!(cpu.read_reg(32 - 32), 0); // idx 0
    }

    #[test]
    fn test_fetch_fault_traps() {
        let mut m = map();
        let mut cpu = Rv32Cpu::new();
        cpu.set_pc(0x0f00_0000); // unmapped
        cpu.step(&mut m).unwrap();
        assert_eq!(cpu.csr(CSR_MCAUSE), Some(CAUSE_INST_ACCESS_FAULT));
        assert_eq!(cpu.csr(CSR_MTVAL), Some(0x0f00_0000));
    }

    // ── Integrasi: ELF program penuh (loader + mem + cpu) ──

    fn make_elf(entry: u64, vaddr: u64, payload: &[u8]) -> Vec<u8> {
        let mut d = vec![0u8; 64 + 56];
        d[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        d[4] = 2; // ELF64
        d[5] = 1; // LE
        d[6] = 1;
        d[18..20].copy_from_slice(&243u16.to_le_bytes()); // RISC-V
        d[24..32].copy_from_slice(&entry.to_le_bytes());
        d[32..40].copy_from_slice(&64u64.to_le_bytes());
        d[54..56].copy_from_slice(&56u16.to_le_bytes());
        d[56..58].copy_from_slice(&1u16.to_le_bytes());
        let po = 64usize;
        d[po..po + 4].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
        d[po + 8..po + 16].copy_from_slice(&(64u64 + 56).to_le_bytes());
        d[po + 16..po + 24].copy_from_slice(&vaddr.to_le_bytes());
        d[po + 32..po + 40].copy_from_slice(&(payload.len() as u64).to_le_bytes());
        d[po + 40..po + 48].copy_from_slice(&(payload.len() as u64).to_le_bytes());
        d.extend_from_slice(payload);
        d
    }

    fn code_bytes(words: &[u32]) -> Vec<u8> {
        let mut v = Vec::with_capacity(words.len() * 4);
        for w in words {
            v.extend_from_slice(&w.to_le_bytes());
        }
        v
    }

    #[test]
    fn test_run_elf_program_end_to_end() {
        // Program: t0=42; a0=0x80000000; sw t0,0(a0); lw a1,0(a0); a2=a1+t0; ebreak
        let code = [
            i(42, 0, 0, 5, ADDI),
            (0x80000u32 << 12) | (10u32 << 7) | LUI,
            s(0, 5, 10, 2, SW),
            i(0, 10, 2, 11, LW),
            r(0, 5, 11, 0, 12, OP),
            0x0010_0073, // ebreak
        ];
        let mut m = map();
        let elf = make_elf(0x8000_0000, 0x8000_0000, &code_bytes(&code));
        let entry = crate::elf::load_elf(&elf, &mut m).expect("load elf");
        assert_eq!(entry, 0x8000_0000);

        let mut cpu = Rv32Cpu::new();
        cpu.set_pc(entry);
        let done = cpu.run_steps(6, &mut m).unwrap();
        assert_eq!(done, 6);
        assert_eq!(cpu.read_reg(12), 84, "a2 = a1 + t0");
        assert_eq!(cpu.read_reg(11), 42, "a1 = lw");
        assert_eq!(m.read(0x8000_0000, 4).unwrap(), 42);
        assert_eq!(cpu.csr(CSR_MCAUSE), Some(CAUSE_BREAKPOINT), "ebreak");
        assert_eq!(cpu.csr(CSR_MEPC), Some(0x8000_0014), "mepc = pc ebreak");
        assert_eq!(cpu.cycles(), 6);
    }
}
