// Sementara: tulis ELF bare-metal test ke path argumen untuk verifikasi CLI.
use maria_core::intern::Symbol;
use maria_emu::mem::{MemoryMap, MemoryPort, RamRegion, RegionKind};

fn make_elf(payload: &[u8]) -> Vec<u8> {
    let entry = 0x8000_0000u64;
    let vaddr = 0x8000_0000u64;
    let mut d = vec![0u8; 64 + 56];
    d[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
    d[4] = 2;
    d[5] = 1;
    d[6] = 1;
    d[18..20].copy_from_slice(&243u16.to_le_bytes());
    d[24..32].copy_from_slice(&entry.to_le_bytes());
    d[32..40].copy_from_slice(&64u64.to_le_bytes());
    d[54..56].copy_from_slice(&56u16.to_le_bytes());
    d[56..58].copy_from_slice(&1u16.to_le_bytes());
    let po = 64usize;
    d[po..po + 4].copy_from_slice(&1u32.to_le_bytes());
    d[po + 8..po + 16].copy_from_slice(&(64u64 + 56).to_le_bytes());
    d[po + 16..po + 24].copy_from_slice(&vaddr.to_le_bytes());
    d[po + 32..po + 40].copy_from_slice(&(payload.len() as u64).to_le_bytes());
    d[po + 40..po + 48].copy_from_slice(&(payload.len() as u64).to_le_bytes());
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
    maria_emu::elf::load_elf(&elf, &mut mem).expect("load");
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
