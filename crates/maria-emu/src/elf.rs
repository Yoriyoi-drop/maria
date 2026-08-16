//! Parser ELF minimal (ELF32/ELF64, little-endian) + loader — EMULATOR.md §12.
//!
//! Cukup untuk boot: ident, entry point, dan segment `PT_LOAD` (kernel/bare-
//! metal). Segmen lain (PT_DYNAMIC, PT_INTERP, dll) diabaikan. Endianness
//! big-endian (ELFCLASS BE) ditolak dengan error jelas.

use crate::mem::{AccessFault, MemoryPort};

pub const PT_LOAD: u32 = 1;
const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const ELFCLASS32: u8 = 1;
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;

/// Header ELF (field yang dibutuhkan loader).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfHeader {
    pub class: u8, // 1=ELF32, 2=ELF64
    pub machine: u16,
    pub entry: u64,
    pub phoff: u64,
    pub phentsize: u16,
    pub phnum: u16,
}

/// Satu program header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phdr {
    pub p_type: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_flags: u32,
}

fn rd_u16(d: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([d[off], d[off + 1]])
}

fn rd_u32(d: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([d[off], d[off + 1], d[off + 2], d[off + 3]])
}

fn rd_u64(d: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([d[off], d[off + 1], d[off + 2], d[off + 3], d[off + 4], d[off + 5], d[off + 6], d[off + 7]])
}

fn err(msg: impl Into<String>) -> String {
    msg.into()
}

/// Parse header + program headers ELF.
pub fn parse_elf(data: &[u8]) -> Result<(ElfHeader, Vec<Phdr>), String> {
    if data.len() < 16 {
        return Err(err("data terlalu pendek untuk header ELF"));
    }
    if data[0..4] != ELF_MAGIC {
        return Err(err("bukan file ELF (magic salah)"));
    }
    let class = data[4];
    let data_enc = data[5];
    if data_enc != ELFDATA2LSB {
        return Err(err(format!("ELF big-endian tidak didukung (data encoding {})", data_enc)));
    }
    let machine = rd_u16(data, 18);
    let (entry, phoff, phentsize, phnum) = match class {
        ELFCLASS64 => {
            if data.len() < 64 {
                return Err(err("ELF64: header kepotong"));
            }
            (rd_u64(data, 24), rd_u64(data, 32), rd_u16(data, 54), rd_u16(data, 56))
        }
        ELFCLASS32 => {
            if data.len() < 52 {
                return Err(err("ELF32: header kepotong"));
            }
            (rd_u32(data, 24) as u64, rd_u32(data, 28) as u64, rd_u16(data, 42), rd_u16(data, 44))
        }
        _ => return Err(err(format!("class ELF tidak dikenal ({})", class))),
    };
    let header = ElfHeader { class, machine, entry, phoff, phentsize, phnum };

    let mut phdrs = Vec::new();
    for i in 0..phnum as usize {
        let off = phoff as usize + i * phentsize as usize;
        let ph = match class {
            ELFCLASS64 => {
                if off + 56 > data.len() {
                    return Err(err(format!("program header {} di luar file", i)));
                }
                Phdr {
                    p_type: rd_u32(data, off),
                    p_flags: rd_u32(data, off + 4),
                    p_offset: rd_u64(data, off + 8),
                    p_vaddr: rd_u64(data, off + 16),
                    p_filesz: rd_u64(data, off + 32),
                    p_memsz: rd_u64(data, off + 40),
                }
            }
            ELFCLASS32 => {
                if off + 32 > data.len() {
                    return Err(err(format!("program header {} di luar file", i)));
                }
                Phdr {
                    p_type: rd_u32(data, off),
                    p_offset: rd_u32(data, off + 4) as u64,
                    p_vaddr: rd_u32(data, off + 8) as u64,
                    p_filesz: rd_u32(data, off + 16) as u64,
                    p_memsz: rd_u32(data, off + 20) as u64,
                    p_flags: rd_u32(data, off + 24),
                }
            }
            _ => unreachable!(),
        };
        phdrs.push(ph);
    }
    Ok((header, phdrs))
}

/// Load semua segmen `PT_LOAD` ke `mem`. Kembalikan entry point.
pub fn load_elf(data: &[u8], mem: &mut dyn MemoryPort) -> Result<u64, String> {
    let (header, phdrs) = parse_elf(data)?;
    for ph in &phdrs {
        if ph.p_type != PT_LOAD {
            continue;
        }
        let file_end = (ph.p_offset + ph.p_filesz) as usize;
        if file_end > data.len() {
            return Err(err(format!(
                "PT_LOAD filesz 0x{:x} melebihi file (offset 0x{:x})",
                ph.p_filesz, ph.p_offset
            )));
        }
        // Isi filesz dari file, lalu zero-fill memsz - filesz (bss).
        let payload = &data[ph.p_offset as usize..file_end];
        let mut buf = payload.to_vec();
        if ph.p_memsz > ph.p_filesz {
            buf.resize(ph.p_memsz as usize, 0);
        }
        write_checked(mem, ph.p_vaddr, &buf)?;
    }
    Ok(header.entry)
}

