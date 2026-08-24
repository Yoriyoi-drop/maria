//! Probe: trace 0x8b12 (and eax,0) and 0x8b25 (mul edx) executions.
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
    let mut c_and = 0u32;
    let mut c_mul = 0u32;
    for _ in 0..120000 {
        if cpu.halted {
            break;
        }
        let pc = cpu.pc();
        let _ = cpu.step(&mut m);
        n += 1;
        if pc == 0x8b12 && c_and < 10 {
            // AFTER executing and eax,0: eax should be 0
            let ebp = cpu.r32(5);
            let now = m.read(ebp.wrapping_sub(4) as u64, 4).unwrap_or(0);
            println!(
                "AND@ {:4} eax=0x{:08x} [ebp-4]={:08x} [ebp-8]={:08x}",
                c_and,
                cpu.r32(0),
                now,
                m.read(ebp.wrapping_sub(8) as u64, 4).unwrap_or(0)
            );
            c_and += 1;
        }
        if pc == 0x8b25 && c_mul < 10 {
            // AFTER mul: eax = context*0x300 (low), edx = high
            println!(
                "MUL@ {:4} eax=0x{:08x} edx=0x{:08x}",
                c_mul,
                cpu.r32(0),
                cpu.r32(2)
            );
            c_mul += 1;
        }
    }
    println!("done steps={}", n);
}
