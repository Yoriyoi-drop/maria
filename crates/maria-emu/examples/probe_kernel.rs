//! Probe: print the last 40 steps before halt.
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
    let mut ring: Vec<(u64, u32, u32, u32, u32, u32, u32, u32)> = Vec::new();
    let mut halt_pc = 0u64;
    let mut last = 0u64;
    let mut in_loop = false;
    let mut loop_count = 0;
    for _ in 0..40_000_000 {
        if cpu.halted { break; }
        let pc = cpu.pc();
        halt_pc = pc;
        if (0xc7d8..=0xc7f0).contains(&pc) && loop_count < 40 {
            println!("LOOP2 step={n} pc=0x{pc:x} eax=0x{:08x} ebx=0x{:08x} ecx=0x{:08x} edx=0x{:08x} esi=0x{:08x} edi=0x{:08x} ebp=0x{:08x} esp=0x{:08x}", cpu.r32(0), cpu.r32(3), cpu.r32(1), cpu.r32(2), cpu.r32(6), cpu.r32(7), cpu.r32(5), cpu.r32(4));
            loop_count += 1;
        }
        if in_loop && loop_count < 12 {
            let mut s = String::new();
            for off in 0..16usize {
                s.push_str(&format!("{:02x} ", m.read((pc as usize + off) as u64, 1).unwrap_or(0)));
            }
            println!("LOOP step={n} pc=0x{pc:x} bytes=[{s}] eax=0x{:08x} ebx=0x{:08x} ecx=0x{:08x} edx=0x{:08x} esi=0x{:08x} edi=0x{:08x} ebp=0x{:08x} esp=0x{:08x}", cpu.r32(0), cpu.r32(3), cpu.r32(1), cpu.r32(2), cpu.r32(6), cpu.r32(7), cpu.r32(5), cpu.r32(4));
            loop_count += 1;
        }
        if !in_loop && pc == 0xbdc9 && last == 0xbdd3 {
            in_loop = true;
            println!("=== LOOP DETECTED at step={n} ===");
        }
        let _ = cpu.step(&mut m);
        n += 1;
        if n % 500 == 0 { println!("step={n} pc=0x{pc:x} eax=0x{:08x} esp=0x{:08x}", cpu.r32(0), cpu.r32(4)); }
        last = pc;
        ring.push((pc, cpu.r32(0), cpu.r32(3), cpu.r32(1), cpu.r32(2), cpu.r32(6), cpu.r32(7), cpu.r32(4)));
        if ring.len() > 40 { ring.remove(0); }
    }
    println!("HALT: {} steps={} halt_pc=0x{halt_pc:x}", cpu.halt_reason, n);
    let a = 0xc7d0usize;
    let mut s = String::new();
    for off in 0..80usize {
        s.push_str(&format!("{:02x} ", m.read((a + off) as u64, 1).unwrap_or(0)));
        if (off + 1) % 16 == 0 { s.push('\n'); }
    }
    println!("mem@0xc7d0:\n{s}");
    for (pc, eax, ebx, ecx, edx, esi, edi, esp) in &ring {
        println!("pc=0x{pc:08x} eax=0x{eax:08x} ebx=0x{ebx:08x} ecx=0x{ecx:08x} edx=0x{edx:08x} esi=0x{esi:08x} edi=0x{edi:08x} esp=0x{esp:08x}");
    }
}
