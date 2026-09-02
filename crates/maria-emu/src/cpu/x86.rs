//! X86Cpu — interpreter x86 real-mode (16-bit + prefix `66`/`67` + segment
//! override) untuk boot chain BIOS: MBR / ISOLINUX hybrid / GRUB boot.img —
//! EMULATOR.md §21 R6 (mesin x86-64; tahap awal = real-mode boot).
//!
//! Target konkret: `ubuntu-26.04-desktop-amd64.iso`.
//! Rantai boot (dianalisis dari ISO, 2026-08-16):
//!   MBR LBA 0 (ISOLINUX hybrid, `eb 63`, INT 13h AH=02) →
//!   El Torito boot catalog LBA 666 → boot image LBA 667 (GRUB boot.img,
//!   `call next`, INT 13h AH=42 extended read, INT 10h AH=0E, `ea` far jump
//!   ke protected mode) → GRUB stage 2 → kernel.
//!
//! Subset instruksi: core 8086 (mov/push/pop/jcc/call/ret/int/lodsb/stosb/
//! movs/test/cmp/add/or/and/sub/xor/shift/lea/cbw/cwd/in/out/hlt) + prefix
//! operand-size 66 (reg 32-bit) + segment override + rep. INT di-dispatch ke
//! BIOS stub (INT 13h disk, INT 10h teletype, INT 16h keyboard, INT 1Ah time).

use super::{CpuCore, CpuFault, CpuStep, Isa};
use crate::mem::MemoryPort;

/// Flag FLAGS (bit positions real mode).
pub const FLAG_CF: u16 = 1 << 0;
pub const FLAG_PF: u16 = 1 << 2;
pub const FLAG_AF: u16 = 1 << 4;
pub const FLAG_ZF: u16 = 1 << 6;
pub const FLAG_SF: u16 = 1 << 7;
pub const FLAG_IF: u16 = 1 << 9;
pub const FLAG_DF: u16 = 1 << 10;
pub const FLAG_OF: u16 = 1 << 11;

/// Backend disk untuk INT 13h (ISO mentah / image).
pub trait X86Disk {
    /// Baca `count` sektor (512-byte unit, HDD) mulai `lba`.
    fn read(&mut self, lba: u64, count: u16, buf: &mut [u8]) -> Result<(), String>;
    fn total_sectors(&self) -> u64;
    /// Baca byte mentah mulai offset byte file (dipakai CD 2048-byte sector:
    /// INT 13h AH=42/AH=4B dengan drive CD → LBA dalam blok 2048).
    fn read_bytes(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), String>;
}

/// Alamat VGA text mode buffer (real mode linear / low memory).
pub const VGA_TEXT_ADDR: u64 = 0xB8000;
/// Ukuran buffer VGA text mode yang di-mirror (16 KB, text mode 4 KB + slack).
pub const VGA_TEXT_SIZE: usize = 0x4000;

/// Interpreter x86 real-mode. Register internal 64-bit (16/32-bit view sesuai
/// prefix 66). Memori dipinjam dari pemanggil via `MemoryPort`.
pub struct X86Cpu {
    /// 8 register umum: AX,CX,DX,BX,SP,BP,SI,DI (nilai penuh 32-bit).
    pub gpr: [u32; 8],
    /// Segmen (16-bit).
    pub cs: u16,
    pub ds: u16,
    pub es: u16,
    pub ss: u16,
    pub fs: u16,
    pub gs: u16,
    /// IP/EIP (offset dalam CS; 32-bit di protected mode).
    pub ip: u32,
    /// FLAGS (bit real mode).
    pub flags: u16,
    /// Disk backend (INT 13h). None → INT 13h gagal (CF set).
    pub disk: Option<Box<dyn X86Disk>>,
    /// Console output (INT 10h AH=0E teletype).
    pub out: Vec<u8>,
    /// CPU berhenti (hlt / INT tak dikenal) → Machine berhenti.
    pub halted: bool,
    pub halt_reason: String,
    /// Jumlah instruksi dieksekusi.
    pub steps: u64,
    /// Geometri CHS dari INT 13h AH=08 (dipakai konversi CHS→LBA AH=02).
    pub drive_spt: u16,
    pub drive_heads: u16,
    /// GDT (lgdt) — base/limit untuk transisi real → protected.
    pub gdt_base: u32,
    pub gdt_limit: u16,
    /// Byte GDT di-cache saat lgdt (dipakai gdt_seg_base).
    pub gdt_cache: Vec<u8>,
    /// CR0..CR7 (bit 0 CR0 = PE).
    pub cr: [u32; 8],
    /// True setelah far jump protected-mode (mode 32-bit aktif).
    pub pmode: bool,
    /// Mirror VGA text buffer (0xB8000+) — setiap sel 2 byte (char+attr).
    /// Diisi oleh write8/16/32 saat alamat masuk region VGA. Dipakai console
    /// pasca-pmode (GRUB/kernel menulis langsung ke VRAM, bukan via INT 10h).
    pub vga: [u8; VGA_TEXT_SIZE],
    /// Nomor drive CD (El Torito no-emul, biasanya 0xE0). INT 13h AH=42 yang
    /// memakai drive ini dibaca dalam blok 2048-byte (CD), bukan 512 (HDD).
    /// None → tidak ada CD; AH=4B AL=01 melaporkan "drive tak ada".
    pub cd_drive: Option<u8>,
    /// Prefetch cache instruksi: 16 byte sejak `pf_base`/`pf_cs`. Mengurangi
    /// access memory-per-byte saat fetch (hot path profil gdb). Di-invalidasi
    /// tiap lompatan.
    pf_valid: bool,
    pf_cs: u16,
    pf_base: u32,
    pf_len: usize,
    pf_data: [u8; 16],
}

impl X86Cpu {
    pub fn new() -> Self {
        Self {
            gpr: [0; 8],
            cs: 0,
            ds: 0,
            es: 0,
            ss: 0,
            fs: 0,
            gs: 0,
            ip: 0,
            flags: FLAG_IF, // IF aktif default
            disk: None,
            out: Vec::new(),
            halted: false,
            halt_reason: String::new(),
            steps: 0,
            drive_spt: 18,
            drive_heads: 2,
            gdt_base: 0,
            gdt_limit: 0,
            gdt_cache: Vec::new(),
            cr: [0; 8],
            pmode: false,
            vga: [0; VGA_TEXT_SIZE],
            cd_drive: None,
            pf_valid: false,
            pf_cs: 0,
            pf_base: 0,
            pf_len: 0,
            pf_data: [0; 16],
        }
    }

    /// Ambil teks VGA (80x25) dari buffer mirror — cell = char@off, attr@off+1.
    /// Baris 80 kolom; atribut dibuang; baris kosong di-skip.
    pub fn vga_text(&self) -> String {
        let mut s = String::new();
        for row in 0..25 {
            let base = row * 80 * 2;
            let mut line = String::new();
            for col in 0..80 {
                let ch = self.vga[base + col * 2];
                line.push(if (32..=126).contains(&ch) {
                    ch as char
                } else {
                    ' '
                });
            }
            let trimmed = line.trim_end();
            if !trimmed.is_empty() {
                s.push_str(trimmed);
                s.push('\n');
            }
        }
        s
    }

    // ── Akses register 8-bit. Indeks x86: 0=AL 1=CL 2=DL 3=BL 4=AH 5=CH 6=DH 7=BH.
    // AL..BL = low byte gpr[0..3]; AH..BH = high byte gpr[0..3].
    #[inline]
    pub fn r8(&self, i: usize) -> u8 {
        if i < 4 {
            (self.gpr[i] & 0xff) as u8
        } else {
            ((self.gpr[i - 4] >> 8) & 0xff) as u8
        }
    }
    #[inline]
    pub fn r8h(&self, i: usize) -> u8 {
        self.r8(i + 4)
    }
    #[inline]
    pub fn r8_set(&mut self, i: usize, v: u8) {
        if i < 4 {
            self.gpr[i] = (self.gpr[i] & !0xff) | v as u32;
        } else {
            self.gpr[i - 4] = (self.gpr[i - 4] & !0xff00) | ((v as u32) << 8);
        }
    }
    #[inline]
    pub fn r8h_set(&mut self, i: usize, v: u8) {
        self.r8_set(i + 4, v);
    }
    #[inline]
    pub fn r16(&self, i: usize) -> u16 {
        (self.gpr[i] & 0xffff) as u16
    }
    #[inline]
    pub fn r16_set(&mut self, i: usize, v: u16) {
        self.gpr[i] = (self.gpr[i] & !0xffff) | v as u32;
    }
    #[inline]
    pub fn r32(&self, i: usize) -> u32 {
        self.gpr[i]
    }
    #[inline]
    pub fn r32_set(&mut self, i: usize, v: u32) {
        self.gpr[i] = v;
    }

    // ── Akses register khusus (SP=4, CX=1, SI=6, DI=7) ──
    #[inline]
    pub fn sp(&self) -> u16 {
        self.r16(4)
    }
    #[inline]
    pub fn sp_set(&mut self, v: u16) {
        self.r16_set(4, v);
    }
    #[inline]
    pub fn cx(&self) -> u16 {
        self.r16(1)
    }
    #[inline]
    pub fn cx_set(&mut self, v: u16) {
        self.r16_set(1, v);
    }
    #[inline]
    pub fn si(&self) -> u16 {
        self.r16(6)
    }
    #[inline]
    pub fn si_set(&mut self, v: u16) {
        self.r16_set(6, v);
    }
    #[inline]
    pub fn di(&self) -> u16 {
        self.r16(7)
    }
    #[inline]
    pub fn di_set(&mut self, v: u16) {
        self.r16_set(7, v);
    }
    /// DL (low byte DX) — nomor drive BIOS.
    #[inline]
    pub fn dl(&self) -> u8 {
        self.r8(2)
    }
    #[inline]
    pub fn dl_set(&mut self, v: u8) {
        self.r8_set(2, v);
    }

    // ── FLAGS ──
    #[inline]
    fn set_flag(&mut self, bit: u16, v: bool) {
        if v {
            self.flags |= bit;
        } else {
            self.flags &= !bit;
        }
    }
    #[inline]
    fn flag(&self, bit: u16) -> bool {
        self.flags & bit != 0
    }
    #[inline]
    pub fn cf(&self) -> bool {
        self.flag(FLAG_CF)
    }

    /// Set flag aritmatika/logika untuk hasil 16-bit.
    #[allow(dead_code)]
    fn set_arith16(&mut self, result: u16, carry: bool, overflow: bool, input: u16, operand: u16) {
        self.r16_set(0, result); // GPR[0]=AX dipakai sementara oleh pemanggil
        let _ = (input, operand);
        self.set_flag(FLAG_CF, carry);
        self.set_flag(FLAG_OF, overflow);
        self.set_flag(FLAG_ZF, result == 0);
        self.set_flag(FLAG_SF, result & 0x8000 != 0);
        self.set_flag(FLAG_PF, (result as u8).count_ones() % 2 == 0);
    }

    /// Set flag hasil logika (CF/OF di-clear) 16-bit.
    #[allow(dead_code)]
    fn set_logic16(&mut self, result: u16) {
        self.set_flag(FLAG_CF, false);
        self.set_flag(FLAG_OF, false);
        self.set_flag(FLAG_ZF, result == 0);
        self.set_flag(FLAG_SF, result & 0x8000 != 0);
        self.set_flag(FLAG_PF, (result as u8).count_ones() % 2 == 0);
    }

    /// Set flag aritmatika 32-bit (prefix 66).
    #[allow(dead_code)]
    fn set_arith32(&mut self, result: u32, carry: bool, overflow: bool) {
        self.set_flag(FLAG_CF, carry);
        self.set_flag(FLAG_OF, overflow);
        self.set_flag(FLAG_ZF, result == 0);
        self.set_flag(FLAG_SF, result & 0x8000_0000 != 0);
        self.set_flag(FLAG_PF, (result as u8).count_ones() % 2 == 0);
    }

    // ── Memori real-mode ──
    /// Alamat linear: real mode = seg<<4 + offset (wrap 1MB, tanpa A20);
    /// protected mode = base GDT[selector] + offset (wrap 4GB).
    #[inline]
    fn lin(&self, seg: u16, off: u32) -> u64 {
        if self.pmode {
            let base = self.gdt_seg_base(seg);
            (base as u64 + off as u64) & 0xffff_ffff
        } else {
            (((seg as u64) << 4) + (off as u64)) & 0xf_ffff
        }
    }

    /// Base segmen dari GDT untuk sebuah selector (protected mode).
    /// Hanya dukungan GDT (TI=0, RPL dibuang). Di luar limit → 0.
    fn gdt_seg_base(&self, sel: u16) -> u32 {
        let idx = (sel >> 3) as usize;
        let off = idx * 8;
        if off + 7 >= self.gdt_cache.len() {
            return 0;
        }
        let g = &self.gdt_cache[off..off + 8];
        // base: byte 2-3 (bit 0-15), byte 4 (bit 16-23), byte 7 (bit 24-31)
        (g[2] as u32) | ((g[3] as u32) << 8) | ((g[4] as u32) << 16) | ((g[7] as u32) << 24)
    }

    fn read8(&self, mem: &mut dyn MemoryPort, seg: u16, off: u32) -> Result<u8, CpuFault> {
        let a = self.lin(seg, off);
        mem.read(a, 1)
            .map(|v| v as u8)
            .map_err(|e| self.fault(format!("read8 0x{:x}: {}", a, e)))
    }
    fn read16(&self, mem: &mut dyn MemoryPort, seg: u16, off: u32) -> Result<u16, CpuFault> {
        let a = self.lin(seg, off);
        let mut b = [0u8; 2];
        mem.read_exact(a, &mut b)
            .map_err(|e| self.fault(format!("read16 0x{:x}: {}", a, e)))?;
        Ok(u16::from_le_bytes(b))
    }
    fn read32(&self, mem: &mut dyn MemoryPort, seg: u16, off: u32) -> Result<u32, CpuFault> {
        let a = self.lin(seg, off);
        let mut b = [0u8; 4];
        mem.read_exact(a, &mut b)
            .map_err(|e| self.fault(format!("read32 0x{:x}: {}", a, e)))?;
        Ok(u32::from_le_bytes(b))
    }
    fn write8(&mut self, mem: &mut dyn MemoryPort, seg: u16, off: u32, v: u8) -> Result<(), CpuFault> {
        let a = self.lin(seg, off);
        if (VGA_TEXT_ADDR..VGA_TEXT_ADDR + VGA_TEXT_SIZE as u64).contains(&a) {
            self.vga[(a - VGA_TEXT_ADDR) as usize] = v;
        }
        mem.write(a, 1, v as u64)
            .map_err(|e| self.fault(format!("write8 0x{:x}: {}", a, e)))
    }
    fn write16(
        &mut self,
        mem: &mut dyn MemoryPort,
        seg: u16,
        off: u32,
        v: u16,
    ) -> Result<(), CpuFault> {
        let a = self.lin(seg, off);
        if (VGA_TEXT_ADDR..VGA_TEXT_ADDR + VGA_TEXT_SIZE as u64).contains(&a) {
            let o = (a - VGA_TEXT_ADDR) as usize;
            if o + 1 < VGA_TEXT_SIZE {
                self.vga[o] = (v & 0xff) as u8;
                self.vga[o + 1] = (v >> 8) as u8;
            }
        }
        mem.write_exact(a, &v.to_le_bytes())
            .map_err(|e| self.fault(format!("write16 0x{:x}: {}", a, e)))
    }
    fn write32(
        &mut self,
        mem: &mut dyn MemoryPort,
        seg: u16,
        off: u32,
        v: u32,
    ) -> Result<(), CpuFault> {
        let a = self.lin(seg, off);
        if (VGA_TEXT_ADDR..VGA_TEXT_ADDR + VGA_TEXT_SIZE as u64).contains(&a) {
            let o = (a - VGA_TEXT_ADDR) as usize;
            for i in 0..4 {
                let idx = o + i;
                if idx < VGA_TEXT_SIZE {
                    self.vga[idx] = ((v >> (8 * i)) & 0xff) as u8;
                }
            }
        }
        mem.write_exact(a, &v.to_le_bytes())
            .map_err(|e| self.fault(format!("write32 0x{:x}: {}", a, e)))
    }

    fn fetch8(&mut self, mem: &mut dyn MemoryPort) -> Result<u8, CpuFault> {
        // Prefetch cache: reload 16 byte bila lompatan / di luar buffer.
        if !self.pf_valid
            || self.pf_cs != self.cs
            || !(self.ip >= self.pf_base && self.ip < self.pf_base + self.pf_len as u32)
        {
            let a = self.lin(self.cs, self.ip as u32);
            if (0xbdc0..=0xbe30).contains(&a) && std::env::var("MARIA_X86_TRACE").is_ok() {
                eprintln!(
                    "FETCH @0x{a:x} ip={} cs=0x{:x} pmode={}",
                    self.ip, self.cs, self.pmode
                );
            }
            // Jangan baca melewati batas offset 16-bit (real mode).
            let cap = if self.pmode {
                16
            } else {
                (0x10000 - (self.ip as usize & 0xffff)).min(16)
            };
            let got = match mem.read_exact(a, &mut self.pf_data[..cap]) {
                Ok(()) => cap,
                Err(_) => {
                    // Region terpotong / keluar peta: baca 1 byte saja.
                    let b = mem
                        .read(a, 1)
                        .map_err(|e| self.fault(format!("fetch @0x{a:x}: {}", e)))? as u8;
                    self.pf_valid = true;
                    self.pf_cs = self.cs;
                    self.pf_base = self.ip;
                    self.pf_len = 1;
                    self.pf_data[0] = b;
                    let r = b;
                    self.ip = self.ip.wrapping_add(1);
                    return Ok(r);
                }
            };
            self.pf_valid = true;
            self.pf_cs = self.cs;
            self.pf_base = self.ip;
            self.pf_len = got;
        }
        let idx = (self.ip - self.pf_base) as usize;
        let b = self.pf_data[idx];
        self.ip = self.ip.wrapping_add(1);
        Ok(b)
    }
    fn fetch16(&mut self, mem: &mut dyn MemoryPort) -> Result<u16, CpuFault> {
        let lo = self.fetch8(mem)? as u16;
        let hi = self.fetch8(mem)? as u16;
        Ok(lo | (hi << 8))
    }
    fn fetch32(&mut self, mem: &mut dyn MemoryPort) -> Result<u32, CpuFault> {
        let b0 = self.fetch8(mem)? as u32;
        let b1 = self.fetch8(mem)? as u32;
        let b2 = self.fetch8(mem)? as u32;
        let b3 = self.fetch8(mem)? as u32;
        Ok(b0 | (b1 << 8) | (b2 << 16) | (b3 << 24))
    }

    fn fault(&self, reason: String) -> CpuFault {
        CpuFault {
            pc: self.pc(),
            reason,
        }
    }

