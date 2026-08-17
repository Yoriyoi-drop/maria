//! Probe: trace early boot (steps 0-600) to find derail after the fix.
use maria_emu::cpu::x86::{FileDisk, X86Cpu};
use maria_emu::cpu::CpuCore;
use maria_emu::mem::{MemoryMap, MemoryPort, RamRegion, RegionKind};
use maria_core::intern::Symbol;

fn main() {
    let mut m = MemoryMap::new();
    m.add(RamRegion::new(Symbol::intern("ram"), 0x0, 0x400_0000, RegionKind::Ram, false).unwrap()).unwrap();
    let iso = std::env::args().nth(1).expect("iso path");
    let mut f = std::fs::File::open(&iso).expect("open iso");
    use std::io::Read;
    let mut mbr = [0u8; 512];
    f.read_exact(&mut mbr).expect("mbr");
    let mut cpu = X86Cpu::new();
    cpu.load_boot_sector(&mut m, &mbr).unwrap();
    cpu.disk = Some(Box::new(FileDisk::open(&iso).unwrap()));
    let mut n = 0;
    let mut last = 0u64;
    for _ in 0..800 {
        if cpu.halted { break; }
        let pc = cpu.pc();
        let _ = cpu.step(&mut m);
        n += 1;
        // print at interesting points: every step 0..120, then every 40
        if n <= 120 || n % 40 == 0 {
            println!("[{n}] pc=0x{pc:08x} eax=0x{:08x} ebx=0x{:08x} ecx=0x{:08x} edx=0x{:08x} esi=0x{:08x} edi=0x{:08x} esp=0x{:08x}",
                cpu.r32(0), cpu.r32(3), cpu.r32(1), cpu.r32(2), cpu.r32(6), cpu.r32(7), cpu.r32(4));
        }
        if n > 120 && pc == last {
            println!("[{n}] pc stuck at 0x{pc:08x}");
            break;
        }
        last = pc;
    }
    println!("HALT: {} steps={}", cpu.halt_reason, n);
}
