//! Probe: trace literal-base computation 0x8b0f..0x8b2c.
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
    let mut in_region = false;
    let mut region_steps = 0;
    for _ in 0..120000 {
        if cpu.halted { break; }
        let pc = cpu.pc();
        let _ = cpu.step(&mut m);
        n += 1;
        // after 2nd literal write, capture region 0x8b0f..0x8b2d
        if pc == 0x8aa3 {
            let al = cpu.r32(0) & 0xff;
            if al == 0xff && cpu.r32(7) == 0x100001 {
                in_region = true;
                println!("--- after 2nd WRITE (al=0xff), next region exec:");
            }
        }
        if in_region && pc >= 0x8b0f && pc <= 0x8b2d {
            let ebp = cpu.r32(5);
            let now = m.read(ebp.wrapping_sub(4) as u64, 4).unwrap_or(0);
            let prev = m.read(ebp.wrapping_sub(8) as u64, 4).unwrap_or(0);
            println!("region pc=0x{pc:04x} eax=0x{:08x} edx=0x{:08x} ecx=0x{:08x} ebx=0x{:08x} [ebp-4]={:08x} [ebp-8]={:08x}",
                cpu.r32(0), cpu.r32(2), cpu.r32(1), cpu.r32(3), now, prev);
            region_steps += 1;
            if region_steps > 14 { in_region = false; }
        }
        if pc > 0x8b2d && in_region { in_region = false; }
    }
    println!("done steps={}", n);
}