    /// Push/pop stack. Real mode: SP 16-bit (wrap 64K); protected mode:
    /// ESP 32-bit penuh. BUG FIX: sebelumnya push32/pop32 memakai sp() (view
    /// 16-bit) → di pmode dgn ESP > 64K (mis. 0x7fff0 GRUB) ret/stack salah.
    fn push16(&mut self, mem: &mut dyn MemoryPort, v: u16) -> Result<(), CpuFault> {
        if self.pmode {
            self.gpr[4] = self.gpr[4].wrapping_sub(2);
            self.write16(mem, self.ss, self.gpr[4], v)
        } else {
            let sp = (self.gpr[4] as u16).wrapping_sub(2);
            self.gpr[4] = sp as u32;
            self.write16(mem, self.ss, sp as u32, v)
        }
    }
    fn pop16(&mut self, mem: &mut dyn MemoryPort) -> Result<u16, CpuFault> {
        if self.pmode {
            let v = self.read16(mem, self.ss, self.gpr[4])?;
            self.gpr[4] = self.gpr[4].wrapping_add(2);
            Ok(v)
        } else {
            let sp = self.gpr[4] as u16;
            let v = self.read16(mem, self.ss, sp as u32)?;
            self.gpr[4] = sp.wrapping_add(2) as u32;
            Ok(v)
        }
    }
    fn push32(&mut self, mem: &mut dyn MemoryPort, v: u32) -> Result<(), CpuFault> {
        if self.pmode {
            self.gpr[4] = self.gpr[4].wrapping_sub(4);
            self.write32(mem, self.ss, self.gpr[4], v)
        } else {
            let sp = (self.gpr[4] as u16).wrapping_sub(4);
            self.gpr[4] = sp as u32;
            self.write32(mem, self.ss, sp as u32, v)
        }
    }
    fn pop32(&mut self, mem: &mut dyn MemoryPort) -> Result<u32, CpuFault> {
        if self.pmode {
            let v = self.read32(mem, self.ss, self.gpr[4])?;
            self.gpr[4] = self.gpr[4].wrapping_add(4);
            Ok(v)
        } else {
            let sp = self.gpr[4] as u16;
            let v = self.read32(mem, self.ss, sp as u32)?;
            self.gpr[4] = sp.wrapping_add(4) as u32;
            Ok(v)
        }
    }

    /// Hentikan CPU (hlt / INT tak dikenal / jmp ke protected mode).
    fn halt(&mut self, reason: &str) {
        self.halted = true;
        self.halt_reason = reason.to_string();
    }
}

/// ModRM: mod(2) reg(3) rm(3).
#[derive(Debug, Clone, Copy)]
struct ModRm {
    m: u8,
    r: u8,
    rm: u8,
}

/// Alamat efektif 16-bit (real mode).
struct Ea {
    /// Alamat linear 20-bit.
    #[allow(dead_code)]
    lin: u64,
    /// Segment yang dipakai.
    seg: u16,
    /// Offset 16-bit (untuk lodsb/stosb dll yang pakai SI/DI).
    off: u32,
    /// mod==3 (register operand).
    is_reg: bool,
    reg: u8,
}

fn sign8(v: u8) -> i32 {
    (v as i8) as i32
}
fn sign16(v: u16) -> i32 {
    (v as i16) as i32
}
fn sign32(v: u32) -> i64 {
    (v as i32) as i64
}

impl X86Cpu {
    /// Decode ModRM + hitung alamat efektif. Real mode → 16-bit; protected
    /// mode → 32-bit (SIB + disp32).
    fn ea16(
        &mut self,
        mem: &mut dyn MemoryPort,
        modrm: ModRm,
        seg_ov: Option<u16>,
    ) -> Result<Ea, CpuFault> {
        if self.pmode {
            return self.ea32(mem, modrm, seg_ov);
        }
        if modrm.m == 3 {
            return Ok(Ea {
                lin: 0,
                seg: 0,
                off: 0,
                is_reg: true,
                reg: modrm.rm,
            });
        }
        // Segmen default: BP-based → SS; sisanya DS. PENTING: mod=0, rm=6
        // (`[disp16]` absolut) BUKAN berbasis BP → harus DS, bukan SS.
        let base_ss = match modrm.rm {
            2 | 3 => true,          // bp+si / bp+di → SS
            6 => modrm.m != 0,      // [bp+d8/d16] → SS; mod=0 = disp16 absolut → DS
            _ => false,
        };
        let default_seg = if base_ss { self.ss } else { self.ds };
        let seg = seg_ov.unwrap_or(default_seg);
        let mut off: u32;
        // GPR index: 0=ax 1=cx 2=dx 3=bx 4=sp 5=bp 6=si 7=di
        match modrm.rm {
            0 => off = self.r16(3) as u32 + self.r16(6) as u32, // bx+si
            1 => off = self.r16(3) as u32 + self.r16(7) as u32, // bx+di
            2 => off = self.r16(5) as u32 + self.r16(6) as u32, // bp+si
            3 => off = self.r16(5) as u32 + self.r16(7) as u32, // bp+di
            4 => off = self.r16(6) as u32,                      // si
            5 => off = self.r16(7) as u32,                      // di
            6 => {
                // [bp] — mod 0 berarti disp16 langsung, mod 1/2 = bp + disp
                if modrm.m == 0 {
                    off = self.fetch16(mem)? as u32;
                } else {
                    off = self.r16(5) as u32;
                }
            }
            7 => off = self.r16(3) as u32, // bx
            _ => unreachable!(),
        }
        if modrm.m == 1 {
            off = off.wrapping_add(sign8(self.fetch8(mem)?) as u32);
        } else if modrm.m == 2 {
            off = off.wrapping_add(sign16(self.fetch16(mem)?) as u32);
        }
        let lin = self.lin(seg, off & 0xffff);
        Ok(Ea {
            lin,
            seg,
            off: off & 0xffff,
            is_reg: false,
            reg: modrm.rm,
        })
    }

    /// Alamat efektif 32-bit (protected mode): modrm + SIB + disp8/32.
    fn ea32(
        &mut self,
        mem: &mut dyn MemoryPort,
        modrm: ModRm,
        seg_ov: Option<u16>,
    ) -> Result<Ea, CpuFault> {
        if modrm.m == 3 {
            return Ok(Ea {
                lin: 0,
                seg: 0,
                off: 0,
                is_reg: true,
                reg: modrm.rm,
            });
        }
        let rm = modrm.rm;
        let mut off: u32;
        let mut base_ss = false;
        let mut no_base = false;
        if rm == 4 {
            // SIB: scale(2) index(3) base(3)
            let sib = self.fetch8(mem)?;
            let scale = sib >> 6;
            let index = (sib >> 3) & 7;
            let base = sib & 7;
            let mut addr: u64 = 0;
            if index != 4 {
                addr += (self.r32(index as usize) as u64) << scale;
            }
            if base == 5 && modrm.m == 0 {
                no_base = true; // disp32 saja
            } else {
                addr += self.r32(base as usize) as u64;
                if base == 4 || base == 5 {
                    base_ss = true; // esp/ebp → SS
                }
            }
            off = addr as u32;
        } else {
            if modrm.m == 0 && rm == 5 {
                // mod=0, rm=5 non-SIB: alamat = disp32 SAJA, tanpa base register
                // (kasus klasik `mov ecx, [0x820c]` — jangan tambah EBP!).
                off = 0;
            } else {
                off = self.r32(rm as usize);
                if rm == 4 || rm == 5 {
                    base_ss = true; // esp/ebp → SS
                }
            }
        }
        let mut disp: u32 = 0;
        if modrm.m == 1 {
            disp = sign8(self.fetch8(mem)?) as u32;
        } else if modrm.m == 2 {
            disp = sign32(self.fetch32(mem)?) as u32;
        } else if modrm.m == 0 && (rm == 5 || (rm == 4 && no_base)) {
            disp = self.fetch32(mem)?;
        }
        off = off.wrapping_add(disp);
        let default_seg = if base_ss { self.ss } else { self.ds };
        let seg = seg_ov.unwrap_or(default_seg);
        let lin = self.lin(seg, off);
        Ok(Ea {
            lin,
            seg,
            off,
            is_reg: false,
            reg: rm,
        })
    }

    /// Baca operand ModRM (register bila mod=3, memori sebaliknya).
    /// PENTING: `wide=false` = operasi 8-bit SELALU, terlepas dari opsz —
    /// cek `!wide` dulu sebelum opsz (bug klasik: `mov [mem], al` di mode
    /// 32-bit ikut baca 32-bit).
    fn read_op(
        &self,
        mem: &mut dyn MemoryPort,
        ea: &Ea,
        opsz: u8,
        wide: bool,
    ) -> Result<u64, CpuFault> {
        if ea.is_reg {
            let i = ea.reg as usize;
            return Ok(if !wide {
                self.r8(i) as u64
            } else if opsz == 32 {
                self.r32(i) as u64
            } else {
                self.r16(i) as u64
            });
        }
        Ok(if !wide {
            self.read8(mem, ea.seg, ea.off)? as u64
        } else if opsz == 32 {
            self.read32(mem, ea.seg, ea.off)? as u64
        } else {
            self.read16(mem, ea.seg, ea.off)? as u64
        })
    }

    /// Tulis operand ModRM.
    fn write_op(
        &mut self,
        mem: &mut dyn MemoryPort,
        ea: &Ea,
        val: u64,
        opsz: u8,
        wide: bool,
    ) -> Result<(), CpuFault> {
        if ea.is_reg {
            let i = ea.reg as usize;
            if !wide {
                self.r8_set(i, val as u8);
            } else if opsz == 32 {
                self.r32_set(i, val as u32);
            } else {
                self.r16_set(i, val as u16);
            }
            return Ok(());
        }
        if !wide {
            self.write8(mem, ea.seg, ea.off, val as u8)
        } else if opsz == 32 {
            self.write32(mem, ea.seg, ea.off, val as u32)
        } else {
            self.write16(mem, ea.seg, ea.off, val as u16)
        }
    }
}

impl X86Cpu {
    /// Eksekusi satu instruksi (prefix dikonsumsi di loop).
    /// Default operand-size: 16 di real mode, 32 di protected mode (prefix 66
    /// membaliknya).
    pub fn exec_one(&mut self, mem: &mut dyn MemoryPort) -> Result<(), CpuFault> {
        let mut opsz = if self.pmode { 32u8 } else { 16u8 };
        let mut seg_ov: Option<u16> = None;
        let mut rep = false;
        loop {
            let op = self.fetch8(mem)?;
            match op {
                0x66 => {
                    opsz = if opsz == 16 { 32 } else { 16 };
                    continue;
                }
                0x67 => continue, // addr-size 16 di pmode: abaikan (offset 32 cukup)
                0x2e => {
                    seg_ov = Some(self.cs);
                    continue;
                }
                0x36 => {
                    seg_ov = Some(self.ss);
                    continue;
                }
                0x3e => {
                    seg_ov = Some(self.ds);
                    continue;
                }
                0x26 => {
                    seg_ov = Some(self.es);
                    continue;
                }
                0x64 => {
                    seg_ov = Some(self.fs);
                    continue;
                }
                0x65 => {
                    seg_ov = Some(self.gs);
                    continue;
                }
                0xf3 | 0xf2 => {
                    rep = true;
                    continue;
                }
                0xf0 => continue, // lock: abaikan
                _ => {
                    let a = self.lin(self.cs, self.ip as u32);
                    if (0xc7e0..=0xc7f8).contains(&a) && std::env::var("MARIA_X86_TRACE").is_ok() {
                        eprintln!(
                            "OP @0x{a:x} op=0x{op:02x} opsz={opsz} rep={rep} ip={} cs=0x{:x}",
                            self.ip, self.cs
                        );
                    }
                    self.exec_op(op, opsz, seg_ov, rep, mem)?;
                    return Ok(());
                }
            }
        }
    }

    fn exec_op(
        &mut self,
        op: u8,
        opsz: u8,
        seg_ov: Option<u16>,
        rep: bool,
        mem: &mut dyn MemoryPort,
    ) -> Result<(), CpuFault> {
        let _wide = opsz == 32 || (op & 1) == 1; // default lebar dari bit LSB opcode
        match op {
            // ── mov r/m, r8/16/32 (88/89) ──
            0x88 | 0x89 => {
                let mr = self.fetch8(mem)?;
                let m = ModRm {
                    m: mr >> 6,
                    r: (mr >> 3) & 7,
                    rm: mr & 7,
                };
                let ea = self.ea16(mem, m, seg_ov)?;
                let src = if op == 0x88 {
                    self.r8(m.r as usize) as u64
                } else if opsz == 32 {
                    self.r32(m.r as usize) as u64
                } else {
                    self.r16(m.r as usize) as u64
                };
                self.write_op(mem, &ea, src, opsz, op == 0x89)?;
            }
            // ── mov r8/16/32, r/m (8a/8b) ──
            0x8a | 0x8b => {
                let mr = self.fetch8(mem)?;
                let m = ModRm {
                    m: mr >> 6,
                    r: (mr >> 3) & 7,
                    rm: mr & 7,
                };
                let ea = self.ea16(mem, m, seg_ov)?;
                let v = self.read_op(mem, &ea, opsz, op == 0x8b)?;
                let r = m.r as usize;
                if op == 0x8a {
                    self.r8_set(r, v as u8);
                } else if opsz == 32 {
                    self.r32_set(r, v as u32);
                } else {
                    self.r16_set(r, v as u16);
                }
            }
            // ── mov r/m, imm (c6 /0, c7 /0) ──
            0xc6 | 0xc7 => {
                let mr = self.fetch8(mem)?;
                let m = ModRm {
                    m: mr >> 6,
                    r: (mr >> 3) & 7,
                    rm: mr & 7,
                };
                if m.r != 0 {
                    return Err(self.fault(format!("mov imm grup /{} tak didukung", m.r)));
                }
                let ea = self.ea16(mem, m, seg_ov)?;
                // c6 = mov r/m8,imm8 (selalu 8-bit); c7 = mov r/m16/32, imm (ikuti opsz).
                let v = if op == 0xc6 {
                    self.fetch8(mem)? as u64
                } else if opsz == 32 {
                    self.fetch32(mem)? as u64
                } else {
                    self.fetch16(mem)? as u64
                };
                self.write_op(mem, &ea, v, opsz, op == 0xc7)?;
            }
            // ── mov reg, imm (b0-bf) ──
            0xb0..=0xbf => {
                let r = (op & 7) as usize;
                if op >= 0xb8 {
                    // mov r16/32, imm
                    if opsz == 32 {
                        let v = self.fetch32(mem)?;
                        self.r32_set(r, v);
                    } else {
                        let v = self.fetch16(mem)?;
                        self.r16_set(r, v);
                    }
                } else if op >= 0xb0 {
                    let v = self.fetch8(mem)?;
                    self.r8_set(r, v);
                }
            }
            // ── mov sreg, r/m (8e) ──
            0x8e => {
                let mr = self.fetch8(mem)?;
                let m = ModRm {
                    m: mr >> 6,
                    r: (mr >> 3) & 7,
                    rm: mr & 7,
                };
                let ea = self.ea16(mem, m, seg_ov)?;
                let v = self.read_op(mem, &ea, 16, true)? as u16;
                match m.r {
                    0 => self.es = v,
                    1 => self.cs = v,
                    2 => self.ss = v,
                    3 => self.ds = v,
                    4 => self.fs = v,
                    5 => self.gs = v,
                    _ => return Err(self.fault("mov sreg invalid".into())),
                }
            }
            // ── mov r/m16, sreg (8c) ──
            0x8c => {
                let mr = self.fetch8(mem)?;
                let m = ModRm {
                    m: mr >> 6,
                    r: (mr >> 3) & 7,
                    rm: mr & 7,
                };
                let v = match m.r {
                    0 => self.es,
                    1 => self.cs,
                    2 => self.ss,
                    3 => self.ds,
                    4 => self.fs,
                    5 => self.gs,
                    _ => return Err(self.fault("mov sreg invalid".into())),
                } as u64;
                let ea = self.ea16(mem, m, seg_ov)?;
                self.write_op(mem, &ea, v, 16, true)?;
            }
            _ => self.exec_op2(op, opsz, seg_ov, rep, mem)?,
        }
        Ok(())
    }

