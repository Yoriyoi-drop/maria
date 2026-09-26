// Sementara: tulis ELF bare-metal test ke path argumen untuk verifikasi CLI.
use mivon_core::intern::Symbol;
use mivon_emu::mem::{MemoryMap, MemoryPort, RamRegion, RegionKind};

/// ELF32 little-endian, e_machine = EM_RISCV (243), satu PT_LOAD —
/// format yang dimuat loader & dijalankan interpreter RV32 (`--run`).
fn make_elf(payload: &[u8]) -> Vec<u8> {
    let entry = 0x8000_0000u32;
    let vaddr = 0x8000_0000u32;
    let ehsize = 52usize;
    let phentsize = 32usize;
    let phoff = ehsize;
    let offset = (ehsize + phentsize) as u32;
    let mut d = vec![0u8; ehsize + phentsize];
    // ELF32 header
    d[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
    d[4] = 1; // ELFCLASS32
    d[5] = 1; // ELFDATA2LSB
    d[6] = 1; // EV_CURRENT
    d[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
    d[18..20].copy_from_slice(&243u16.to_le_bytes()); // EM_RISCV
    d[20..24].copy_from_slice(&1u32.to_le_bytes()); // e_version
    d[24..28].copy_from_slice(&entry.to_le_bytes()); // e_entry
    d[28..32].copy_from_slice(&(phoff as u32).to_le_bytes()); // e_phoff
    d[40..42].copy_from_slice(&(ehsize as u16).to_le_bytes()); // e_ehsize
    d[42..44].copy_from_slice(&(phentsize as u16).to_le_bytes()); // e_phentsize
    d[44..46].copy_from_slice(&1u16.to_le_bytes()); // e_phnum
                                                    // Program header PT_LOAD
    d[phoff..phoff + 4].copy_from_slice(&1u32.to_le_bytes()); // p_type
    d[phoff + 4..phoff + 8].copy_from_slice(&offset.to_le_bytes()); // p_offset
    d[phoff + 8..phoff + 12].copy_from_slice(&vaddr.to_le_bytes()); // p_vaddr
    d[phoff + 12..phoff + 16].copy_from_slice(&vaddr.to_le_bytes()); // p_paddr
    d[phoff + 16..phoff + 20].copy_from_slice(&(payload.len() as u32).to_le_bytes()); // p_filesz
    d[phoff + 20..phoff + 24].copy_from_slice(&(payload.len() as u32).to_le_bytes()); // p_memsz
    d[phoff + 24..phoff + 28].copy_from_slice(&7u32.to_le_bytes()); // p_flags RWX
    d[phoff + 28..phoff + 32].copy_from_slice(&4u32.to_le_bytes()); // p_align
    d.extend_from_slice(payload);
    d
}

fn main() {
    let out = std::env::args().nth(1).expect("arg: out.elf");
    // t0=42; a0=0x80000000; sw t0,0(a0); ebreak
    // Perbaiki sw: rs2 = 5 (t0), rs1 = 10 (a0), f3=2 (sw), op=0x23
    let sw = (5u32 << 20) | (10u32 << 15) | (2u32 << 12) | 0x23;
    let code = [
        (42u32 << 20) | (5u32 << 7) | 0x13,       // addi t0, zero, 42
        (0x80000u32 << 12) | (10u32 << 7) | 0x37, // lui a0, 0x80000
        sw,                                       // sw t0, 0(a0)
        0x0010_0073,                              // ebreak
    ];
    let elf = make_elf(&code_bytes(&code));
    std::fs::write(&out, &elf).expect("write elf");
    // Verifikasi: load ke RAM map dan baca kembali
    let mut mem = MemoryMap::new();
    mem.add(
        RamRegion::new(
            Symbol::intern("ram"),
            0x8000_0000,
            0x10000,
            RegionKind::Ram,
            false,
        )
        .unwrap(),
    )
    .unwrap();
    mivon_emu::elf::load_elf(&elf, &mut mem).expect("load");
    let stored = mem.read(0x8000_0000, 4).unwrap_or(0);
    println!(
        "ELF {} entry=0x80000000 first_word=0x{:08x} sw=0x{:08x}",
        out, stored, sw
    );
}

fn code_bytes(words: &[u32]) -> Vec<u8> {
    let mut v = Vec::with_capacity(words.len() * 4);
    for w in words {
        v.extend_from_slice(&w.to_le_bytes());
    }
    v
}
