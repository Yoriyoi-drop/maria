//! Probe: trace every step 195..310 to see the call into LzmaDecode.
use maria_core::intern::Symbol;
use maria_emu::cpu::x86::{FileDisk, X86Cpu};
use maria_emu::cpu::CpuCore;
use maria_emu::mem::{MemoryMap, MemoryPort, RamRegion, RegionKind};

fn main() {
    let mut m = MemoryMap::new();
    m.add(
        RamRegion::new(
            Symbol::intern("ram"),
            0x0,
            0x400_0000,
            RegionKind::Ram,
            false,
        )
        .unwrap(),
    )
    .unwrap();
    let iso = std::env::args().nth(1).expect("iso path");
    let mut f = std::fs::File::open(&iso).expect("open iso");
    use std::io::Read;
    let mut mbr = [0u8; 512];
    f.read_exact(&mut mbr).expect("mbr");
    let mut cpu = X86Cpu::new();
    cpu.load_boot_sector(&mut m, &mbr).unwrap();
    cpu.disk = Some(Box::new(FileDisk::open(&iso).unwrap()));
    let mut n = 0;
    for _ in 0..320 {
        if cpu.halted {
            break;
        }
        let pc = cpu.pc();
        let _ = cpu.step(&mut m);
        n += 1;
        if n >= 195 && n <= 310 {
            println!("[{n}] pc=0x{pc:08x} eax=0x{:08x} ebx=0x{:08x} ecx=0x{:08x} edx=0x{:08x} esi=0x{:08x} edi=0x{:08x} esp=0x{:08x}",
                cpu.r32(0), cpu.r32(3), cpu.r32(1), cpu.r32(2), cpu.r32(6), cpu.r32(7), cpu.r32(4));
        }
    }
    println!("HALT: {} steps={}", cpu.halt_reason, n);
}