    fn exec_op2(
        &mut self,
        op: u8,
        opsz: u8,
        seg_ov: Option<u16>,
        rep: bool,
        mem: &mut dyn MemoryPort,
    ) -> Result<(), CpuFault> {
        let _wide = opsz == 32 || (op & 1) == 1;
        match op {
            // ── prefix 0f (jcc rel32, movzx/movsx) — HARUS sebelum 0x00..=0x3f ──
            0x0f => self.exec_0f_group(opsz, mem)?,
            // ── push/pop seg (06/0e/16/1e push, 07/17/1f pop) — JANGAN sampai
            // tertangkap grup aritmatika 00-3f ──
            0x06 => self.push16(mem, self.es)?,
            0x0e => self.push16(mem, self.cs)?,
            0x16 => self.push16(mem, self.ss)?,
            0x1e => self.push16(mem, self.ds)?,
            0x07 => self.es = self.pop16(mem)?,
            0x17 => self.ss = self.pop16(mem)?,
            0x1f => self.ds = self.pop16(mem)?,
            // ── accumulator-immediate 8-bit (04/0c/14/1c/24/2c/34/3c) ──
            // SELALU AL, imm8 — ukuran operand 8-bit tidak berubah oleh opsz.
            0x04 | 0x0c | 0x14 | 0x1c | 0x24 | 0x2c | 0x34 | 0x3c => {
                let opc = (op >> 3) & 7;
                let imm = self.fetch8(mem)? as u64;
                let a = self.r8(0) as u64;
                let r = self.arith_apply(opc, a, imm, 8, false);
                if opc != 7 {
                    self.r8_set(0, r as u8);
                }
            }
            // ── accumulator-immediate 16/32-bit (05/0d/15/1d/25/2d/35/3d) ──
            0x05 | 0x0d | 0x15 | 0x1d | 0x25 | 0x2d | 0x35 | 0x3d => {
                let opc = (op >> 3) & 7;
                let imm = if opsz == 32 {
                    self.fetch32(mem)? as u64
                } else {
                    self.fetch16(mem)? as u64
                };
                let a = if opsz == 32 {
                    self.r32(0) as u64
                } else {
                    self.r16(0) as u64
                };
                let r = self.arith_apply(opc, a, imm, opsz, true);
                if opc != 7 {
                    if opsz == 32 {
                        self.r32_set(0, r as u32);
                    } else {
                        self.r16_set(0, r as u16);
                    }
                }
            }
            // ── aritmatika/logika grup ModRM (00-3f) ──
            0x00..=0x3f => self.exec_arith_group(op, opsz, seg_ov, mem)?,
            // ── inc/dec reg (40-4f) ──
            0x40..=0x47 => {
                let r = (op & 7) as usize;
                if opsz == 32 {
                    let v = self.r32(r).wrapping_add(1);
                    self.r32_set(r, v);
                    self.set_logic_flags(v as u64, 32);
                    self.set_flag(FLAG_OF, v == 0x8000_0000);
                } else {
                    let v = self.r16(r).wrapping_add(1);
                    self.r16_set(r, v);
                    self.set_logic_flags(v as u64, 16);
                    self.set_flag(FLAG_OF, v == 0x8000);
                }
            }
            0x48..=0x4f => {
                let r = (op & 7) as usize;
                if opsz == 32 {
                    let v = self.r32(r).wrapping_sub(1);
                    self.r32_set(r, v);
                    self.set_logic_flags(v as u64, 32);
                    self.set_flag(FLAG_OF, v == 0x7fff_ffff);
                } else {
                    let v = self.r16(r).wrapping_sub(1);
                    self.r16_set(r, v);
                    self.set_logic_flags(v as u64, 16);
                    self.set_flag(FLAG_OF, v == 0x7fff);
                }
            }
            // ── push/pop reg (50-5f) ──
            0x50..=0x57 => {
                let r = (op & 7) as usize;
                if opsz == 32 {
                    self.push32(mem, self.r32(r))?;
                } else {
                    self.push16(mem, self.r16(r))?;
                }
            }
            0x58..=0x5f => {
                let r = (op & 7) as usize;
                if opsz == 32 {
                    let v = self.pop32(mem)?;
                    self.r32_set(r, v);
                } else {
                    let v = self.pop16(mem)?;
                    self.r16_set(r, v);
                }
            }
            0x68 => {
                // push imm16 (real mode) / imm32 (pmode opsz 32)
                if opsz == 32 {
                    let v = self.fetch32(mem)?;
                    self.push32(mem, v)?;
                } else {
                    let v = self.fetch16(mem)?;
                    self.push16(mem, v)?;
                }
            }
            0x6a => {
                // push imm8 sign-extended; lebar push mengikuti opsz
                let v = sign8(self.fetch8(mem)?);
                if opsz == 32 {
                    self.push32(mem, v as i32 as u32)?;
                } else {
                    self.push16(mem, v as u16)?;
                }
            }
            // ── imul r, r/m, imm (69/6b) ──
            0x69 | 0x6b => {
                let mr = self.fetch8(mem)?;
                let m = ModRm {
                    m: mr >> 6,
                    r: (mr >> 3) & 7,
                    rm: mr & 7,
                };
                let ea = self.ea16(mem, m, seg_ov)?;
                let b = self.read_op(mem, &ea, opsz, true)?;
                let r = m.r as usize;
                if opsz == 32 {
                    let imm = if op == 0x69 {
                        self.fetch32(mem)? as i32 as i64
                    } else {
                        sign8(self.fetch8(mem)?) as i64
                    };
                    let rv = (b as i32 as i64).wrapping_mul(imm);
                    self.r32_set(r, rv as u32);
                    let cf = (rv as u32) & 0x8000_0000 != 0 && rv != (rv as i32 as i64);
                    self.set_flag(FLAG_CF, cf);
                    self.set_flag(FLAG_OF, cf);
                } else {
                    let imm = if op == 0x69 {
                        self.fetch16(mem)? as i16 as i64
                    } else {
                        sign8(self.fetch8(mem)?) as i64
                    };
                    let rv = (b as i16 as i64).wrapping_mul(imm);
                    self.r16_set(r, rv as u16);
                    let cf = (rv as u16) & 0x8000 != 0 && rv != (rv as i16 as i64);
                    self.set_flag(FLAG_CF, cf);
                    self.set_flag(FLAG_OF, cf);
                }
            }
            // ── pusha/popad (60/61) ──
            0x60 => {
                // pusha 16-bit / pushad 32-bit. SP di tengah (slot 5) sesuai
                // spec — bukan terakhir.
                if opsz == 32 {
                    let esp_save = self.gpr[4];
                    for r in [0, 1, 2, 3] {
                        self.push32(mem, self.gpr[r])?;
                    }
                    self.push32(mem, esp_save)?;
                    for r in [5, 6, 7] {
                        self.push32(mem, self.gpr[r])?;
                    }
                } else {
                    let sp_save = self.sp();
                    for r in [0, 1, 2, 3] {
                        self.push16(mem, self.r16(r))?;
                    }
                    self.push16(mem, sp_save)?;
                    for r in [5, 6, 7] {
                        self.push16(mem, self.r16(r))?;
                    }
                }
            }
            0x61 => {
                // popa 16-bit / popad 32-bit
                if opsz == 32 {
                    let edi = self.pop32(mem)?;
                    let esi = self.pop32(mem)?;
                    let ebp = self.pop32(mem)?;
                    let _ = self.pop32(mem)?; // esp
                    let ebx = self.pop32(mem)?;
                    let edx = self.pop32(mem)?;
                    let ecx = self.pop32(mem)?;
                    let eax = self.pop32(mem)?;
                    self.r32_set(7, edi);
                    self.r32_set(6, esi);
                    self.r32_set(5, ebp);
                    self.r32_set(3, ebx);
                    self.r32_set(2, edx);
                    self.r32_set(1, ecx);
                    self.r32_set(0, eax);
                } else {
                    let di = self.pop16(mem)?;
                    let si = self.pop16(mem)?;
                    let bp = self.pop16(mem)?;
                    let _ = self.pop16(mem)?; // sp
                    let bx = self.pop16(mem)?;
                    let dx = self.pop16(mem)?;
                    let cx = self.pop16(mem)?;
                    let ax = self.pop16(mem)?;
                    self.r16_set(7, di);
                    self.r16_set(6, si);
                    self.r16_set(5, bp);
                    self.r16_set(3, bx);
                    self.r16_set(2, dx);
                    self.r16_set(1, cx);
                    self.r16_set(0, ax);
                }
            }
            // ── jcc pendek (70-7f) ──
            0x70..=0x7f => {
                let disp = sign8(self.fetch8(mem)?);
                if self.jcc_taken(op & 0x0f) {
                    self.ip = (self.ip as i64 + disp as i64) as u32;
                }
            }
            // ── alu r/m, r grup (80-83) ──
            0x80..=0x83 => self.exec_alu_imm_group(op, opsz, seg_ov, mem)?,
            // ── test al/ax/eax, imm (a8/a9) ──
            0xa8 | 0xa9 => {
                let v = if op == 0xa8 {
                    self.r8(0) as u64
                } else if opsz == 32 {
                    self.r32(0) as u64
                } else {
                    self.r16(0) as u64
                };
                let imm = if op == 0xa8 {
                    self.fetch8(mem)? as u64
                } else if opsz == 32 {
                    self.fetch32(mem)? as u64
                } else {
                    self.fetch16(mem)? as u64
                };
                let r = v & imm;
                let bits = if op == 0xa8 {
                    8
                } else if opsz == 32 {
                    32
                } else {
                    16
                };
                self.set_logic_flags(r, bits);
                self.set_flag(FLAG_CF, false);
                self.set_flag(FLAG_OF, false);
            }
            // ── test r/m, r (84/85) ──
            0x84 | 0x85 => {
                let mr = self.fetch8(mem)?;
                let m = ModRm {
                    m: mr >> 6,
                    r: (mr >> 3) & 7,
                    rm: mr & 7,
                };
                let ea = self.ea16(mem, m, seg_ov)?;
                let a = self.read_op(mem, &ea, opsz, op == 0x85)?;
                let b = if op == 0x84 {
                    self.r8(m.r as usize) as u64
                } else if opsz == 32 {
                    self.r32(m.r as usize) as u64
                } else {
                    self.r16(m.r as usize) as u64
                };
                let r = a & b;
                let bits = if op == 0x84 {
                    8
                } else if opsz == 32 {
                    32
                } else {
                    16
                };
                self.set_logic_flags(r, bits);
            }
            // ── xchg (86/87) ──
            0x86 | 0x87 => {
                let mr = self.fetch8(mem)?;
                let m = ModRm {
                    m: mr >> 6,
                    r: (mr >> 3) & 7,
                    rm: mr & 7,
                };
                let ea = self.ea16(mem, m, seg_ov)?;
                let a = self.read_op(mem, &ea, opsz, op == 0x87)?;
                let b = if op == 0x86 {
                    self.r8(m.r as usize) as u64
                } else if opsz == 32 {
                    self.r32(m.r as usize) as u64
                } else {
                    self.r16(m.r as usize) as u64
                };
                self.write_op(mem, &ea, b, opsz, op == 0x87)?;
                if op == 0x86 {
                    self.r8_set(m.r as usize, a as u8);
                } else if opsz == 32 {
                    self.r32_set(m.r as usize, a as u32);
                } else {
                    self.r16_set(m.r as usize, a as u16);
                }
            }
            // ── nop (90) / xchg ax,reg (90-97) ──
            0x90 => {}
            0x91..=0x97 => {
                let r = (op & 7) as usize;
                let a = self.r16(0);
                self.r16_set(0, self.r16(r));
                self.r16_set(r, a);
            }
            // ── cbw/cwde (98), cwd/cdq (99) ──
            0x98 => {
                if opsz == 32 {
                    self.r32_set(0, (self.r16(0) as i16) as i32 as u32);
                } else {
                    self.r8h_set(0, if self.r8(0) & 0x80 != 0 { 0xff } else { 0 });
                }
            }
            0x99 => {
                if opsz == 32 {
                    // cdq: edx = sign(eax)
                    self.r32_set(
                        2,
                        if self.r32(0) & 0x8000_0000 != 0 {
                            0xffff_ffff
                        } else {
                            0
                        },
                    );
                } else {
                    self.r16_set(2, if self.r16(0) & 0x8000 != 0 { 0xffff } else { 0 });
                }
            }
            // ── mov moffs (a0-a3) — ukuran moffs mengikuti ADDRESS size,
            //    bukan operand size: real mode = moffs16 SELALU (prefix 66 hanya
            //    mengubah operand), protected mode = moffs32. ──
            0xa0 => {
                let off = if self.pmode {
                    self.fetch32(mem)?
                } else {
                    self.fetch16(mem)? as u32
                };
                let v = self.read8(mem, self.ds, off)?;
                self.r8_set(0, v);
            }
            0xa1 => {
                let off = if self.pmode {
                    self.fetch32(mem)?
                } else {
                    self.fetch16(mem)? as u32
                };
                if opsz == 32 {
                    let v = self.read32(mem, self.ds, off)?;
                    self.r32_set(0, v);
                } else {
                    let v = self.read16(mem, self.ds, off)?;
                    self.r16_set(0, v);
                }
            }
            0xa2 => {
                let off = if self.pmode {
                    self.fetch32(mem)?
                } else {
                    self.fetch16(mem)? as u32
                };
                self.write8(mem, self.ds, off, self.r8(0))?;
            }
            0xa3 => {
                let off = if self.pmode {
                    self.fetch32(mem)?
                } else {
                    self.fetch16(mem)? as u32
                };
                if opsz == 32 {
                    self.write32(mem, self.ds, off, self.r32(0))?;
                } else {
                    self.write16(mem, self.ds, off, self.r16(0))?;
                }
            }
            // ── pushf/pushfd (9c), popf/popfd (9d) ──
            0x9c => {
                if opsz == 32 {
                    self.push32(mem, self.flags as u32)?;
                } else {
                    self.push16(mem, self.flags)?;
                }
            }
             0x9d => {
                if opsz == 32 {
                    self.flags = self.pop32(mem)? as u16;
                } else {
                    self.flags = self.pop16(mem)?;
                }
            }
            // ── sahf (9e) / lahf (9f): transfer FLAGS ↔ AH ──
            0x9e => {
                // sahf: load SF/ZF/AF/PF/CF dari AH (bit 7/6/4/2/0).
                let ah = self.r8h(0);
                let keep = self.flags & !(FLAG_CF | FLAG_PF | FLAG_AF | FLAG_ZF | FLAG_SF);
                self.flags = keep
                    | if ah & 0x80 != 0 { FLAG_SF } else { 0 }
                    | if ah & 0x40 != 0 { FLAG_ZF } else { 0 }
                    | if ah & 0x10 != 0 { FLAG_AF } else { 0 }
                    | if ah & 0x04 != 0 { FLAG_PF } else { 0 }
                    | if ah & 0x01 != 0 { FLAG_CF } else { 0 };
            }
            0x9f => {
                // lahf: AH = SF ZF x AF x x PF x CF (bit1 selalu 1).
                let f = self.flags;
                let ah = (if f & FLAG_SF != 0 { 0x80 } else { 0 })
                    | (if f & FLAG_ZF != 0 { 0x40 } else { 0 })
                    | (if f & FLAG_AF != 0 { 0x10 } else { 0 })
                    | (if f & FLAG_PF != 0 { 0x04 } else { 0 })
                    | (if f & FLAG_CF != 0 { 0x01 } else { 0 })
                    | 0x02;
                self.r8h_set(0, ah as u8);
            }
            // ── mov string (a4-a7) ──
            0xa4 => self.movs(mem, 1, rep, seg_ov)?,
            0xa5 => self.movs(mem, if opsz == 32 { 4 } else { 2 }, rep, seg_ov)?,
            0xa6 => self.cmps(mem, 1, rep, seg_ov)?,
            0xa7 => self.cmps(mem, if opsz == 32 { 4 } else { 2 }, rep, seg_ov)?,
            // ── stos (aa/ab) ──
            0xaa => self.stos(mem, 1, rep, seg_ov)?,
            0xab => self.stos(mem, if opsz == 32 { 4 } else { 2 }, rep, seg_ov)?,
            // ── lods (ac/ad) ──
            0xac => self.lods(mem, 1)?,
            0xad => self.lods(mem, if opsz == 32 { 4 } else { 2 })?,
            // ── ret (c3), retf (cb), ret imm (c2/ca) — pop 16-bit real / 32-bit pmode ──
            0xc9 => {
                // leave: mov esp, ebp; pop ebp
                let new_esp = self.r32(5);
                self.r32_set(4, new_esp);
                let saved = self.pop32(mem)?;
                self.r32_set(5, saved);
            }
            0xc3 => {
                let popped = if opsz == 32 {
                    self.pop32(mem)?
                } else {
                    self.pop16(mem)? as u32
                };
                if (8_548_000..=8_626_000).contains(&self.steps)
                    && std::env::var("MARIA_X86_CALLS").is_ok()
                {
                    eprintln!(
                        "RET  step={} @0x{:08x} pop=0x{:08x} sp_after=0x{:08x} cs=0x{:x}",
                        self.steps,
                        self.pc(),
                        popped,
                        self.sp(),
                        self.cs
                    );
                }
                self.ip = popped;
            }
            0xc2 => {
                let n = if opsz == 32 {
                    self.fetch32(mem)?
                } else {
                    self.fetch16(mem)? as u32
                };
                self.ip = if opsz == 32 {
                    self.pop32(mem)?
                } else {
                    self.pop16(mem)? as u32
                };
                self.sp_set(self.sp().wrapping_add(n as u16));
            }
            0xcb => {
                if opsz == 32 {
                    self.ip = self.pop32(mem)?;
                } else {
                    self.ip = self.pop16(mem)? as u32;
                }
                self.cs = self.pop16(mem)?;
            }
            0xca => {
                let n = if opsz == 32 {
                    self.fetch32(mem)?
                } else {
                    self.fetch16(mem)? as u32
                };
                if opsz == 32 {
                    self.ip = self.pop32(mem)?;
                } else {
                    self.ip = self.pop16(mem)? as u32;
                }
                self.cs = self.pop16(mem)?;
                self.sp_set(self.sp().wrapping_add(n as u16));
            }
            // ── shifts grup (c0/c1 imm8, d0/d1 by 1, d2/d3 by cl) ──
            0xc0..=0xc1 | 0xd0..=0xd3 => self.exec_shift_group(op, opsz, seg_ov, mem)?,
            // ── in/out imm (e4-e7) ──
            // ── loop/loope/loopne (e0-e2) ──
            0xe0 | 0xe1 | 0xe2 => {
                let disp = sign8(self.fetch8(mem)?);
                let dec = if opsz == 32 {
                    let v = self.gpr[1].wrapping_sub(1);
                    self.gpr[1] = v;
                    v != 0
                } else {
                    let v = self.r16(1).wrapping_sub(1);
                    self.r16_set(1, v);
                    v != 0
                };
                let zf = self.flag(FLAG_ZF);
                let take = match op {
                    0xe0 => dec && !zf, // loopne
                    0xe1 => dec && zf,  // loope
                    _ => dec,           // loop
                };
                if take {
                    self.ip = (self.ip as i64 + disp as i64) as u32;
                }
            }
            0xe4 => {
                let _ = self.fetch8(mem)?;
                self.r8_set(0, 0); // port I/O: tidak ada hardware → 0
            }
            0xe5 => {
                let _ = self.fetch8(mem)?;
                self.r16_set(0, 0);
            }
            0xe6 | 0xe7 => {
                let _ = self.fetch8(mem)?;
            }
            // ── call near (e8) / jmp near (e9) / jmp far (ea) / jmp short (eb) ──
            0xe8 => {
                // call rel: rel16 real mode, rel32 dengan opsz 32 (prefix 66 / pmode)
                let disp = if opsz == 32 {
                    sign32(self.fetch32(mem)?)
                } else {
                    sign16(self.fetch16(mem)?) as i64
                };
                let ret = self.ip;
                if opsz == 32 {
                    self.push32(mem, self.ip)?;
                } else {
                    self.push16(mem, self.ip as u16)?;
                }
                if (8_548_000..=8_626_000).contains(&self.steps)
                    && std::env::var("MARIA_X86_CALLS").is_ok()
                {
                    eprintln!(
                        "CALL step={} @0x{:08x} ret=0x{:08x} target=0x{:08x} sp=0x{:08x}",
                        self.steps,
                        self.pc(),
                        ret,
                        (ret as i64 + disp) as u32,
                        self.sp()
                    );
                }
                self.ip = (self.ip as i64 + disp) as u32;
            }
            0xe9 => {
                let disp = if opsz == 32 {
                    sign32(self.fetch32(mem)?)
                } else {
                    sign16(self.fetch16(mem)?) as i64
                };
                self.ip = (self.ip as i64 + disp) as u32;
            }
            0xea => {
                // Far jump. 66 ea → off32 + sel16; ea → off16 + sel16.
                // Bila PE (CR0 bit 0) aktif: transisi ke protected mode 32-bit.
                if opsz == 32 {
                    let off = self.fetch32(mem)?;
                    let seg = self.fetch16(mem)?;
                    self.ip = off;
                    self.cs = seg;
                    // Mode operan di-flush oleh far jump: PE (CR0 bit 0) aktif →
                    // protected mode 32-bit; PE clear → real mode 16-bit
                    // (transisi prot→real via `mov cr0` + `jmp 0:off`, GRUB prot_to_real).
                    self.pmode = self.cr[0] & 1 != 0;
                } else {
                    let off = self.fetch16(mem)? as u32;
                    let seg = self.fetch16(mem)?;
                    // Far jump real mode (normalisasi CS, umum di boot code).
                    self.ip = off;
                    self.cs = seg;
                    self.pmode = self.cr[0] & 1 != 0;
                }
            }
            0xeb => {
                let disp = sign8(self.fetch8(mem)?);
                self.ip = (self.ip as i64 + disp as i64) as u32;
            }
            // ── in/out dx (ec-ef) ──
            0xec => self.r8_set(0, 0),
            0xed => self.r16_set(0, 0),
            0xee | 0xef => {}
            // ── cld/fd, cli/fb, sti, hlt, int ──
            0xfc => self.set_flag(FLAG_DF, false),
            0xfd => self.set_flag(FLAG_DF, true),
            0xfa => self.set_flag(FLAG_IF, false),
            0xfb => self.set_flag(FLAG_IF, true),
            0xf5 => self.set_flag(FLAG_CF, !self.flag(FLAG_CF)), // cmc
            0xf8 => self.set_flag(FLAG_CF, false),               // clc
            0xf9 => self.set_flag(FLAG_CF, true),                // stc
            0xf4 => self.halt("hlt"),
            0xf6 | 0xf7 => self.exec_f6_group(op, opsz, seg_ov, mem)?,
            // ── grup fe: inc/dec r/m8 ──
            0xfe => {
                let mr = self.fetch8(mem)?;
                let m = ModRm {
                    m: mr >> 6,
                    r: (mr >> 3) & 7,
                    rm: mr & 7,
                };
                if m.r > 1 {
                    self.halt(&format!("fe /{} belum didukung", m.r));
                    return Ok(());
                }
                let ea = self.ea16(mem, m, seg_ov)?;
                let v = self.read_op(mem, &ea, 8, false)?;
                let r = if m.r == 0 {
                    v.wrapping_add(1)
                } else {
                    v.wrapping_sub(1)
                };
                self.write_op(mem, &ea, r, 8, false)?;
                self.set_logic_flags(r, 8);
                // 8-bit INC/DEC OF: hasil == INT_MIN (INC) / INT_MAX (DEC).
                self.set_flag(FLAG_OF, if m.r == 0 { r == 0x80 } else { r == 0x7f });
            }
            // ── grup ff: inc/dec/call/jmp/push r/m ──
            0xff => self.exec_ff_group(op, opsz, seg_ov, mem)?,
            0xcd => {
                let n = self.fetch8(mem)?;
                self.int_dispatch(n, mem)?;
            }
            0xcc => self.int_dispatch(3, mem)?,
            // ── lea (8d) ──
            0x8d => {
                let mr = self.fetch8(mem)?;
                let m = ModRm {
                    m: mr >> 6,
                    r: (mr >> 3) & 7,
                    rm: mr & 7,
                };
                let ea = self.ea16(mem, m, seg_ov)?;
                let r = m.r as usize;
                if opsz == 32 {
                    self.r32_set(r, ea.off);
                } else {
                    self.r16_set(r, ea.off as u16);
                }
            }
            // ── pop r/m (8f /0) ──
            0x8f => {
                let mr = self.fetch8(mem)?;
                let m = ModRm {
                    m: mr >> 6,
                    r: (mr >> 3) & 7,
                    rm: mr & 7,
                };
                if m.r != 0 {
                    self.halt(&format!("8f /{} belum didukung", m.r));
                    return Ok(());
                }
                let ea = self.ea16(mem, m, seg_ov)?;
                let v = if opsz == 32 {
                    self.pop32(mem)?
                } else {
                    self.pop16(mem)? as u32
                };
                self.write_op(mem, &ea, v as u64, opsz, true)?;
            }
            // ── iret (cf) ──
            0xcf => {
                self.ip = self.pop16(mem)? as u32;
                self.cs = self.pop16(mem)?;
                self.flags = self.pop16(mem)?;
            }
            _ => {
                self.halt(&format!("opcode 0x{:02x} belum didukung", op));
            }
        }
        Ok(())
    }

