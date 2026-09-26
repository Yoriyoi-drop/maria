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
            format!(
                "{} — console: [{}] ({} bytes)",
                head,
                txt,
                self.console.len()
            )
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
    /// Counter langkah/cycle KUMULATIF lintas run — restore snapshot membuat
    /// run lanjutan dihitung dari angka tersimpan (summary CLI utuh).
    steps: u64,
    cycles: u64,
}

impl Machine {
    pub fn new(cpu: Box<dyn CpuCore>, mem: MemoryMap, max_steps: u64) -> Self {
        Self {
            cpu,
            mem,
            max_steps,
            steps: 0,
            cycles: 0,
        }
    }

    /// Jalankan sampai trap / max_steps. `MmioAccess` (R4, co-sim RTL device)
    /// diabaikan untuk saat ini — RAM/ROM di-service langsung oleh MemoryMap.
    pub fn run(&mut self) -> Result<MachineResult, CpuFault> {
        let mut run_steps = 0u64;
        let mut run_cycles = 0u64;
        while run_steps < self.max_steps {
            match self.cpu.step(&mut self.mem)? {
                CpuStep::InstructionExecuted { cycles: c } => {
                    run_steps += 1;
                    run_cycles += c;
                    // Interpreter (RV32): trap internal → ebreak tak pernah
                    // jadi CpuStep::Trap; berhenti via halt_status (setara
                    // sinyal `trap` Direct RTL CPU).
                    if let Some((cause, tval)) = self.cpu.halt_status() {
                        let steps = self.steps + run_steps;
                        let cycles = self.cycles + run_cycles;
                        self.steps = steps;
                        self.cycles = cycles;
                        return Ok(MachineResult {
                            steps,
                            cycles,
                            // tval = alamat instruksi penyebab (ebreak).
                            pc: tval,
                            halted: true,
                            cause,
                            tval,
                            console: self.cpu.console_output().to_vec(),
                        });
                    }
                }
                CpuStep::MmioAccess { .. } => {
                    // R4: dispatch ke Direct RTL Device. Belum diimplementasikan.
                }
                CpuStep::Trap { cause, tval } => {
                    let steps = self.steps + run_steps + 1;
                    let cycles = self.cycles + run_cycles;
                    self.steps = steps;
                    self.cycles = cycles;
                    return Ok(MachineResult {
                        steps,
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
        self.steps += run_steps;
        self.cycles += run_cycles;
        Ok(MachineResult {
            steps: self.steps,
            cycles: self.cycles,
            pc: self.cpu.pc(),
            halted: false,
            cause: 0,
            tval: 0,
            console: self.cpu.console_output().to_vec(),
        })
    }

    /// Ambil snapshot mesin (CPU + seluruh region memori + counter) —
    /// EMULATOR.md §14 (R5). CPU tanpa dukungan snapshot (RTL-linked/JIT
    /// stub) → `Err` jelas.
    pub fn snapshot(&self) -> Result<crate::snapshot::MachineSnapshot, String> {
        let cpu = self.cpu.snapshot()?;
        let regions = self
            .mem
            .regions
            .iter()
            .map(|r| crate::snapshot::RegionSnapshot {
                name: r.name.as_str().to_string(),
                base: r.base,
                size: r.size,
                pages: crate::snapshot::MachineSnapshot::encode_pages(r.bytes()),
            })
            .collect();
        Ok(crate::snapshot::MachineSnapshot {
            steps: self.steps,
            cycles: self.cycles,
            cpu,
            regions,
        })
    }

    /// Restore snapshot: state CPU + isi region (region di-zero dulu lalu
    /// halaman tersimpan ditulis ulang → identik dengan saat disimpan).
    /// Region di memory map HARUS persis cocok (nama/base/size) — RAM size
    /// berbeda = state tak bisa dipindahkan, error jelas.
    pub fn restore(&mut self, snap: &crate::snapshot::MachineSnapshot) -> Result<(), String> {
        // 1. State CPU dulu — gagal = tak ada perubahan apa pun.
        self.cpu.restore(&snap.cpu)?;

        // 2. Validasi pasangan region dua arah (snapshot ↔ map).
        let mut matched = vec![false; self.mem.regions.len()];
        for rs in &snap.regions {
            let idx = self
                .mem
                .regions
                .iter()
                .enumerate()
                .position(|(i, r)| {
                    !matched[i]
                        && r.name.as_str() == rs.name
                        && r.base == rs.base
                        && r.size == rs.size
                })
                .ok_or_else(|| {
                    format!(
                        "snapshot: region '{}' (0x{:x}, {} byte) tidak cocok dengan memory map \
                         (nama/base/size RAM harus sama — cek ram di config .meu)",
                        rs.name, rs.base, rs.size
                    )
                })?;
            matched[idx] = true;
        }
        if let Some(missing) = matched.iter().position(|m| !m) {
            let r = &self.mem.regions[missing];
            return Err(format!(
                "snapshot: region '{}' (0x{:x}, {} byte) tidak ada di snapshot — \
                 snapshot dibuat dari memory map berbeda",
                r.name.as_str(),
                r.base,
                r.size
            ));
        }

        // 3. Tulis ulang halaman (sparse): zero penuh dulu → halaman yang
        // tidak ada di snapshot kembali nol persis seperti saat disimpan.
        for rs in &snap.regions {
            let region = self
                .mem
                .regions
                .iter_mut()
                .find(|r| r.name.as_str() == rs.name && r.base == rs.base && r.size == rs.size)
                .expect("validated above");
            region.fill_zero();
            for (page_idx, data) in &rs.pages {
                let off = *page_idx as usize * crate::snapshot::PAGE;
                if off + data.len() > rs.size as usize {
                    return Err(format!(
                        "snapshot: region '{}' halaman {} melebihi ukuran region",
                        rs.name, page_idx
                    ));
                }
                region
                    .write_slice(rs.base + off as u64, data)
                    .map_err(|e| e.to_string())?;
            }
        }
        self.steps = snap.steps;
        self.cycles = snap.cycles;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::{Isa, Rv32Cpu};
    use crate::mem::{MemoryMap, MemoryPort, RamRegion, RegionKind};
    use mivon_core::intern::Symbol;

    #[test]
    fn test_machine_summary_halted() {
        let r = MachineResult {
            steps: 7,
            cycles: 42,
            pc: 0x8000_001c,
            halted: true,
            cause: 11,
            tval: 0x8000_001c,
            console: Vec::new(),
        };
        let s = r.summary();
        assert!(s.contains("halted"));
        assert!(s.contains("cause=11"));
        // Console RTL device ikut di-ringkas.
        let r2 = MachineResult {
            steps: 3,
            cycles: 12,
            pc: 0x1000,
            halted: true,
            cause: 11,
            tval: 0x1000,
            console: b"ABC".to_vec(),
        };
        assert!(r2.summary().contains("console: [ABC] (3 bytes)"));
    }

    #[test]
    fn test_machine_max_steps_no_halt() {
        let mut mem = MemoryMap::new();
        mem.add(
            RamRegion::new(Symbol::intern("ram"), 0x0, 0x1000, RegionKind::Ram, false).unwrap(),
        )
        .unwrap();
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

    /// RAM shape identik dengan program snapshot (lihat test di bawah).
    fn snap_mem() -> MemoryMap {
        let mut mem = MemoryMap::new();
        mem.add(
            RamRegion::new(Symbol::intern("ram"), 0x0, 0x1000, RegionKind::Ram, false).unwrap(),
        )
        .unwrap();
        // addi x1,x0,42; sw x1,0x40(x0); 6x addi x1,x1,1; ebreak
        let code: [u32; 9] = [
            0x02a0_0093, // addi x1, x0, 42
            0x0410_2023, // sw   x1, 0x40(x0)
            0x0010_8093,
            0x0010_8093,
            0x0010_8093,
            0x0010_8093,
            0x0010_8093,
            0x0010_8093, // addi x1, x1, 1 (x6)
            0x0010_0073, // ebreak
        ];
        for (i, w) in code.iter().enumerate() {
            mem.write(i as u64 * 4, 4, *w as u64).unwrap();
        }
        mem
    }

    #[test]
    fn test_machine_snapshot_resume_deterministic() {
        // ── Mesin A: jalan 3 step → snapshot → lanjut sampai trap ──
        let mut a = Machine::new(Box::new(Rv32Cpu::new()), snap_mem(), 3);
        let r1 = a.run().unwrap();
        assert!(!r1.halted);
        assert_eq!(r1.steps, 3);
        let snap = a.snapshot().expect("snapshot rv32 didukung");
        assert_eq!(snap.steps, 3, "counter kumulatif ikut snapshot");
        assert!(!snap.cpu.is_empty());
        assert_eq!(snap.regions.len(), 1);
        assert_eq!(snap.regions[0].pages.len(), 1, "code+data satu halaman");

        let r2 = {
            a.max_steps = 100; // lanjutan: batas run kedua
            a.run().unwrap()
        };
        assert!(r2.halted, "lanjut → ebreak");
        assert_eq!(r2.steps, 9, "3 sebelum snapshot + 5 addi + ebreak");
        assert_eq!(a.cpu.read_reg(1), 48, "42 + 6 increment");
        let mem_a = a.mem.regions[0].bytes().to_vec();

        // ── Mesin B: mesin baru kosong → restore → run sama ──
        let mut b = Machine::new(Box::new(Rv32Cpu::new()), snap_mem_empty(), 100);
        b.restore(&snap).expect("restore");
        assert_eq!(
            b.mem.read(0x40, 4).unwrap(),
            42,
            "tulisan sebelum snapshot ter-restore"
        );
        let r3 = b.run().unwrap();
        assert!(r3.halted);
        assert_eq!(r3.steps, r2.steps, "steps kumulatif identik");
        assert_eq!(r3.pc, r2.pc);
        assert_eq!(b.cpu.read_reg(1), a.cpu.read_reg(1));
        assert_eq!(b.mem.regions[0].bytes(), mem_a.as_slice(), "memori identik");
    }

    /// RAM shape sama, isi KOSONG (untuk bukti restore benar-benar menulis).
    fn snap_mem_empty() -> MemoryMap {
        let mut mem = MemoryMap::new();
        mem.add(
            RamRegion::new(Symbol::intern("ram"), 0x0, 0x1000, RegionKind::Ram, false).unwrap(),
        )
        .unwrap();
        mem
    }

    #[test]
    fn test_machine_restore_region_mismatch() {
        let mut a = Machine::new(Box::new(Rv32Cpu::new()), snap_mem(), 3);
        a.run().unwrap();
        let snap = a.snapshot().unwrap();

        // RAM beda ukuran → error jelas, state tak berubah.
        let mut mem = MemoryMap::new();
        mem.add(
            RamRegion::new(Symbol::intern("ram"), 0x0, 0x2000, RegionKind::Ram, false).unwrap(),
        )
        .unwrap();
        let mut b = Machine::new(Box::new(Rv32Cpu::new()), mem, 3);
        let err = b.restore(&snap).unwrap_err();
        assert!(err.contains("tidak cocok"), "pesan: {}", err);

        // Snapshot yang kehilangan satu region → error "tidak ada di snapshot".
        let mut mem2 = MemoryMap::new();
        mem2.add(
            RamRegion::new(Symbol::intern("ram"), 0x0, 0x1000, RegionKind::Ram, false).unwrap(),
        )
        .unwrap();
        mem2.add(
            RamRegion::new(
                Symbol::intern("extra"),
                0x2000,
                0x1000,
                RegionKind::Ram,
                false,
            )
            .unwrap(),
        )
        .unwrap();
        let mut c = Machine::new(Box::new(Rv32Cpu::new()), mem2, 3);
        let err = c.restore(&snap).unwrap_err();
        assert!(err.contains("tidak ada di snapshot"), "pesan: {}", err);
    }

    #[test]
    fn test_machine_snapshot_cpu_unsupported() {
        // CPU tanpa implementasi snapshot (stub) → error jelas.
        struct NoSnap;
        impl CpuCore for NoSnap {
            fn reset(&mut self) {}
            fn step(&mut self, _m: &mut dyn crate::mem::MemoryPort) -> Result<CpuStep, CpuFault> {
                Err(CpuFault {
                    pc: 0,
                    reason: "x".into(),
                })
            }
            fn pc(&self) -> u64 {
                0
            }
            fn set_pc(&mut self, _a: u64) {}
            fn raise_interrupt(&mut self, _i: u32, _l: bool) {}
            fn read_reg(&self, _i: usize) -> u64 {
                0
            }
            fn isa(&self) -> Isa {
                Isa::RiscV64
            }
        }
        let mut mem = MemoryMap::new();
        mem.add(RamRegion::new(Symbol::intern("ram"), 0x0, 0x100, RegionKind::Ram, false).unwrap())
            .unwrap();
        let m = Machine::new(Box::new(NoSnap), mem, 1);
        let err = m.snapshot().unwrap_err();
        assert!(err.contains("belum didukung"), "pesan: {}", err);
    }
}