/// Tulis byte ke MemoryPort per potongan ≤ 8 byte (trait byte-granularity).
fn write_checked(mem: &mut dyn MemoryPort, addr: u64, data: &[u8]) -> Result<(), String> {
    let mut cur = addr;
    for chunk in data.chunks(8) {
        let mut v = 0u64;
        for (i, b) in chunk.iter().enumerate() {
            v |= (*b as u64) << (8 * i);
        }
        mem.write(cur, chunk.len() as u8, v)
            .map_err(|e: AccessFault| e.to_string())?;
        cur += chunk.len() as u64;
    }
    Ok(())
}

/// Baca isi memori (untuk verifikasi loader di test).
#[cfg(test)]
fn read_mem(mem: &dyn MemoryPort, addr: u64, len: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut cur = addr;
    while out.len() < len {
        let n = (len - out.len()).min(8) as u8;
        let v = mem.read(cur, n).expect("read");
        for i in 0..n {
            out.push(((v >> (8 * i)) & 0xff) as u8);
        }
        cur += n as u64;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::{MemoryMap, RamRegion, RegionKind};
    use maria_core::intern::Symbol;

    fn map() -> MemoryMap {
        let mut m = MemoryMap::new();
        m.add(RamRegion::new(Symbol::intern("ram"), 0x8000_0000, 0x1_0000, RegionKind::Ram, false).unwrap()).unwrap();
        m
    }

    /// Bangun ELF64 LE minimal: 1 segmen PT_LOAD.
    fn make_elf64(entry: u64, vaddr: u64, payload: &[u8], memsz_extra: usize) -> Vec<u8> {
        let mut d = vec![0u8; 64 + 56];
        d[0..4].copy_from_slice(&ELF_MAGIC);
        d[4] = ELFCLASS64;
        d[5] = ELFDATA2LSB;
        d[6] = 1; // version
        d[18..20].copy_from_slice(&243u16.to_le_bytes()); // e_machine = RISC-V
        d[24..32].copy_from_slice(&entry.to_le_bytes());
        d[32..40].copy_from_slice(&64u64.to_le_bytes()); // e_phoff
        d[54..56].copy_from_slice(&56u16.to_le_bytes()); // e_phentsize
        d[56..58].copy_from_slice(&1u16.to_le_bytes()); // e_phnum
        // PT_LOAD
        let po = 64usize;
        d[po..po + 4].copy_from_slice(&PT_LOAD.to_le_bytes());
        d[po + 8..po + 16].copy_from_slice(&(64u64 + 56).to_le_bytes()); // p_offset (setelah header)
        d[po + 16..po + 24].copy_from_slice(&vaddr.to_le_bytes());
        d[po + 32..po + 40].copy_from_slice(&(payload.len() as u64).to_le_bytes());
        d[po + 40..po + 48].copy_from_slice(&((payload.len() + memsz_extra) as u64).to_le_bytes());
        d.extend_from_slice(payload);
        d
    }

    #[test]
    fn test_parse_elf64() {
        let d = make_elf64(0x8000_0000, 0x8000_0000, &[0x13, 0x00, 0x00, 0x00], 0);
        let (h, phdrs) = parse_elf(&d).expect("parse");
        assert_eq!(h.class, ELFCLASS64);
        assert_eq!(h.machine, 243); // RISC-V
        assert_eq!(h.entry, 0x8000_0000);
        assert_eq!(phdrs.len(), 1);
        assert_eq!(phdrs[0].p_type, PT_LOAD);
        assert_eq!(phdrs[0].p_vaddr, 0x8000_0000);
        assert_eq!(phdrs[0].p_filesz, 4);
    }

    #[test]
    fn test_reject_non_elf_and_be() {
        assert!(parse_elf(b"not an elf at all....").is_err());
        let mut d = make_elf64(0, 0, &[0], 0);
        d[5] = 2; // big-endian
        assert!(parse_elf(&d).is_err());
    }

    #[test]
    fn test_load_elf_segment_and_bss() {
        let payload = [0x13, 0x01, 0x00, 0x00]; // nop-ish
        let d = make_elf64(0x8000_0000, 0x8000_0000, &payload, 8); // bss 8 byte
        let mut m = map();
        let entry = load_elf(&d, &mut m).expect("load");
        assert_eq!(entry, 0x8000_0000);
        assert_eq!(&read_mem(&m, 0x8000_0000, 4), &payload);
        // bss zero-filled
        assert_eq!(&read_mem(&m, 0x8000_0004, 8), &[0u8; 8]);
    }

    #[test]
    fn test_load_elf_into_ram() {
        let d = make_elf64(0x8000_0100, 0x8000_0100, &[0xaa, 0xbb, 0xcc], 0);
        let mut m = map();
        let entry = load_elf(&d, &mut m).expect("load");
        assert_eq!(entry, 0x8000_0100);
        assert_eq!(&read_mem(&m, 0x8000_0100, 3), &[0xaa, 0xbb, 0xcc]);
    }

    #[test]
    fn test_load_elf_out_of_memory_fails() {
        // vaddr di luar region RAM (tidak ada mapping).
        let d = make_elf64(0x9000_0000, 0x9000_0000, &[1, 2, 3], 0);
        let mut m = map();
        assert!(load_elf(&d, &mut m).is_err(), "PT_LOAD di alamat unmapped");
    }

    #[test]
    fn test_load_elf_truncated_file_fails() {
        let d = make_elf64(0x8000_0000, 0x8000_0000, &[1, 2, 3, 4, 5], 0);
        // Potong payload → filesz melebihi file.
        let truncated = &d[..68];
        let mut m = map();
        assert!(load_elf(truncated, &mut m).is_err());
    }
}