    /// Aritmatika/logika grup 00-3f: ADD/OR/ADC/SBB/AND/SUB/XOR/CMP.
    /// Bit1 = direction (0 → dest r/m, 1 → dest reg), bit0 = width.
    fn exec_arith_group(
        &mut self,
        op: u8,
        opsz: u8,
        seg_ov: Option<u16>,
        mem: &mut dyn MemoryPort,
    ) -> Result<(), CpuFault> {
        // Bit0 opcode menentukan lebar: 0 = 8-bit SELALU, 1 = 16/32-bit
        // mengikuti opsz. Jangan tambahkan `opsz == 32` di sini.
        let wide = (op & 1) == 1;
        let mr = self.fetch8(mem)?;
        let m = ModRm {
            m: mr >> 6,
            r: (mr >> 3) & 7,
            rm: mr & 7,
        };
        let opc = (op >> 3) & 7; // 0=add 1=or 2=adc 3=sbb 4=and 5=sub 6=xor 7=cmp
        let dst_reg = ((op >> 1) & 1) == 1; // bit1: 1 → dest = reg
        let ea = self.ea16(mem, m, seg_ov)?;
        let ea_val = self.read_op(mem, &ea, opsz, wide)?;
        let reg_val = if !wide {
            self.r8(m.r as usize) as u64
        } else if opsz == 32 {
            self.r32(m.r as usize) as u64
        } else {
            self.r16(m.r as usize) as u64
        };
        // a = operand sisi dest, b = sumber
        let (a, b) = if dst_reg {
            (reg_val, ea_val)
        } else {
            (ea_val, reg_val)
        };
        let dst = if dst_reg { m.r as usize } else { m.rm as usize };
        let r = self.arith_apply(opc, a, b, opsz, wide);
        if opc != 7 {
            if dst_reg {
                if !wide {
                    self.r8_set(dst, r as u8);
                } else if opsz == 32 {
                    self.r32_set(dst, r as u32);
                } else {
                    self.r16_set(dst, r as u16);
                }
            } else {
                self.write_op(mem, &ea, r, opsz, wide)?;
            }
        }
        Ok(())
    }

    /// Terapkan operasi aritmatika + set flag. Kembalikan hasil.
    fn arith_apply(&mut self, opc: u8, a: u64, b: u64, opsz: u8, wide: bool) -> u64 {
        let bits = if !wide {
            8
        } else if opsz == 32 {
            32
        } else {
            16
        };
        let mask = if bits == 32 {
            0xffff_ffffu64
        } else if bits == 16 {
            0xffff
        } else {
            0xff
        };
        let sign_bit = if bits == 32 {
            0x8000_0000u64
        } else if bits == 16 {
            0x8000
        } else {
            0x80
        };
        match opc {
            0 => {
                // add
                let r = (a + b) & mask;
                self.set_flag(FLAG_CF, (a + b) > mask);
                self.set_flag(
                    FLAG_OF,
                    (a & sign_bit) == (b & sign_bit) && (r & sign_bit) != (a & sign_bit),
                );
                self.set_logic_flags(r, bits as u8);
                r
            }
            1 => {
                let r = (a | b) & mask;
                self.set_logic_flags(r, bits as u8);
                self.set_flag(FLAG_CF, false);
                self.set_flag(FLAG_OF, false);
                r
            }
            4 => {
                let r = (a & b) & mask;
                self.set_logic_flags(r, bits as u8);
                self.set_flag(FLAG_CF, false);
                self.set_flag(FLAG_OF, false);
                r
            }
            5 => {
                // sub
                let r = (a.wrapping_sub(b)) & mask;
                self.set_flag(FLAG_CF, a < b);
                self.set_flag(
                    FLAG_OF,
                    (a & sign_bit) != (b & sign_bit) && (r & sign_bit) != (a & sign_bit),
                );
                self.set_logic_flags(r, bits as u8);
                r
            }
            6 => {
                let r = (a ^ b) & mask;
                self.set_logic_flags(r, bits as u8);
                self.set_flag(FLAG_CF, false);
                self.set_flag(FLAG_OF, false);
                r
            }
            7 => {
                // cmp
                let r = (a.wrapping_sub(b)) & mask;
                self.set_flag(FLAG_CF, a < b);
                self.set_flag(
                    FLAG_OF,
                    (a & sign_bit) != (b & sign_bit) && (r & sign_bit) != (a & sign_bit),
                );
                self.set_logic_flags(r, bits as u8);
                r
            }
            2 => {
                // adc: a + b + CF
                let cin = self.cf() as u64;
                let full = a + b + cin;
                let r = full & mask;
                self.set_flag(FLAG_CF, full > mask);
                self.set_flag(
                    FLAG_OF,
                    (a & sign_bit) == (b & sign_bit) && (r & sign_bit) != (a & sign_bit),
                );
                self.set_logic_flags(r, bits as u8);
                r
            }
            3 => {
                // sbb: a - b - CF
                let cin = self.cf() as u64;
                let r = a.wrapping_sub(b).wrapping_sub(cin) & mask;
                self.set_flag(FLAG_CF, a < b + cin);
                self.set_flag(
                    FLAG_OF,
                    (a & sign_bit) != (b & sign_bit) && (r & sign_bit) != (a & sign_bit),
                );
                self.set_logic_flags(r, bits as u8);
                r
            }
            _ => 0,
        }
    }

    /// Set ZF/SF/PF untuk hasil (CF/OF tidak disentuh — pemanggil atur).
    fn set_logic_flags(&mut self, r: u64, bits: u8) {
        let mask = if bits == 32 {
            0xffff_ffffu64
        } else if bits == 16 {
            0xffff
        } else {
            0xff
        };
        let sign_bit = if bits == 32 {
            0x8000_0000u64
        } else if bits == 16 {
            0x8000
        } else {
            0x80
        };
        self.set_flag(FLAG_ZF, (r & mask) == 0);
        self.set_flag(FLAG_SF, r & sign_bit != 0);
        self.set_flag(FLAG_PF, (r as u8).count_ones() % 2 == 0);
    }

    /// ALU r/m, imm grup 80-83: /0 add /1 or /4 and /5 sub /7 cmp (+/6 xor)
    fn exec_alu_imm_group(
        &mut self,
        op: u8,
        opsz: u8,
        seg_ov: Option<u16>,
        mem: &mut dyn MemoryPort,
    ) -> Result<(), CpuFault> {
        // 0x80 = ALWAYS 8-bit (imm8), 0x81 = 16/32-bit (ikuti opsz),
        // 0x83 = 16/32-bit dengan imm8 sign-extended.
        let wide = op != 0x80;
        let mr = self.fetch8(mem)?;
        let m = ModRm {
            m: mr >> 6,
            r: (mr >> 3) & 7,
            rm: mr & 7,
        };
        let ea = self.ea16(mem, m, seg_ov)?;
        let a = self.read_op(mem, &ea, opsz, wide)?;
        let imm = if op == 0x80 {
            self.fetch8(mem)? as u64
        } else if op == 0x83 {
            sign8(self.fetch8(mem)?) as u64
        } else if opsz == 32 {
            self.fetch32(mem)? as u64
        } else {
            self.fetch16(mem)? as u64
        };
        let r = self.arith_apply(m.r, a, imm, opsz, wide);
        if m.r != 7 {
            self.write_op(mem, &ea, r, opsz, wide)?;
        }
        Ok(())
    }

    /// Shifts/rotates grup d0-d3: /4 shl /5 shr /6 sal /7 sar (+ /0 rol dll).
    /// Bit0 opcode: d0/d2 = 8-bit SELALU, d1/d3 = 16/32-bit (ikuti opsz).
    fn exec_shift_group(
        &mut self,
        op: u8,
        opsz: u8,
        seg_ov: Option<u16>,
        mem: &mut dyn MemoryPort,
    ) -> Result<(), CpuFault> {
        let wide = (op & 1) == 1;
        let mr = self.fetch8(mem)?;
        let m = ModRm {
            m: mr >> 6,
            r: (mr >> 3) & 7,
            rm: mr & 7,
        };
        let ea = self.ea16(mem, m, seg_ov)?;
        let a = self.read_op(mem, &ea, opsz, wide)?;
        let cnt = if op >= 0xd2 {
            self.r8(1) as u32 // shift by cl
        } else if op >= 0xd0 {
            1 // shift by 1
        } else {
            (self.fetch8(mem)? & 0x1f) as u32 // c0/c1: shift by imm8
        };
        let bits = if !wide {
            8u32
        } else if opsz == 32 {
            32
        } else {
            16
        };
        let mask = if bits == 32 {
            0xffff_ffffu64
        } else if bits == 16 {
            0xffff
        } else {
            0xff
        };
        let sign_bit = if bits == 32 {
            0x8000_0000u64
        } else if bits == 16 {
            0x8000
        } else {
            0x80
        };
        let r = match m.r {
            4 | 6 => {
                let r = (a << cnt) & mask;
                if cnt > 0 {
                    self.set_flag(FLAG_CF, (a >> (bits - cnt.min(bits))) & 1 != 0);
                }
                r
            }
            5 => {
                let r = (a >> cnt) & mask;
                if cnt > 0 {
                    self.set_flag(FLAG_CF, (a >> (cnt - 1)) & 1 != 0);
                }
                r
            }
            7 => {
                // sar: shift kanan aritmatika
                let sign = if a & sign_bit != 0 { mask } else { 0 };
                let r = ((a >> cnt) | (sign & !(mask >> cnt))) & mask;
                if cnt > 0 {
                    self.set_flag(FLAG_CF, (a >> (cnt - 1)) & 1 != 0);
                }
                r
            }
            0 => {
                // rol: putar kiri (bit yang keluar masuk dari kanan).
                if cnt == 0 {
                    a
                } else {
                    let c = cnt % bits;
                    let out = (a >> (bits - c)) & 1;
                    let r = ((a << c) | (a >> (bits - c))) & mask;
                    self.set_flag(FLAG_CF, out != 0);
                    r
                }
            }
            1 => {
                // ror: putar kanan (bit yang keluar masuk dari kiri).
                if cnt == 0 {
                    a
                } else {
                    let c = cnt % bits;
                    let out = (a >> (c - 1)) & 1;
                    let r = ((a >> c) | (a << (bits - c))) & mask;
                    self.set_flag(FLAG_CF, out != 0);
                    r
                }
            }
            2 => {
                // rcl: putar kiri melalui carry.
                if cnt == 0 {
                    a
                } else {
                    let c = cnt % (bits + 1);
                    let mut v = a;
                    for _ in 0..c {
                        let new_cf = (v >> (bits - 1)) & 1;
                        v = ((v << 1) | self.flag(FLAG_CF) as u64) & mask;
                        self.set_flag(FLAG_CF, new_cf != 0);
                    }
                    v
                }
            }
            3 => {
                // rcr: putar kanan melalui carry.
                if cnt == 0 {
                    a
                } else {
                    let c = cnt % (bits + 1);
                    let mut v = a;
                    for _ in 0..c {
                        let new_cf = v & 1;
                        v = ((v >> 1) | ((self.flag(FLAG_CF) as u64) << (bits - 1))) & mask;
                        self.set_flag(FLAG_CF, new_cf != 0);
                    }
                    v
                }
            }
            _ => a,
        };
        self.set_logic_flags(r, bits as u8);
        self.write_op(mem, &ea, r, opsz, wide)?;
        Ok(())
    }

    /// Grup f6/f7: /0 test /2 not /3 neg /4 mul /6 div (substet).
    /// f6 = SELALU 8-bit, f7 = 16/32-bit (ikuti opsz).
    fn exec_f6_group(
        &mut self,
        op: u8,
        opsz: u8,
        seg_ov: Option<u16>,
        mem: &mut dyn MemoryPort,
    ) -> Result<(), CpuFault> {
        let wide = op == 0xf7;
        let mr = self.fetch8(mem)?;
        let m = ModRm {
            m: mr >> 6,
            r: (mr >> 3) & 7,
            rm: mr & 7,
        };
        let ea = self.ea16(mem, m, seg_ov)?;
        let a = self.read_op(mem, &ea, opsz, wide)?;
        match m.r {
            0 => {
                let imm = if op == 0xf7 {
                    if opsz == 32 {
                        self.fetch32(mem)? as u64
                    } else {
                        self.fetch16(mem)? as u64
                    }
                } else {
                    self.fetch8(mem)? as u64
                };
                let r = a & imm;
                self.set_logic_flags(
                    r,
                    if !wide {
                        8
                    } else if opsz == 32 {
                        32
                    } else {
                        16
                    },
                );
                self.set_flag(FLAG_CF, false);
                self.set_flag(FLAG_OF, false);
            }
            2 => {
                let r = !a;
                self.write_op(mem, &ea, r, opsz, wide)?;
            }
            3 => {
                // neg r/m: 0 - a (CF = a != 0)
                let bits = if !wide {
                    8u64
                } else if opsz == 32 {
                    32
                } else {
                    16
                };
                let mask = if bits == 32 {
                    0xffff_ffffu64
                } else if bits == 16 {
                    0xffff
                } else {
                    0xff
                };
                let r = (mask.wrapping_sub(a) + 1) & mask;
                self.write_op(mem, &ea, r, opsz, wide)?;
                self.set_logic_flags(r, bits as u8);
                self.set_flag(FLAG_CF, a != 0);
                self.set_flag(FLAG_OF, a == (1u64 << (bits - 1)));
            }
            4 => {
                // mul r/m: tanpa tanda. 8-bit: AX = AL*r8; 16-bit: DX:AX; 32-bit: EDX:EAX
                if !wide {
                    let prod = self.r8(0) as u16 * a as u16;
                    self.r8_set(0, prod as u8);
                    self.r8h_set(0, (prod >> 8) as u8);
                    self.set_flag(FLAG_CF, prod >> 8 != 0);
                    self.set_flag(FLAG_OF, self.flag(FLAG_CF));
                } else if opsz == 32 {
                    let prod = self.r32(0) as u64 * a;
                    self.r32_set(0, prod as u32);
                    self.r32_set(2, (prod >> 32) as u32);
                    self.set_flag(FLAG_CF, prod >> 32 != 0);
                    self.set_flag(FLAG_OF, self.flag(FLAG_CF));
                } else {
                    let prod = self.r16(0) as u32 * a as u32;
                    self.r16_set(0, prod as u16);
                    self.r16_set(2, (prod >> 16) as u16);
                    self.set_flag(FLAG_CF, prod >> 16 != 0);
                    self.set_flag(FLAG_OF, self.flag(FLAG_CF));
                }
            }
            5 => {
                // imul r/m: bertanda. 8-bit: AX = AL*r8; 16-bit: DX:AX; 32-bit: EDX:EAX
                if !wide {
                    let prod = (self.r8(0) as i8 as i64) * (a as i8 as i64);
                    self.r8_set(0, prod as u8);
                    self.r8h_set(0, (prod >> 8) as u8);
                    self.set_flag(FLAG_CF, prod != (prod as i8 as i64));
                    self.set_flag(FLAG_OF, self.flag(FLAG_CF));
                } else if opsz == 32 {
                    let prod = (self.r32(0) as i32 as i64) * (a as i32 as i64);
                    self.r32_set(0, prod as u32);
                    self.r32_set(2, (prod >> 32) as u32);
                    self.set_flag(FLAG_CF, prod != (prod as i32 as i64));
                    self.set_flag(FLAG_OF, self.flag(FLAG_CF));
                } else {
                    let prod = (self.r16(0) as i16 as i64) * (a as i16 as i64);
                    self.r16_set(0, prod as u16);
                    self.r16_set(2, (prod >> 16) as u16);
                    self.set_flag(FLAG_CF, prod != (prod as i16 as i64));
                    self.set_flag(FLAG_OF, self.flag(FLAG_CF));
                }
            }
            6 => {
                // div r/m: tanpa tanda. 8-bit: AX/r8; 16-bit: DX:AX/r16; 32-bit: EDX:EAX/r32
                if a == 0 {
                    self.halt("div by zero");
                    return Ok(());
                }
                if !wide {
                    let dividend = self.r16(0) as u64;
                    let q = dividend / a;
                    let rem = dividend % a;
                    if q > 0xff {
                        self.halt("div overflow");
                    } else {
                        self.r8_set(0, q as u8);
                        self.r8h_set(0, rem as u8);
                    }
                } else if opsz == 32 {
                    let dividend = ((self.r32(2) as u64) << 32) | self.r32(0) as u64;
                    let q = dividend / a;
                    let rem = dividend % a;
                    if q > 0xffff_ffff {
                        self.halt("div overflow");
                    } else {
                        self.r32_set(0, q as u32);
                        self.r32_set(2, rem as u32);
                    }
                } else {
                    let dividend = ((self.r16(2) as u64) << 16) | self.r16(0) as u64;
                    let q = dividend / a;
                    let rem = dividend % a;
                    if q > 0xffff {
                        self.halt("div overflow");
                    } else {
                        self.r16_set(0, q as u16);
                        self.r16_set(2, rem as u16);
                    }
                }
            }
            7 => {
                // idiv r/m: bertanda. 8-bit: AX/a; 16-bit: DX:AX/a; 32-bit: EDX:EAX/a.
                // Quotient → AX/EAX, sisa → AH/EDX (tanda mengikuti dividend).
                if a == 0 {
                    self.halt("idiv by zero");
                    return Ok(());
                }
                if !wide {
                    let divd = self.r16(0) as i16 as i32;
                    let d = a as i8 as i32;
                    let q = divd / d;
                    let rem = divd % d;
                    if !(-128..=127).contains(&q) {
                        self.halt("idiv overflow");
                    } else {
                        self.r8_set(0, q as u8);
                        self.r8h_set(0, rem as u8);
                    }
                } else if opsz == 32 {
                    // EDX:EAX 64-bit signed dividend.
                    let hi = self.r32(2) as u32;
                    let lo = self.r32(0) as u32;
                    let divd = (((hi as i64) << 32) | lo as i64) as i64 as i128;
                    let d = a as u32 as i32 as i128;
                    let q = divd / d;
                    let rem = divd % d;
                    if divd != 0 && (q > i64::MAX as i128 || q < i64::MIN as i128) {
                        self.halt("idiv overflow");
                    } else {
                        self.r32_set(0, q as u32);
                        self.r32_set(2, rem as u32);
                    }
                } else {
                    // DX:AX 32-bit signed dividend.
                    let hi = self.r16(2) as u16;
                    let lo = self.r16(0) as u16;
                    let divd = (((hi as i32) << 16) | lo as i32) as i64;
                    let d = a as u16 as i16 as i64;
                    let q = divd / d;
                    let rem = divd % d;
                    if !(-32768..=32767).contains(&q) {
                        self.halt("idiv overflow");
                    } else {
                        self.r16_set(0, q as u16);
                        self.r16_set(2, rem as u16);
                    }
                }
            }
            _ => self.halt(&format!("f6/f7 /{} belum didukung", m.r)),
        }
        Ok(())
    }

    /// Grup ff: /0 inc r/m, /1 dec r/m, /2 call near, /4 jmp near,
    /// /5 jmp far [mem], /6 push r/m.
    fn exec_ff_group(
        &mut self,
        op: u8,
        opsz: u8,
        seg_ov: Option<u16>,
        mem: &mut dyn MemoryPort,
    ) -> Result<(), CpuFault> {
        let _wide = opsz == 32;
        let mr = self.fetch8(mem)?;
        let m = ModRm {
            m: mr >> 6,
            r: (mr >> 3) & 7,
            rm: mr & 7,
        };
        let ea = self.ea16(mem, m, seg_ov)?;
        match m.r {
            0 | 1 => {
                let v = self.read_op(mem, &ea, opsz, true)?;
                let bits = if opsz == 32 { 32u32 } else { 16 };
                let r = if m.r == 0 {
                    v.wrapping_add(1)
                } else {
                    v.wrapping_sub(1)
                };
                self.write_op(mem, &ea, r, opsz, true)?;
                self.set_logic_flags(r, bits as u8);
                let sign_bit = if opsz == 32 { 0x8000_0000u64 } else { 0x8000 };
                // OF konsisten dgn grup inc/dec reg (40-4f):
                //   INC OF = hasil == INT_MIN (0x8000..) — operand +max → −min
                //   DEC OF = hasil == INT_MAX (0x7fff..) — operand −min → +max
                let ov = if m.r == 0 {
                    r == sign_bit
                } else {
                    r == sign_bit.wrapping_sub(1)
                };
                self.set_flag(FLAG_OF, ov);
            }
            2 => {
                let t = self.read_op(mem, &ea, opsz, true)?;
                if opsz == 32 {
                    self.push32(mem, self.ip)?;
                } else {
                    self.push16(mem, self.ip as u16)?;
                }
                self.ip = t as u32;
            }
            4 => {
                let t = self.read_op(mem, &ea, opsz, true)?;
                self.ip = t as u32;
            }
            5 => {
                let off = self.read16(mem, ea.seg, ea.off)?;
                let seg = self.read16(mem, ea.seg, ea.off + 2)?;
                self.ip = off as u32;
                self.cs = seg;
                self.pmode = self.cr[0] & 1 != 0;
            }
            6 => {
                // push r/m — ukuran mengikuti opsz (16-bit real, 32-bit pmode).
                let v = self.read_op(mem, &ea, opsz, true)?;
                if opsz == 32 {
                    self.push32(mem, v as u32)?;
                } else {
                    self.push16(mem, v as u16)?;
                }
            }
            _ => self.halt(&format!("ff /{} belum didukung", m.r)),
        }
        let _ = op;
        Ok(())
    }

    /// Prefix 0f: jcc rel16/32 (80-8f), movzx (b6/b7), movsx (be/bf).
    /// Displacement jcc mengikuti operand-size: rel16 di mode 16-bit,
    /// rel32 dengan prefix 66 — PENTING untuk boot code GRUB/ISOLINUX.
    /// SHLD/SHRD (double-precision shift): 0F A4/A5 shld, 0F AC/AD shrd.
    /// dest = hasil geser dest, dengan bit yang "masuk" berasal dari src.
    fn exec_double_shift(
        &mut self,
        op: u8,
        opsz: u8,
        mem: &mut dyn MemoryPort,
    ) -> Result<(), CpuFault> {
        let mr = self.fetch8(mem)?;
        let m = ModRm {
            m: mr >> 6,
            r: (mr >> 3) & 7,
            rm: mr & 7,
        };
        let is_imm = op == 0xa4 || op == 0xac;
        let is_shrd = op == 0xac || op == 0xad;
        let count = if is_imm {
            self.fetch8(mem)? as u32
        } else {
            self.r8(1) as u32 // CL
        };
        let (bits, mask): (u32, u32) = if opsz == 32 { (32, 31) } else { (16, 15) };
        let x = count & mask;
        let ea = self.ea16(mem, m, None)?;
        let d = self.read_op(mem, &ea, opsz, true)? & ((1u64 << bits) - 1);
        let s = if opsz == 32 {
            self.r32(m.r as usize) as u64
        } else {
            self.r16(m.r as usize) as u64
        };
        let r: u64 = if x == 0 {
            // count&mask == 0 → tidak ada pergeseran; CF/OF dibiarkan.
            d
        } else if is_shrd {
            (d >> x) | ((s << (bits - x)) & ((1u64 << bits) - 1))
        } else {
            ((d << x) & ((1u64 << bits) - 1)) | (s >> (bits - x))
        };
        if x > 0 {
            // CF = bit terakhir yang keluar.
            let out = if is_shrd { x - 1 } else { bits - x };
            self.set_flag(FLAG_CF, (d >> out) & 1 != 0);
            // OF terdefinisi hanya saat x==1 (Intel): MSB hasil XOR MSB dest.
            if x == 1 {
                let msb = bits - 1;
                self.set_flag(FLAG_OF, ((r >> msb) & 1) != ((d >> msb) & 1));
            }
        }
        self.set_logic_flags(r, bits as u8);
        self.write_op(mem, &ea, r, opsz, true)?;
        Ok(())
    }

    fn exec_0f_group(&mut self, opsz: u8, mem: &mut dyn MemoryPort) -> Result<(), CpuFault> {
        let op = self.fetch8(mem)?;
        match op {
            0x80..=0x8f => {
                let disp = if opsz == 32 {
                    sign32(self.fetch32(mem)?)
                } else {
                    sign16(self.fetch16(mem)?) as i64
                };
                if self.jcc_taken(op & 0x0f) {
                    self.ip = (self.ip as i64 + disp) as u32;
                }
            }
            0x90..=0x9f => {
                // setcc r/m8
                let mr = self.fetch8(mem)?;
                let m = ModRm {
                    m: mr >> 6,
                    r: (mr >> 3) & 7,
                    rm: mr & 7,
                };
                let v = self.jcc_taken(op & 0x0f) as u8;
                if m.m == 3 {
                    self.r8_set(m.rm as usize, v);
                } else {
                    let ea = self.ea16(mem, m, None)?;
                    self.write8(mem, ea.seg, ea.off, v)?;
                }
            }
            0xaf => {
                // imul r16/32, r/m16/32
                let mr = self.fetch8(mem)?;
                let m = ModRm {
                    m: mr >> 6,
                    r: (mr >> 3) & 7,
                    rm: mr & 7,
                };
                let ea = self.ea16(mem, m, None)?;
                let b = self.read_op(mem, &ea, opsz, true)?;
                let r = m.r as usize;
                if opsz == 32 {
                    let a = self.r32(r) as i32 as i64;
                    let prod = a.wrapping_mul(b as i32 as i64);
                    self.r32_set(r, prod as u32);
                    let sign = (prod as u32) & 0x8000_0000 != 0;
                    self.set_flag(FLAG_CF, sign && prod != (prod as i32 as i64));
                    self.set_flag(FLAG_OF, self.flag(FLAG_CF));
                } else {
                    let a = self.r16(r) as i16 as i64;
                    let prod = a.wrapping_mul(b as i16 as i64);
                    self.r16_set(r, prod as u16);
                    let sign = (prod as u16) & 0x8000 != 0;
                    self.set_flag(FLAG_CF, sign && prod != (prod as i16 as i64));
                    self.set_flag(FLAG_OF, self.flag(FLAG_CF));
                }
            }
            0xb6 | 0xb7 => {
                // movzx r16/32, r/m8/16
                let mr = self.fetch8(mem)?;
                let m = ModRm {
                    m: mr >> 6,
                    r: (mr >> 3) & 7,
                    rm: mr & 7,
                };
                let ea = self.ea16(mem, m, None)?;
                let v = self.read_op(mem, &ea, 16, op == 0xb7)?;
                let r = m.r as usize;
                if opsz == 32 {
                    self.r32_set(r, v as u32);
                } else {
                    self.r16_set(r, v as u16);
                }
            }
            0xbe | 0xbf => {
                // movsx r16/32, r/m8/16
                let mr = self.fetch8(mem)?;
                let m = ModRm {
                    m: mr >> 6,
                    r: (mr >> 3) & 7,
                    rm: mr & 7,
                };
                let ea = self.ea16(mem, m, None)?;
                let v = self.read_op(mem, &ea, 16, op == 0xbf)?;
                let r = m.r as usize;
                let signed: i64 = if op == 0xbf {
                    sign16(v as u16) as i64
                } else {
                    sign8(v as u8) as i64
                };
                if opsz == 32 {
                    self.r32_set(r, signed as u32);
                } else {
                    self.r16_set(r, signed as u16);
                }
            }
            // ── grup sistem 0f 01: sgdt/sidt/lgdt/lidt/smsw/lmsw/invlpg ──
            0x01 => {
                let mr = self.fetch8(mem)?;
                let m = ModRm {
                    m: mr >> 6,
                    r: (mr >> 3) & 7,
                    rm: mr & 7,
                };
                if m.m == 3 {
                    // mod=11: smsw r/m16 (reg 4), lmsw r/m16 (reg 6)
                    match m.r {
                        4 => {
                            // smsw r/m16
                            let v = self.cr[0] as u16;
                            if m.rm == 0 {
                                self.r16_set(0, v);
                            } else {
                                let ea = Ea {
                                    lin: 0,
                                    seg: 0,
                                    off: 0,
                                    is_reg: true,
                                    reg: m.rm,
                                };
                                self.write_op(mem, &ea, v as u64, 16, true)?;
                            }
                        }
                        6 => {
                            // lmsw r/m16: muat CR0 bit 0-15
                            let v = if m.rm == 0 {
                                self.r16(0)
                            } else {
                                let ea = Ea {
                                    lin: 0,
                                    seg: 0,
                                    off: 0,
                                    is_reg: true,
                                    reg: m.rm,
                                };
                                self.read_op(mem, &ea, 16, true)? as u16
                            };
                            self.cr[0] = (self.cr[0] & !0xffff) | v as u32;
                        }
                        _ => return Err(self.fault(format!("0f 01 /{} mod11 tak didukung", m.r))),
                    }
                } else {
                    // mod != 11: sgdt/sidt (store), lgdt/lidt (load), invlpg (ignore)
                    let ea = self.ea16(mem, m, None)?;
                    match m.r {
                        0 => {
                            // sgdt m: limit16 + base32
                            self.write16(mem, ea.seg, ea.off, self.gdt_limit)?;
                            self.write32(mem, ea.seg, ea.off + 2, self.gdt_base)?;
                        }
                        1 => {
                            // sidt m (IDT tak di-track → limit 0)
                            self.write16(mem, ea.seg, ea.off, 0)?;
                            self.write32(mem, ea.seg, ea.off + 2, 0)?;
                        }
                        2 => {
                            // lgdt m: baca limit16 + base32, cache byte GDT DARI BASE
                            // (deskriptor memberi base; GDT berada DI base, bukan di
                            // lokasi deskriptor — bug lama: cache salah → base segmen
                            // garbage → stack salah di protected mode).
                            let limit = self.read16(mem, ea.seg, ea.off)?;
                            let base = self.read32(mem, ea.seg, ea.off + 2)?;
                            self.gdt_limit = limit;
                            self.gdt_base = base;
                            let mut cache = Vec::new();
                            for i in 0..=(limit as usize).min(8191) {
                                let b = mem.read(base as u64 + i as u64, 1).unwrap_or(0) as u8;
                                cache.push(b);
                            }
                            self.gdt_cache = cache;
                        }
                        3 => {
                            // lidt m: IDT tak dipakai boot chain → baca & buang
                            let _limit = self.read16(mem, ea.seg, ea.off)?;
                            let _base = self.read32(mem, ea.seg, ea.off + 2)?;
                        }
                        7 => {
                            // invlpg m: no-op (cache tak disimulasikan)
                            let _ = ea;
                        }
                        _ => {
                            return Err(
                                self.fault(format!("0f 01 /{} mod{} tak didukung", m.r, m.m))
                            )
                        }
                    }
                }
            }
            // ── mov r32, crN (0f 20) / mov crN, r32 (0f 22) ──
            0x20 | 0x22 => {
                let mr = self.fetch8(mem)?;
                let m = ModRm {
                    m: mr >> 6,
                    r: (mr >> 3) & 7,
                    rm: mr & 7,
                };
                if m.r > 7 {
                    return Err(self.fault(format!("mov cr reg invalid {}", m.r)));
                }
                if op == 0x20 {
                    // mov r32, crN
                    let v = self.cr[m.r as usize];
                    self.r32_set(m.rm as usize, v);
                } else {
                    // mov crN, r32
                    let v = self.r32(m.rm as usize);
                    self.cr[m.r as usize] = v;
                }
            }
            // ── clts (0f 06): clear CR0.TS ──
            0x06 => {
                self.cr[0] &= !(1 << 3);
            }
            // ── cpuid (0f a2): CPU identification — GRUB/Linux membutuhkan ──
            // EAX = leaf: 0 → vendor string, 1 → features/stepping, 0x80000001 → extended.
            0xa2 => {
                let leaf = self.r32(0);
                match leaf {
                    0 => {
                        // Vendor: "GenuineIntel" (EBX 'uneG', EDX 'ineI', ECX 'ntel')
                        self.r32_set(1, 0x756e6547); // EBX: 'uneG'
                        self.r32_set(3, 0x49656e69); // EDX: 'ineI'
                        self.r32_set(2, 0x6c65746e); // ECX: 'ntel'
                    }
                    1 => {
                        // Family 6, Model 0x1A (Nehalem), Stepping 0
                        // EAX: [31:28] ext_family=0, [27:20]=0, [19:16] family=6,
                        //       [15:12] ext_model=0, [11:8] model=0x1A, [7:0] stepping=0
                        self.r32_set(0, 0x000_06_1A_00);
                        // EBX: brand index=0, cache line=0, max apic=0, logical proc=0
                        self.r32_set(3, 0);
                        // ECX: feature bits (SSE3=0, SSSE3=0, CX16=0, POPCNT=0)
                        // GRUB/Linux hanya cek少量 feature — semua 0 cukup untuk boot
                        self.r32_set(2, 0);
                        // EDX: feature bits — set FPU(0), TSC(4), CX8(8), SSE(25),
                        // SSE2(26) agar GRUB/LINUX tidak reject CPU
                        self.r32_set(3, (1 << 0) | (1 << 4) | (1 << 8) | (1 << 25) | (1 << 26));
                    }
                    0x8000_0001 => {
                        // Extended: set Long Mode bit (29) untuk 64-bit support
                        // EDX: LM(29), NX(20) — Linux perlu LM untuk 64-bit
                        self.r32_set(3, (1 << 20) | (1 << 29));
                        self.r32_set(2, 0); // ECX extended features = 0
                    }
                    0x8000_0002..=0x8000_0004 => {
                        // Processor name string (48 bytes dari 3 leaf)
                        // "Maria Virtual CPU    " (32 chars per 4 regs = 16 bytes)
                        let name_parts: [[u32; 4]; 3] = [
                            // leaf 2: "Maria Virtu"
                            [0x7261694D, 0x20616C65, 0x75726956, 0x00206C61],
                            // leaf 3: "al CPU     "
                            [0x20204C50, 0x20202055, 0x20202020, 0x20202020],
                            // leaf 4: (padding)
                            [0x20202020, 0x20202020, 0x20202020, 0x00202020],
                        ];
                        let idx = (leaf - 0x8000_0002) as usize;
                        if idx < 3 {
                            for (i, &v) in name_parts[idx].iter().enumerate() {
                                self.r32_set(i, v);
                            }
                        }
                    }
                    _ => {
                        // Unknown CPUID leaf: return zeros (BIOS/Linux toleransi)
                        self.r32_set(0, 0);
                        self.r32_set(1, 0);
                        self.r32_set(2, 0);
                        self.r32_set(3, 0);
                    }
                }
            }
            // ── rdtsc (0f 31): read timestamp counter → EDX:EAX ──
            // Return cycle count monotonik — GRUB/Linux pakai untuk timing.
            0x31 => {
                // Timestamp berdasarkan instruction count (estimasi)
                let ts = self.steps.wrapping_mul(1);
                self.r32_set(0, ts as u32);          // EAX low
                self.r32_set(2, (ts >> 32) as u32);   // EDX high
            }
            // ── bswap r32 (0f c8-cf): byte swap endian ──
            // Berguna untuk GRUB little-endian ↔ big-endian conversion.
            0xc8..=0xcf => {
                let r = (op - 0xc8) as usize;
                let v = self.r32(r);
                let swapped = v.swap_bytes();
                self.r32_set(r, swapped);
            }
            // ── shld/shrd (0f a4/a5 = shld imm/cl, 0f ac/ad = shrd imm/cl) ──
            0xa4 | 0xa5 | 0xac | 0xad => {
                self.exec_double_shift(op, opsz, mem)?;
            }
            _ => self.halt(&format!("opcode 0f 0x{:02x} belum didukung", op)),
        }
        Ok(())
    }

    /// Kondisi jcc (op & 0x0f): 0=o 1=no 2=b/c 3=ae/nb 4=e/z 5=ne/nz 6=be 7=a
    /// 8=s 9=ns a=p b=np c=l d=ge e=le f=g.
    fn jcc_taken(&self, cond: u8) -> bool {
        let cf = self.cf();
        let zf = self.flag(FLAG_ZF);
        let sf = self.flag(FLAG_SF);
        let of = self.flag(FLAG_OF);
        let pf = self.flag(FLAG_PF);
        match cond {
            0 => of,
            1 => !of,
            2 => cf,
            3 => !cf,
            4 => zf,
            5 => !zf,
            6 => cf || zf,
            7 => !cf && !zf,
            8 => sf,
            9 => !sf,
            10 => pf,
            11 => !pf,
            12 => sf != of,
            13 => sf == of,
            14 => zf || sf != of,
            15 => !zf && sf == of,
            _ => false,
        }
    }

    // ── string ops ──
    fn dir_step(&self, n: u32) -> i32 {
        if self.flag(FLAG_DF) {
            -(n as i32)
        } else {
            n as i32
        }
    }

    /// SI 32-bit penuh (protected mode; di real mode = 16-bit, high bits 0).
    fn si32(&self) -> u32 {
        if self.pmode {
            self.r32(6)
        } else {
            self.si() as u32
        }
    }
    /// DI 32-bit penuh (protected mode; di real mode = 16-bit, high bits 0).
    fn di32(&self) -> u32 {
        if self.pmode {
            self.r32(7)
        } else {
            self.di() as u32
        }
    }
    fn si32_set(&mut self, v: u32) {
        if self.pmode {
            self.r32_set(6, v);
        } else {
            self.si_set(v as u16);
        }
    }
    fn di32_set(&mut self, v: u32) {
        if self.pmode {
            self.r32_set(7, v);
        } else {
            self.di_set(v as u16);
        }
    }
    /// Counter REP: ECX di pmode (address-size 32), CX di real mode.
    fn rep_cnt(&self) -> u32 {
        if self.pmode {
            self.r32(1)
        } else {
            self.cx() as u32
        }
    }
    fn rep_cnt_set(&mut self, v: u32) {
        if self.pmode {
            self.r32_set(1, v);
        } else {
            self.cx_set(v as u16);
        }
    }

    /// Cek apakah range [addr, addr+n) menyentuh buffer VGA text (0xB8000+).
    /// Write ke VGA harus lewat write8/16/32 per-byte agar mirror terisi.
    #[inline]
    fn vga_hits(&self, addr: u64, n: usize) -> bool {
        let va = VGA_TEXT_ADDR;
        let vr = VGA_TEXT_ADDR + VGA_TEXT_SIZE as u64;
        addr < vr && addr + n as u64 > va
    }

    fn movs(
        &mut self,
        mem: &mut dyn MemoryPort,
        n: u32,
        rep: bool,
        seg_ov: Option<u16>,
    ) -> Result<(), CpuFault> {
        let src_seg = seg_ov.unwrap_or(self.ds);
        loop {
            let (si, di) = (self.si32(), self.di32());
            if std::env::var("MARIA_X86_TRACE").is_ok() && !rep {
                eprintln!(
                    "MOVS n={n} si=0x{si:x} di=0x{di:x} es=0x{:x} ds=0x{:x} pmode={}",
                    self.es, src_seg, self.pmode
                );
            }
            // Fast path: copy bulk (n ∈ {1,2,4}). Dest VGA → per-byte (mirror).
            let dst_lin = self.lin(self.es, di);
            if self.vga_hits(dst_lin, n as usize) {
                for i in 0..n {
                    let b = self.read8(mem, src_seg, si + i)?;
                    self.write8(mem, self.es, di + i, b)?;
                }
            } else {
                let mut buf = [0u8; 4];
                let src_lin = self.lin(src_seg, si);
                mem.read_exact(src_lin, &mut buf[..n as usize])
                    .map_err(|e| self.fault(format!("movs read 0x{:x}: {}", src_lin, e)))?;
                mem.write_exact(dst_lin, &buf[..n as usize])
                    .map_err(|e| self.fault(format!("movs write 0x{:x}: {}", dst_lin, e)))?;
            }
            let step = self.dir_step(n);
            self.si32_set((si as i64 + step as i64) as u32);
            self.di32_set((di as i64 + step as i64) as u32);
            if std::env::var("MARIA_X86_TRACE").is_ok() && !rep && false {
                eprintln!("MOVS after: si=0x{:x} di=0x{:x}", self.si32(), self.di32());
            }
            if !rep {
                break;
            }
            self.rep_cnt_set(self.rep_cnt().wrapping_sub(1));
            if self.rep_cnt() == 0 {
                break;
            }
        }
        Ok(())
    }

    fn cmps(
        &mut self,
        mem: &mut dyn MemoryPort,
        n: u32,
        rep: bool,
        seg_ov: Option<u16>,
    ) -> Result<(), CpuFault> {
        let src_seg = seg_ov.unwrap_or(self.ds);
        loop {
            let (si, di) = (self.si32(), self.di32());
            let mut a = [0u8; 4];
            let mut b = [0u8; 4];
            let src_lin = self.lin(src_seg, si);
            let dst_lin = self.lin(self.es, di);
            mem.read_exact(src_lin, &mut a[..n as usize])
                .map_err(|e| self.fault(format!("cmps read 0x{:x}: {}", src_lin, e)))?;
            mem.read_exact(dst_lin, &mut b[..n as usize])
                .map_err(|e| self.fault(format!("cmps read 0x{:x}: {}", dst_lin, e)))?;
            let mut neq = false;
            for i in 0..n as usize {
                if a[i] != b[i] {
                    neq = true;
                }
            }
            self.set_flag(FLAG_ZF, !neq);
            let step = self.dir_step(n);
            self.si32_set((si as i64 + step as i64) as u32);
            self.di32_set((di as i64 + step as i64) as u32);
            if !rep {
                break;
            }
            self.rep_cnt_set(self.rep_cnt().wrapping_sub(1));
            if self.rep_cnt() == 0 || (rep && !neq) {
                break;
            }
        }
        Ok(())
    }

    fn stos(
        &mut self,
        mem: &mut dyn MemoryPort,
        n: u32,
        rep: bool,
        seg_ov: Option<u16>,
    ) -> Result<(), CpuFault> {
        let _ = seg_ov;
        // Pola byte dari register A (dilebarkan ke 4 byte).
        let mut pat = [0u8; 4];
        for i in 0..n {
            pat[i as usize] = if n == 1 {
                self.r8(0)
            } else if n == 2 {
                (self.r16(0) >> (8 * i)) as u8
            } else {
                (self.r32(0) >> (8 * i)) as u8
            };
        }
        loop {
            let di = self.di32();
            let dst_lin = self.lin(self.es, di);
            if self.vga_hits(dst_lin, n as usize) {
                // Dest VGA → per-byte (agar mirror VGA ikut terisi).
                for i in 0..n as usize {
                    self.write8(mem, self.es, di + i as u32, pat[i])?;
                }
            } else {
                mem.write_exact(dst_lin, &pat[..n as usize])
                    .map_err(|e| self.fault(format!("stos write 0x{:x}: {}", dst_lin, e)))?;
            }
            self.di32_set((di as i64 + self.dir_step(n) as i64) as u32);
            if !rep {
                break;
            }
            self.rep_cnt_set(self.rep_cnt().wrapping_sub(1));
            if self.rep_cnt() == 0 {
                break;
            }
        }
        Ok(())
    }

    fn lods(&mut self, mem: &mut dyn MemoryPort, n: u32) -> Result<(), CpuFault> {
        let step = self.dir_step(n);
        let si = self.si32();
        let src_lin = self.lin(self.ds, si);
        let mut buf = [0u8; 4];
        mem.read_exact(src_lin, &mut buf[..n as usize])
            .map_err(|e| self.fault(format!("lods read 0x{:x}: {}", src_lin, e)))?;
        if n == 1 {
            self.r8_set(0, buf[0]);
        } else if n == 2 {
            self.r16_set(0, u16::from_le_bytes([buf[0], buf[1]]));
        } else {
            self.r32_set(0, u32::from_le_bytes(buf));
        }
        self.si32_set((si as i64 + step as i64) as u32);
        Ok(())
    }

    // ── INT dispatch → BIOS stub ──
    fn int_dispatch(&mut self, n: u8, mem: &mut dyn MemoryPort) -> Result<(), CpuFault> {
        match n {
            0x10 => self.int10(mem),
            0x12 => self.int12(mem),
            0x13 => self.int13(mem),
            0x15 => self.int15(mem),
            0x16 => self.int16(mem),
            0x1a => self.int1a(mem),
            0x21 => self.int21(mem),
            _ => {
                // INT tak dikenal: hentikan (agar Machine berhenti, tidak loop).
                self.halt(&format!("INT 0x{:02x} belum didukung", n));
                Ok(())
            }
        }
    }

    /// INT 10h — video. AH=0E teletype (output ke console). Lain → no-op.
    fn int10(&mut self, mem: &mut dyn MemoryPort) -> Result<(), CpuFault> {
        let _ = mem;
        match self.r8h(0) {
            0x0e => self.out.push(self.r8(0)),
            0x00 => {
                // set video mode: no-op
            }
            _ => {}
        }
        Ok(())
    }

    /// INT 12h — memory size (conventional). Return AX = KB di bawah 1MB.
    fn int12(&mut self, mem: &mut dyn MemoryPort) -> Result<(), CpuFault> {
        let _ = mem;
        // 640 KB conventional (standard IBM PC).
        self.r16_set(0, 640);
        Ok(())
    }

    /// INT 13h — disk BIOS.
    /// AH=00 reset, AH=02 read CHS, AH=41 check ext, AH=42 extended read (DAP).
    fn int13(&mut self, mem: &mut dyn MemoryPort) -> Result<(), CpuFault> {
        let ah = self.r8h(0);
        match ah {
            0x00 => {
                self.r8h_set(0, 0); // sukses
            }
            0x02 => {
                // read CHS: ch=cyl, cl=sect(+cyl bit 7-6), dh=head, dl=drive; es:bx = buffer
                let cyl = (self.r8h(1) as u64) | (((self.r8(1) >> 6) & 3) as u64) << 8;
                let sect = (self.r8(1) & 0x3f) as u64;
                let dh = self.r8h(2) as u64;
                let al = self.r8(0) as u64;
                let spt = self.drive_spt as u64;
                let heads = self.drive_heads as u64;
                let lba = ((cyl * heads + dh) * spt) + (sect - 1); // CHS→LBA
                let buf_seg = self.es;
                let buf_off = self.r16(3);
                let mut data = vec![0u8; (al * 512) as usize];
                match self.read_disk(lba, al as u16, &mut data) {
                    Ok(()) => {
                        for i in 0..data.len() {
                            self.write8(mem, buf_seg, buf_off as u32 + i as u32, data[i])?;
                        }
                        self.r8h_set(0, 0);
                    }
                    Err(_) => {
                        self.set_flag(FLAG_CF, true);
                    }
                }
            }
            0x41 => {
                // check extensions: request BX=55AAh → sukses BX=AA55h (spec INT 13h),
                // AH=0, CX = API subset (bit0 = fixed disk)
                self.r16_set(3, 0xaa55);
                self.r8h_set(0, 0);
                self.r16_set(1, 0x0007); // dukung fixed disk + lock + enhanced
                self.set_flag(FLAG_CF, false);
            }
            0x08 => {
                // get drive parameters: DH = max head - 1, CL = sector | cyl bits
                // 7-6, CH = cyl low, DL = jumlah drive. Geometri konsisten dengan
                // konversi CHS→LBA di AH=02 (18 sector/track, 2 heads).
                self.r8h_set(2, (self.drive_heads - 1) as u8); // DH
                self.r8_set(1, (self.drive_spt & 0x3f) as u8); // CL (sector)
                self.r8h_set(1, 0); // CH (cyl low)
                self.r8_set(2, 0x80); // DL = drive 0x80
                self.set_flag(FLAG_CF, false);
            }
            0x42 => {
                // extended read: DAP 16-byte di DS:SI (size@0, count@2,
                // offset@4, seg@6, lba@8) — layout yang dipakai boot loader
                // real (isohdpfx/GRUB), BUKAN 24-byte (seg@12). LBA = 64-bit
                // (dword rendah + dword tinggi), bukan 32-bit — DAP spec.
                let si = self.si() as u32;
                let count = self.read16(mem, self.ds, si + 2)?;
                let off = self.read16(mem, self.ds, si + 4)?;
                let seg = self.read16(mem, self.ds, si + 6)?;
                let lba = self.read32(mem, self.ds, si + 8)? as u64
                    | ((self.read32(mem, self.ds, si + 12)? as u64) << 32);
                // Drive CD (El Torito no-emul): LBA & count dalam blok 2048-byte
                // (CD logical sector), bukan 512 (HDD). GRUB CD (biosdisk)
                // membuka CD dengan log_sector_size=11 → semua AH=42 via DL CD.
                let is_cd = self.is_cd_drive(self.dl());
                let unit: u64 = if is_cd { 2048 } else { 512 };
                let mut data = vec![0u8; (count as usize) * unit as usize];
                match if is_cd {
                    self.read_cd_blocks(lba, count, &mut data)
                } else {
                    self.read_disk(lba, count, &mut data)
                } {
                    Ok(()) => {
                        // Tulis bulk ke buffer seg:off (fast path).
                        let dst_lin = self.lin(seg, off as u32);
                        if self.vga_hits(dst_lin, data.len()) {
                            for (i, &b) in data.iter().enumerate() {
                                self.write8(mem, seg, off as u32 + i as u32, b)?;
                            }
                        } else {
                            mem.write_exact(dst_lin, &data).map_err(|e| {
                                self.fault(format!("int13 AH=42 buf 0x{:x}: {}", dst_lin, e))
                            })?;
                        }
                        self.r8h_set(0, 0);
                        self.set_flag(FLAG_CF, false);
                    }
                    Err(_) => {
                        self.r8h_set(0, 0x0c);
                        self.set_flag(FLAG_CF, true);
                    }
                }
            }
            0x4b => {
                // El Torito/BIOS CD extensions.
                // AL=01: Get CD-ROM Information — isi CD drive parameters (CDRP)
                // di DS:SI; GRUB biosdisk membaca media_type@1 dan drive_no@2.
                // AL=00: read dengan callback (grub cdrom.S lama) — belum didukung.
                let al = self.r8(0);
                let Some(dn) = self.cd_drive else {
                    // Tidak ada CD → gagal (GRUB: "no CD").
                    self.r8h_set(0, 0x31);
                    self.set_flag(FLAG_CF, true);
                    return Ok(());
                };
                match al {
                    0x01 => {
                        let base = (self.ds as u64) * 16 + self.si() as u64;
                        // struct grub_biosdisk_cdrp (biosdisk.h):
                        //   size@0, media_type@1, drive_no@2, controller_no@3,
                        //   image_lba@4..8, ...
                        mem.write(base + 1, 1, 0)
                            .map_err(|e| self.fault(format!("cdrp media_type: {e}")))?;
                        mem.write(base + 2, 1, dn as u64)
                            .map_err(|e| self.fault(format!("cdrp drive_no: {e}")))?;
                        self.r8h_set(0, 0); // sukses
                        self.set_flag(FLAG_CF, false);
                    }
                    _ => {
                        // AL=00 read-with-callback: belum didukung → CF error.
                        self.r8h_set(0, 0x05);
                        self.set_flag(FLAG_CF, true);
                    }
                }
            }
            _ => {
                self.halt(&format!("INT 13h AH=0x{:02x} belum didukung", ah));
            }
        }
        Ok(())
    }

    /// Baca sektor dari disk backend.
    fn read_disk(&mut self, lba: u64, count: u16, buf: &mut [u8]) -> Result<(), String> {
        let Some(disk) = self.disk.as_mut() else {
            return Err("tidak ada disk".into());
        };
        disk.read(lba, count, buf)
    }

    /// Drive `dl` adalah drive CD (El Torito no-emul) yang dikonfigurasi?
    #[inline]
    fn is_cd_drive(&self, dl: u8) -> bool {
        self.cd_drive == Some(dl)
    }

    /// Baca `count` blok 2048-byte dari CD ISO: byte offset = lba * 2048.
    /// Tiap blok dibaca terpisah agar EOF parsial di-zero-fill benar.
    fn read_cd_blocks(&mut self, lba: u64, count: u16, buf: &mut [u8]) -> Result<(), String> {
        let Some(disk) = self.disk.as_mut() else {
            return Err("tidak ada disk".into());
        };
        let mut pos = 0usize;
        for i in 0..count as u64 {
            let off = lba.saturating_mul(2048) + i * 2048;
            let end = (pos + 2048).min(buf.len());
            if end <= pos {
                break;
            }
            disk.read_bytes(off, &mut buf[pos..end])?;
            pos = end;
        }
        Ok(())
    }

    /// INT 15h — system: AX=E820 memory map, AH=88 extended mem size.
    fn int15(&mut self, mem: &mut dyn MemoryPort) -> Result<(), CpuFault> {
        let eax = self.r32(0);
        if eax == 0xe820 {
            // Get System Memory Map: ES:DI = buffer, EBX = continuation,
            // ECX = buffer size (24), EDX = 'SMAP'. Satu entry: base(8),
            // length(8), type(4), acpi(4).
            let di = self.r16(7) as u32;
            let buf_seg = self.es;
            let ram = 0x400_0000u64; // 64MB (probe)
                                     // base 0, length 64MB, type 1 (usable)
            for (i, v) in [0u64, ram, 1u64, 1u64].iter().enumerate() {
                let off = di + (i as u32) * 8;
                self.write32(mem, buf_seg, off, *v as u32)?;
                self.write32(mem, buf_seg, off + 4, (v >> 32) as u32)?;
            }
            self.r32_set(1, 0); // ebx = done (1 entry)
            self.r32_set(2, 24); // ecx = ukuran entry
            self.r32_set(3, 0x534d4150); // edx = 'SMAP'
            self.set_flag(FLAG_CF, false);
            return Ok(());
        }
        match self.r8h(0) {
            0x88 => {
                // extended mem (KB di atas 1MB) → AX
                self.r16_set(0, ((0x400_0000 - 0x100000) / 1024) as u16);
                self.set_flag(FLAG_CF, false);
            }
            0xc0 => {
                // get system parameters: ES:BX → descriptor
                self.r16_set(3, 0);
                self.r8h_set(0, 0);
                self.set_flag(FLAG_CF, false);
            }
            _ => {
                self.set_flag(FLAG_CF, true); // tak didukung
            }
        }
        Ok(())
    }

    /// INT 16h — keyboard: AH=00 baca key (kosong → AX=0), AH=01 status.
    fn int16(&mut self, _mem: &mut dyn MemoryPort) -> Result<(), CpuFault> {
        match self.r8h(0) {
            0x00 => {
                self.r16_set(0, 0); // tidak ada key
            }
            0x01 => {
                self.set_flag(FLAG_ZF, true); // buffer kosong
            }
            _ => {}
        }
        Ok(())
    }

    /// INT 1Ah — time: AH=00/02 → CX:DX = 0 (jam tengah malam).
    fn int1a(&mut self, _mem: &mut dyn MemoryPort) -> Result<(), CpuFault> {
        match self.r8h(0) {
            0x00 | 0x02 => {
                self.r16_set(1, 0);
                self.r16_set(2, 0);
            }
            _ => {}
        }
        Ok(())
    }

    /// INT 21h — DOS: AH=0x02/0x06 tulis char, AH=0x09 string. Console.
    fn int21(&mut self, mem: &mut dyn MemoryPort) -> Result<(), CpuFault> {
        match self.r8h(0) {
            0x02 | 0x06 => self.out.push(self.r8(2)),
            0x09 => {
                // string di DS:DX sampai '$'
                let mut off = self.r16(2) as u32;
                loop {
                    let b = self.read8(mem, self.ds, off)?;
                    if b == b'$' {
                        break;
                    }
                    self.out.push(b);
                    off += 1;
                }
            }
            _ => {}
        }
        Ok(())
    }

    // ── boot helper ──
    /// Muat boot sector ke 0x0000:0x7C00 (BIOS boot convention).
    pub fn load_boot_sector(
        &mut self,
        mem: &mut dyn MemoryPort,
        data: &[u8],
    ) -> Result<(), CpuFault> {
        self.write8(mem, 0x0000, 0x7c00, 0)?; // pastikan region ada
        for (i, b) in data.iter().take(512).enumerate() {
            self.write8(mem, 0x0000, 0x7c00 + i as u32, *b)?;
        }
        self.cs = 0x0000;
        self.ip = 0x7c00;
        self.ds = 0x0000;
        self.es = 0x0000;
        self.ss = 0x0000;
        self.sp_set(0x7c00);
        Ok(())
    }

    // ── boot helper CD (El Torito no-emul) ──
    /// Muat boot image ISO (cdboot/GRUB) ke 0x0000:0x7C00 seperti BIOS CD:
    /// DL = nomor drive CD (`drive`, biasanya 0xE0) + `cd_drive` dikonfigurasi
    /// agar INT 13h dibaca sebagai drive CD (blok 2048).
    pub fn load_boot_image(
        &mut self,
        mem: &mut dyn MemoryPort,
        data: &[u8],
        drive: u8,
    ) -> Result<(), CpuFault> {
        // Muat maksimal sampai 0x9FC00 (bawah EBDA), sisanya tidak relevan
        // boot image no-emul (cdboot 512 byte; BIOS umumnya muat 1 sektor).
        let cap = (0x9FC00 - 0x7C00).min(data.len());
        self.write8(mem, 0x0000, 0x7c00, 0)?; // pastikan region ada
        for i in 0..cap {
            self.write8(mem, 0x0000, 0x7c00 + i as u32, data[i])?;
        }
        self.cs = 0x0000;
        self.ip = 0x7c00;
        self.ds = 0x0000;
        self.es = 0x0000;
        self.ss = 0x0000;
        self.sp_set(0x7c00);
        self.dl_set(drive);
        self.cd_drive = Some(drive);
        Ok(())
    }
}

/// Disk backend dari file mentah (ISO / image). `lba` = sektor 512-byte.
pub struct FileDisk {
    file: std::fs::File,
    len: u64,
}

impl FileDisk {
    pub fn open(path: &str) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {}", path, e))?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        Ok(Self { file, len })
    }
}

impl X86Disk for FileDisk {
    fn read(&mut self, lba: u64, count: u16, buf: &mut [u8]) -> Result<(), String> {
        use std::io::{Read, Seek, SeekFrom};
        let need = lba
            .checked_mul(512)
            .and_then(|o| o.checked_add((count as u64) * 512));
        let Some(need) = need else {
            return Err("overflow".into());
        };
        if need > self.len {
            // baca parsial: zero-fill sisanya (CD media sering baca melewati EOF)
            buf.fill(0);
            let avail = self.len.saturating_sub(lba * 512);
            let n = (avail / 512) as u16 * 512;
            if n == 0 {
                return Ok(());
            }
            self.file
                .seek(SeekFrom::Start(lba * 512))
                .map_err(|e| e.to_string())?;
            self.file
                .read_exact(&mut buf[..n as usize])
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        self.file
            .seek(SeekFrom::Start(lba * 512))
            .map_err(|e| e.to_string())?;
        self.file.read_exact(buf).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn total_sectors(&self) -> u64 {
        self.len / 512
    }

    fn read_bytes(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), String> {
        use std::io::{Read, Seek, SeekFrom};
        if offset >= self.len {
            // baca lewat EOF: zero-fill (CD media sering baca melewati akhir).
            buf.fill(0);
            return Ok(());
        }
        let avail = self.len.saturating_sub(offset) as usize;
        let n = avail.min(buf.len());
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
        self.file
            .read_exact(&mut buf[..n])
            .map_err(|e| e.to_string())?;
        buf[n..].fill(0);
        Ok(())
    }
}

impl CpuCore for X86Cpu {
    fn reset(&mut self) {
        *self = X86Cpu::new();
    }

    fn step(&mut self, mem: &mut dyn MemoryPort) -> Result<CpuStep, CpuFault> {
        if self.halted {
            return Ok(CpuStep::Trap {
                cause: 0,
                tval: self.pc(),
            });
        }
        self.steps += 1;
        let dbg_step = (8_623_590..=8_623_680).contains(&self.steps)
            && std::env::var("MARIA_X86_DBG").is_ok();
        if dbg_step {
            // State sebelum instruksi (dipecahkan per-byte, tanpa akses memori
            // tambahan yang bisa salah saat mode campuran).
            let pc = self.pc();
            let mut bs = [0u8; 8];
            for (i, b) in bs.iter_mut().enumerate() {
                *b = self.read8(mem, self.cs, self.ip.wrapping_add(i as u32))?;
            }
            eprintln!(
                "DBG step={} pc=0x{:08x} cs=0x{:x} ip=0x{:08x} pmode={} cr0={:#x} sp=0x{:08x} ax={:#x} bx={:#x} cx={:#x} dx={:#x} si={:#x} di={:#x} bp={:#x} [{}]",
                self.steps, pc, self.cs, self.ip, self.pmode, self.cr[0],
                self.gpr[4], self.gpr[0], self.gpr[3], self.gpr[1], self.gpr[2],
                self.gpr[6], self.gpr[7], self.gpr[5],
                bs.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
            );
        }
        self.exec_one(mem)?;
        if self.halted {
            return Ok(CpuStep::Trap {
                cause: 0,
                tval: self.pc(),
            });
        }
        Ok(CpuStep::InstructionExecuted { cycles: 1 })
    }

    fn pc(&self) -> u64 {
        if self.pmode {
            self.gdt_seg_base(self.cs) as u64 + self.ip as u64
        } else {
            ((self.cs as u64) << 4) + self.ip as u64
        }
    }

    fn set_pc(&mut self, addr: u64) {
        self.cs = ((addr >> 4) & 0xffff) as u16;
        self.ip = (addr & 0xf) as u32;
    }

    fn raise_interrupt(&mut self, _irq: u32, _level: bool) {
        // Real-mode boot: PIC belum dimodelkan.
    }

    fn read_reg(&self, idx: usize) -> u64 {
        if idx < 8 {
            self.gpr[idx] as u64
        } else {
            0
        }
    }

    fn isa(&self) -> Isa {
        Isa::X86_64
    }

    /// Console output BIOS (INT 10h AH=0E teletype / INT 21h DOS output).
    /// Di-wire ke `MachineResult.console` agar CLI `--boot-iso` menampilkan
    /// output BIOS nyata (sebelumnya selalu kosong — `out` tidak pernah
    /// diekspos).
    fn console_output(&self) -> &[u8] {
        &self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::{MemoryMap, RamRegion, RegionKind};
    use maria_core::intern::Symbol;

    fn mem() -> MemoryMap {
        let mut m = MemoryMap::new();
        m.add(
            RamRegion::new(
                Symbol::intern("ram"),
                0x0,
                0x10_0000,
                RegionKind::Ram,
                false,
            )
            .unwrap(),
        )
        .unwrap();
        m
    }

    fn root_of(rel: &str) -> String {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .unwrap()
            .to_path_buf();
        root.join(rel).to_string_lossy().to_string()
    }

    /// Muat opcode mentah ke 0x7c00, CS:IP = 0:0x7c00.
    fn load(m: &mut MemoryMap, code: &[u8]) -> X86Cpu {
        let mut cpu = X86Cpu::new();
        cpu.load_boot_sector(m, code).unwrap();
        cpu
    }

    fn run(cpu: &mut X86Cpu, m: &mut MemoryMap, n: usize) {
        for _ in 0..n {
            let _ = cpu.step(m).unwrap();
            if cpu.halted {
                break;
            }
        }
    }

    #[test]
    fn test_mov_imm_and_jcc() {
        // b8 34 12 (mov ax, 0x1234); 3c 34 (cmp al, 0x34 → ZF); 75 02 (jnz +2, TIDAK diambil →
        // fall-through ke mov); b8 99 00 (mov ax, 0x99)
        let code = [0xb8, 0x34, 0x12, 0x3c, 0x34, 0x75, 0x02, 0xb8, 0x99, 0x00];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 4); // mov, cmp, (jnz tidak diambil), mov ax=0x99
        assert_eq!(cpu.r16(0), 0x99, "ax harus 0x99 (jcc tidak diambil)");
    }

    #[test]
    fn test_push_pop_call_ret() {
        // b8 99 00 (mov ax, 0x99); e8 06 00 (call sub di 0x7c0c); eb fe (jmp -2, "setelah call");
        // 00 00 00 00 (padding); b8 2a 00 (mov ax, 0x2a); c3 (ret → kembali ke 0x7c06)
        let code = [
            0xb8, 0x99, 0x00, // 0x7c00: mov ax, 0x99
            0xe8, 0x06, 0x00, // 0x7c03: call +6 → 0x7c0c
            0xeb, 0xfe, // 0x7c06: jmp -2 (return address dari call)
            0x00, 0x00, 0x00, 0x00, // padding
            0xb8, 0x2a, 0x00, // 0x7c0c: mov ax, 0x2a
            0xc3, // 0x7c0f: ret
        ];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 4); // mov, call, mov ax=0x2a, ret
        assert_eq!(cpu.r16(0), 0x2a, "call/ret harus kembali ke mov ax,0x2a");
    }

    #[test]
    fn test_opsize_prefix_32bit() {
        // 66 b8 78 56 34 12 (mov eax, 0x12345678); 66 50 (push eax); 66 58 (pop eax)
        let code = [0x66, 0xb8, 0x78, 0x56, 0x34, 0x12, 0x66, 0x50, 0x66, 0x58];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 3);
        assert_eq!(cpu.r32(0), 0x1234_5678, "push/pop 32-bit harus bulat");
    }

    #[test]
    fn test_mem_mov_and_segment_override() {
        // be 00 80 (mov si, 0x8000); 2e 8a 04 (mov al, [cs:si]); 8a 04 (mov al, [ds:si])
        let code = [0xbe, 0x00, 0x80, 0x2e, 0x8a, 0x04, 0x8a, 0x04];
        let mut m = mem();
        m.write(0x8000, 1, 0x42).unwrap(); // [cs:si] dengan cs=0 → 0x8000
        m.write(0x8080, 1, 0x24).unwrap(); // [ds:si] dengan ds=8 → 0x8080
        let mut cpu = load(&mut m, &code);
        cpu.ds = 0x0008;
        run(&mut cpu, &mut m, 3);
        assert_eq!(cpu.r8(0), 0x24, "al = [ds:si] (0x8080) = 0x24");
    }

    #[test]
    fn test_lodsb_int10_output() {
        // be 0c 7c (mov si, 0x7c0c → data); ac (lodsb); b4 0e (mov ah, 0x0e); cd 10 (int 10h);
        // 3c 00 (cmp al,0); 75 f7 (jnz -9) — data 'A' di 0x7c0c, 0x00 di 0x7c0d
        let code = [
            0xbe, 0x0c, 0x7c, 0xac, 0xb4, 0x0e, 0xcd, 0x10, 0x3c, 0x00, 0x75, 0xf7, b'A', 0x00,
        ];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 6); // loop sekali: tulis 'A', al=0 → stop
        assert_eq!(cpu.out, vec![b'A'], "INT 10h AH=0E harus output 'A'");
    }

    #[test]
    fn test_int13_extended_read_iso_mbr() {
        // INT 13h AH=42 dengan DAP di DS:SI — baca sektor 0 dari ISO
        let iso = root_of("ubuntu-26.04-desktop-amd64.iso");
        let mut cpu = X86Cpu::new();
        cpu.disk = Some(Box::new(FileDisk::open(&iso).expect("open iso")));
        let mut m = mem();
        cpu.es = 0x1000;
        // DAP 16-byte: size=16@0, count=1@2, off=0@4, seg=0x1000@6, lba=0@8
        m.write(0x0, 1, 16).unwrap();
        m.write(0x1, 1, 0).unwrap();
        m.write(0x2, 1, 1).unwrap();
        m.write(0x4, 1, 0x00).unwrap();
        m.write(0x5, 1, 0x00).unwrap();
        m.write(0x6, 1, 0x00).unwrap();
        m.write(0x7, 1, 0x10).unwrap(); // seg = 0x1000
        m.write(0x8, 1, 0).unwrap();
        m.write(0x9, 1, 0).unwrap();
        m.write(0xa, 1, 0).unwrap();
        m.write(0xb, 1, 0).unwrap();
        let code = [0xcd, 0x13];
        let mut cpu2 = load(&mut m, &code);
        cpu2.disk = cpu.disk.take();
        cpu2.ds = 0;
        cpu2.si_set(0);
        cpu2.r8h_set(0, 0x42); // AH=0x42 di CPU yang benar (cpu2), bukan cpu
        run(&mut cpu2, &mut m, 1);
        assert!(!cpu2.cf(), "INT 13h AH=42 harus sukses");
        // sektor 0 (MBR) terbaca di 0x1000:0
        let b = m.read(0x10000, 1).unwrap();
        assert_eq!(b, 0xeb, "MBR byte pertama = 0xeb (jmp short)");
        // signature 55 aa — read(510,2) little-endian = 0xaa55
        let sig = m.read(0x10000 + 510, 2).unwrap();
        assert_eq!(sig, 0xaa55, "MBR signature (little-endian)");
    }

    /// E2E: eksekusi MBR asli ISO (ISOLINUX hybrid) — beberapa instruksi pertama.
    #[test]
    fn test_boot_iso_mbr_executes() {
        let iso = root_of("ubuntu-26.04-desktop-amd64.iso");
        let mut file = std::fs::File::open(&iso).expect("open");
        let mut mbr = [0u8; 512];
        use std::io::Read;
        file.read_exact(&mut mbr).expect("read mbr");
        assert_eq!(mbr[510], 0x55);
        assert_eq!(mbr[511], 0xaa);
        let mut m = mem();
        let mut cpu = load(&mut m, &mbr);
        cpu.disk = Some(Box::new(FileDisk::open(&iso).expect("disk")));
        // jmp 0x65; nop...; boot sector ISOLINUX — harus jalan tanpa fault
        let mut ok = 0;
        for _ in 0..40 {
            match cpu.step(&mut m) {
                Ok(_) => ok += 1,
                Err(e) => panic!("fault @0x{:x}: {}", cpu.pc(), e.reason),
            }
            if cpu.halted {
                break;
            }
        }
        assert!(
            ok >= 10,
            "MBR harus mengeksekusi >= 10 instruksi, dapat {}",
            ok
        );
        // IP harus maju melewati header (jmp 0x65) dan memanggil INT 13h
        assert!(
            cpu.ip > 0x65,
            "IP harus melewati jmp awal, ip=0x{:x}",
            cpu.ip
        );
    }

    /// E2E: GRUB boot.img (El Torito LBA 667) — eksekusi dengan INT 13h AH=42.
    #[test]
    fn test_boot_grub_bootimg_executes() {
        let iso = root_of("ubuntu-26.04-desktop-amd64.iso");
        use std::io::{Read, Seek, SeekFrom};
        let mut file = std::fs::File::open(&iso).expect("open");
        file.seek(SeekFrom::Start(667 * 2048)).expect("seek");
        let mut img = [0u8; 4096];
        file.read_exact(&mut img).expect("read boot img");
        let mut m = mem();
        let mut cpu = load(&mut m, &img);
        cpu.disk = Some(Box::new(FileDisk::open(&iso).expect("disk")));
        // boot.img: call next / jmp; ...; xor ax,ax; mov ss,ax; mov sp,0x6000; ...
        let mut ok = 0;
        for _ in 0..80 {
            match cpu.step(&mut m) {
                Ok(_) => ok += 1,
                Err(e) => panic!("fault @0x{:x}: {}", cpu.pc(), e.reason),
            }
            if cpu.halted {
                break;
            }
        }
        assert!(
            ok >= 20,
            "GRUB boot.img harus mengeksekusi >= 20 instruksi, dapat {}",
            ok
        );
    }

    #[test]
    fn test_cpuid_leaf0_vendor() {
        // CPUID leaf 0 → vendor string "GenuineIntel"
        let code = [
            0x31, 0xc0,             // xor eax, eax  (eax = 0)
            0x0f, 0xa2,             // cpuid
            0x90,                   // nop (placeholder)
        ];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 3);
        // EBX=0x756e6547 ('uneG'), EDX=0x49656e69 ('ineI'), ECX=0x6c65746e ('ntel')
        assert_eq!(cpu.r32(1), 0x756e_6547, "EBX vendor 'uneG'");
        assert_eq!(cpu.r32(3), 0x4965_6e69, "EDX vendor 'ineI'");
        assert_eq!(cpu.r32(2), 0x6c65_746e, "ECX vendor 'ntel'");
        // Vendor string utuh "GenuineIntel" (little-endian per 4-byte reg).
        let mut v: Vec<u8> = Vec::new();
        v.extend_from_slice(&cpu.r32(1).to_le_bytes());
        v.extend_from_slice(&cpu.r32(3).to_le_bytes());
        v.extend_from_slice(&cpu.r32(2).to_le_bytes());
        assert_eq!(String::from_utf8_lossy(&v), "GenuineIntel");
    }

    #[test]
    fn test_cpuid_leaf1_features() {
        // CPUID leaf 1 → features + stepping
        // Real mode: xor eax, eax + mov al, 1 (set low byte saja, 0 extend ke 32-bit)
        let code = [
            0x31, 0xc0,    // xor eax, eax
            0xb0, 0x01,    // mov al, 1 → eax = 1
            0x0f, 0xa2,    // cpuid
            0x90,          // nop
        ];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 4);
        // EDX harus punya FPU(0), TSC(4), CX8(8), SSE(25), SSE2(26)
        let edx = cpu.r32(3);
        assert!(edx & (1 << 0) != 0, "FPU bit harus set");
        assert!(edx & (1 << 4) != 0, "TSC bit harus set");
        assert!(edx & (1 << 25) != 0, "SSE bit harus set");
        assert!(edx & (1 << 26) != 0, "SSE2 bit harus set");
    }

    #[test]
    fn test_bswap_ecx() {
        // BSWAP ECX (0f c9): 0x12345678 → 0x78563412
        // Real mode: prefix 66 untuk 32-bit operand
        let code = [
            0x66, 0xb9, 0x78, 0x56, 0x34, 0x12, // mov ecx, 0x12345678
            0x0f, 0xc9,                           // bswap ecx
            0x90,                                 // nop
        ];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 3);
        assert_eq!(cpu.r32(1), 0x7856_3412, "bswap ecx harus balik byte");
    }

    #[test]
    fn test_bswap_eax() {
        // BSWAP EAX (0f c8): 0xAABBCCDD → 0xDDCCBBAA
        let code = [
            0x66, 0xb8, 0xDD, 0xCC, 0xBB, 0xAA, // mov eax, 0xAABBCCDD
            0x0f, 0xc8,                           // bswap eax
            0x90,                                 // nop
        ];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 3);
        assert_eq!(cpu.r32(0), 0xDDCC_BBAA, "bswap eax harus balik byte");
    }

    #[test]
    fn test_shrd_shld() {
        // 16-bit SHRD imm8 (0f ac /2, imm): shrd ax, cx, 4
        // ax = 0x1234, cx = 0xABCD → (0x1234>>4) | (0xABCD<<12) = 0x0123 | 0xD000 = 0xD123
        // Bahaya endian: shrd dest, src, x: dest.right(x) | src.left(16-x)
        let code = [
            0xb8, 0x34, 0x12,             // mov ax, 0x1234
            0xb9, 0xcd, 0xab,             // mov cx, 0xABCD
            0x0f, 0xac, 0xc8, 0x04,       // shrd ax, cx, 4
            0x90,
        ];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 4);
        assert_eq!(cpu.r16(0), 0xD123, "shrd ax, cx, 4 = 0xD123");
        // CF = bit terakhir keluar = bit (x-1=3) dari dest 0x1234 = 0
        assert_eq!(cpu.flag(FLAG_CF), false, "CF = bit 3 dest = 0");

        // 32-bit SHRD imm (prefix 66): shrd eax, ecx, 8
        // eax = 0x12345678, ecx = 0xAAAAAAAA → (eax>>8)|(ecx<<24) = 0x00123456 | 0xAA000000 = 0xAA123456
        let code = [
            0x66, 0xb8, 0x78, 0x56, 0x34, 0x12, // mov eax, 0x12345678
            0x66, 0xb9, 0xaa, 0xaa, 0xaa, 0xaa, // mov ecx, 0xAAAAAAAA
            0x66, 0x0f, 0xac, 0xc8, 0x08,       // shrd eax, ecx, 8
            0x90,
        ];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 4);
        assert_eq!(cpu.r32(0), 0xAA12_3456, "shrd eax, ecx, 8");

        // SHLD 16-bit: shld ax, cx, 4 → (0x1234<<4)|(0xABCD>>12) = 0x2340|0x000A = 0x234A
        let code = [
            0xb8, 0x34, 0x12,             // mov ax, 0x1234
            0xb9, 0xcd, 0xab,             // mov cx, 0xABCD
            0x0f, 0xa4, 0xc8, 0x04,       // shld ax, cx, 4
            0x90,
        ];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 4);
        assert_eq!(cpu.r16(0), 0x234A, "shld ax, cx, 4 = 0x234A");

        // SHRD by CL (0f ad): cl = 3 → shrd ax, dx, 3
        // ax = 0x1234, dx = 0xABCD → (0x1234>>3)|(0xABCD<<13) = 0x0246 | 0xA000 = 0xA246
        let code = [
            0xb8, 0x34, 0x12,             // mov ax, 0x1234
            0xba, 0xcd, 0xab,             // mov dx, 0xABCD (bukan CX — CL dipakai count)
            0xb1, 0x03,                   // mov cl, 3
            0x0f, 0xad, 0xd0,             // shrd ax, dx, cl  (modrm D0: rm=AX dest, reg=DX src)
            0x90,
        ];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 5);
        assert_eq!(cpu.r16(0), 0xA246, "shrd ax, dx, cl(3) = 0xA246");
    }

    #[test]
    fn test_rdtsc_returns_nonzero() {
        // RDTSC (0f 31): harus return timestamp > 0 setelah beberapa instruksi
        let code = [
            0x0f, 0x31, // rdtsc → EDX:EAX
            0x90,       // nop
        ];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 2);
        let ts = cpu.r32(0) as u64 | ((cpu.r32(2) as u64) << 32);
        assert!(ts > 0, "RDTSC harus return timestamp > 0");
    }

    #[test]
    fn test_cpuid_leaf_extended() {
        // CPUID leaf 0x80000001 → extended features (LM bit)
        let code = [
            0x66, 0xb8, 0x01, 0x00, 0x00, 0x80, // mov eax, 0x80000001
            0x0f, 0xa2,                           // cpuid
            0x90,                                 // nop
        ];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 3);
        let edx = cpu.r32(3);
        assert!(edx & (1 << 29) != 0, "Long Mode bit harus set");
        assert!(edx & (1 << 20) != 0, "NX bit harus set");
    }

    #[test]
    fn test_inc_dec_of_flag() {
        // OF INC/DEC harus set hanya saat signed overflow:
        //   INC: hasil == INT_MIN (0x8000)  → operand +max
        //   DEC: hasil == INT_MAX (0x7fff)  → operand −min
        // Grup ff (r/m16): ff c0 (inc ax), ff c8 (dec ax).
        let mut m = mem();
        // mov ax, 0x7fff; inc ax; dec ax
        let code = [
            0xb8, 0xff, 0x7f, // mov ax, 0x7fff
            0xff, 0xc0,       // inc ax → 0x8000 (overflow!)
            0xff, 0xc8,       // dec ax → 0x7fff (back, overflow!)
        ];
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 3);
        assert_eq!(cpu.flag(FLAG_OF), true, "inc ax 0x7fff→0x8000 harus OF");

        // Dari 0x8000: inc → 0x8001 (TIDAK overflow), dec dari 0x8001 → 0x8000 (tidak)
        let code = [
            0xb8, 0x00, 0x80, // mov ax, 0x8000
            0xff, 0xc0,       // inc ax → 0x8001 (tidak overflow)
            0xff, 0xc8,       // dec ax → 0x8000 (tidak overflow)
        ];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 3);
        assert_eq!(cpu.flag(FLAG_OF), false, "inc/dec di sekitar 0x8000 (negatif) tak OF");

        // 8-bit (fe): inc al 0x7f→0x80 overflow; dec al dari 0x00 → 0xff TIDAK overflow
        let code = [
            0xb0, 0x7f, // mov al, 0x7f
            0xfe, 0xc0, // inc al → 0x80 (overflow)
        ];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 2);
        assert_eq!(cpu.flag(FLAG_OF), true, "inc al 0x7f→0x80 harus OF");

        let code = [
            0xb0, 0x00, // mov al, 0x00
            0xfe, 0xc8, // dec al → 0xff (tidak overflow)
        ];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 2);
        assert_eq!(cpu.flag(FLAG_OF), false, "dec al 0x00→0xff tak OF");
    }

    #[test]
    fn test_idiv_signed_16bit() {
        // DX:AX = -21 (0xFFFFFFEB); idiv cx (cx=7) → AX = -3 (0xFFFD), DX = 0
        let code = [
            0xba, 0xff, 0xff, // mov dx, 0xffff
            0xb8, 0xeb, 0xff, // mov ax, 0xffeb
            0xb9, 0x07, 0x00, // mov cx, 7
            0xf7, 0xf9,       // idiv cx
            0x90,             // nop
        ];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 4);
        assert_eq!(cpu.r16(0) as i16, -3, "kuosien idiv -21/7 = -3");
        assert_eq!(cpu.r16(2), 0, "sisa idiv -21/7 = 0");
    }

    #[test]
    fn test_idiv_remainder_and_byzero() {
        // DX:AX = 7; idiv cx (cx=2) → AX=3, DX=1
        let code = [
            0xba, 0x00, 0x00, // mov dx, 0
            0xb8, 0x07, 0x00, // mov ax, 7
            0xb9, 0x02, 0x00, // mov cx, 2
            0xf7, 0xf9,       // idiv cx
            0x90,
        ];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 4);
        assert_eq!(cpu.r16(0), 3, "7/2 = 3");
        assert_eq!(cpu.r16(2), 1, "7%2 = 1");

        // idiv by zero → halted
        let code = [
            0xb9, 0x00, 0x00, // mov cx, 0
            0xf7, 0xf9,       // idiv cx → by zero
        ];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 2);
        assert!(cpu.halted, "idiv by zero harus halt");
    }

    #[test]
    fn test_lahf_sahf() {
        // lahf: 'lahf' opcode 0x9f — set flags lalu AH = FLAGS yg relevan.
        let code = [
            0xf9,       // stc → CF=1
            0x9f,       // lahf → AH = 0x03 (CF + bit1)
            0xb4, 0x00, // mov ah, 0
        ];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 2);
        assert_eq!(cpu.r8h(0), 0x03, "lahf dgn CF set → AH = 0x03");

        // sahf: muat AH ke flags → SF/ZF/CF (0xC1: SF+ZF+CF).
        let code = [
            0xb4, 0xc1, // mov ah, 0xc1 (SF+ZF+CF)
            0x9e,       // sahf
        ];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        run(&mut cpu, &mut m, 2);
        assert!(cpu.flag(FLAG_SF), "sahf SF");
        assert!(cpu.flag(FLAG_ZF), "sahf ZF");
        assert!(cpu.flag(FLAG_CF), "sahf CF");
        assert!(!cpu.flag(FLAG_AF), "sahf AF harus bersih");
        assert!(!cpu.flag(FLAG_PF), "sahf PF harus bersih");
    }

    /// E2E boot CD (El Torito): load_boot_image (DL=0xE0) + AH=42 drive CD
    /// membaca dalam blok 2048 (byte offset = lba*2048). Disk rekaman mencatat
    /// read_bytes — cdboot harus membacanya di offset bi_file*2048 = 667*2048.
    #[test]
    fn test_cd_boot_eltorito_ah42_reads_2048_block() {
        use crate::iso::{parse_eltorito, read_boot_image};
        struct RecDisk {
            reads: Vec<u64>,
        }
        impl X86Disk for RecDisk {
            fn read(&mut self, _lba: u64, _count: u16, buf: &mut [u8]) -> Result<(), String> {
                buf.fill(0);
                Ok(())
            }
            fn read_bytes(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), String> {
                self.reads.push(offset);
                buf.fill(0xEE);
                Ok(())
            }
            fn total_sectors(&self) -> u64 {
                0
            }
        }
        let iso = root_of("ubuntu-26.04-desktop-amd64.iso");
        if !std::path::Path::new(&iso).exists() {
            eprintln!("skipped: ISO tidak ada");
            return;
        }
        let mut f = std::fs::File::open(&iso).unwrap();
        let boot = parse_eltorito(&mut f).unwrap();
        assert_eq!(boot.entry.image_lba, 667);
        let image = read_boot_image(&mut f, &boot.entry, 0x10000).unwrap();
        let mut m = mem();
        let mut cpu = X86Cpu::new();
        cpu.disk = Some(Box::new(RecDisk { reads: Vec::new() }));
        cpu.load_boot_image(&mut m, &image, 0xE0).unwrap();
        assert_eq!(cpu.cd_drive, Some(0xE0));
        assert_eq!(cpu.dl(), 0xE0);
        // cdboot step 1-32: inisialisasi + baca bi_file (LBA 667, blok 2048).
        run(&mut cpu, &mut m, 40);
        // Target baca cdboot: esc:bx = (DATA_ADDR-0x200)>>4 : 0 → 0x800:0 = linear 0x8000.
        // Sebaris data 0xEE dari read_bytes → menandakan read_cd_blocks dipakai.
        let b = m.read(0x8000, 1).unwrap();
        assert_eq!(b, 0xEE, "cdboot AH=42 (CD) harus membaca via read_bytes (blok 2048)");
        // Belum halt.
        assert!(!cpu.halted, "cdboot 40 step tanpa fault");
    }

    /// E2E Machine: output INT 10h AH=0E (BIOS teletype) harus muncul di
    /// `MachineResult.console` — sebelumnya X86Cpu.`out` tidak pernah
    /// diekspos lewat `console_output()` → CLI `--boot-iso` selalu
    /// "BIOS console:" kosong walau BIOS sudah mencetak "GRUB ".
    #[test]
    fn test_console_output_machine_propagates() {
        use crate::machine::Machine;
        // loop @0x7c00: lodsb; cmp al,0; je done(0x7c0e); mov ah,0x0e; int 10h; jmp loop
        // done @0x7c0e: hlt. data "Hi!\0" @0x7c11.
        let code = [
            0xbe, 0x11, 0x7c, // 0x7c00: mov si, 0x7c11
            0xac,             // 0x7c03: lodsb
            0x3c, 0x00,       // 0x7c04: cmp al, 0
            0x74, 0x06,       // 0x7c06: je +6 → 0x7c0e
            0xb4, 0x0e,       // 0x7c08: mov ah, 0x0e
            0xcd, 0x10,       // 0x7c0a: int 10h
            0xeb, 0xf1,       // 0x7c0c: jmp -15 → 0x7c00
            0xf4,             // 0x7c0e: hlt (done)
            0x90, 0x90,       // 0x7c0f-0x7c10: pad
            b'H', b'i', b'!', 0x00, // 0x7c11
        ];
        let mut m = mem();
        let mut cpu = load(&mut m, &code);
        cpu.disk = None;
        let mut machine = Machine::new(Box::new(cpu), m, 20);
        let r = machine.run().unwrap();
        assert_eq!(r.console, b"Hi!", "console harus berisi output INT 10h");
        assert!(r.summary().contains("console: [Hi!] (3 bytes)"));
    }

    /// VGA text buffer mirror: write16 ke 0xB8000 ter-capture di `cpu.vga`
    /// dan `vga_text()` mengembalikan baris teks 80x25.
    #[test]
    fn test_vga_text_mirror() {
        let mut m = mem();
        // mov word [ds:0x0000], 0x0748 ('H'); mov word [ds:0x0002], 0x0769 ('i')
        // dengan ds=0xB800 → alamat linear 0xB8000 (region VGA text).
        let code = [
            0xc7, 0x06, 0x00, 0x00, 0x48, 0x07, // mov word [0x0000], 0x0748
            0xc7, 0x06, 0x02, 0x00, 0x69, 0x07, // mov word [0x0002], 0x0769
        ];
        let mut cpu = load(&mut m, &code);
        cpu.ds = 0xB800;
        run(&mut cpu, &mut m, 2);
        assert_eq!(cpu.vga[0], b'H');
        assert_eq!(cpu.vga[1], 0x07);
        assert_eq!(cpu.vga[2], b'i');
        let t = cpu.vga_text();
        assert!(t.starts_with("Hi"), "vga_text harus 'Hi...', dapat: {:?}", t);
        // Memori guest juga menerima tulis (region RAM menutupi 0xB8000).
        assert_eq!(m.read(0xB8000, 2).unwrap(), 0x0748);
    }

    /// INT 13h AH=42 DAP 64-bit LBA: dword tinggi (+12) harus ikut dibaca.
    /// Disk rekaman mencatat LBA yang diminta.
    #[test]
    fn test_int13_dap_64bit_lba() {
        struct RecDisk {
            lba: u64,
            count: u16,
        }
        impl X86Disk for RecDisk {
            fn read(&mut self, lba: u64, count: u16, buf: &mut [u8]) -> Result<(), String> {
                self.lba = lba;
                self.count = count;
                // Tulis LBA yang diminta ke awal buffer agar bisa diverifikasi
                // oleh test (Box<dyn> tak bisa dibaca balik).
                for (i, b) in lba.to_le_bytes().iter().enumerate() {
                    if i < buf.len() {
                        buf[i] = *b;
                    }
                }
                Ok(())
            }
            fn total_sectors(&self) -> u64 {
                0x1_0000_0002
            }
            fn read_bytes(&mut self, _offset: u64, buf: &mut [u8]) -> Result<(), String> {
                buf.fill(0xBB);
                Ok(())
            }
        }
        let mut m = mem();
        // DAP di DS:SI = 0:0x600: lba_lo=2 lba_hi=0x42 → lba = 0x42_0000_0002
        m.write(0x602, 1, 1).unwrap(); // count
        m.write(0x604, 1, 0x00).unwrap(); // offset low = 0
        m.write(0x605, 1, 0x00).unwrap(); // offset high = 0
        m.write(0x606, 1, 0x00).unwrap(); // seg low
        m.write(0x607, 1, 0x10).unwrap(); // seg = 0x1000 (buffer @ 0x10000)
        m.write(0x608, 1, 2).unwrap(); // lba lo (dword 1)
        m.write(0x60c, 1, 0x42).unwrap(); // lba hi (dword 2)
        let mut cpu = load(&mut m, &[0xcd, 0x13]); // int 13h
        cpu.disk = Some(Box::new(RecDisk { lba: 0, count: 0 }));
        cpu.ds = 0;
        cpu.si_set(0x600);
        cpu.r8h_set(0, 0x42);
        run(&mut cpu, &mut m, 1);
        assert!(!cpu.cf(), "INT 13h AH=42 harus sukses");
        // Buffer 0x1000:0 berisi LBA 64-bit yang diminta disk (0x42_0000_0002).
        let lba = m.read(0x10000, 8).unwrap();
        assert_eq!(lba, 0x42_0000_0002, "DAP LBA 64-bit harus ikut dword tinggi");
        let _ = cpu.disk.as_ref().unwrap().total_sectors();
    }
}
