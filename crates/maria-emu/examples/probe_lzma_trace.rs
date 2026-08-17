//! Probe: trace GRUB LZMA bit decoder (0x89e5) state vs Python reference.
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
    // Run to the LZMA entry quickly (decompressor reached ~step 180-270).
    let mut n = 0;
    let mut bit_calls = 0u32;
    let mut writes = 0u32;
    for _ in 0..120000 {
        if cpu.halted { break; }
        let pc = cpu.pc();
        let _ = cpu.step(&mut m);
        n += 1;
        if pc == 0x89e5 {
            // bit decoder entry: eax = prob idx, [ebp-0xc]=range, [ebp-0x10]=code
            let ebp = cpu.r32(5);
            let rng = m.read(ebp.wrapping_sub(0xc) as u64, 4).unwrap_or(0);
            let code = m.read(ebp.wrapping_sub(0x10) as u64, 4).unwrap_or(0);
            if bit_calls < 24 {
                println!("BIT {:4} idx=0x{:04x} range={:08x} code={:08x} eax=0x{:08x}",
                    bit_calls, cpu.r32(0), rng, code, cpu.r32(0));
            }
            bit_calls += 1;
        }
        if pc == 0x8aa3 {
            // output write helper: al = byte, [ebp-0x8] = prev
            let al = cpu.r32(0) & 0xff;
            if writes < 12 {
                println!("WRITE {:3} al=0x{:02x} edi=0x{:08x}", writes, al, cpu.r32(7));
            }
            writes += 1;
        }
    }
    println!("done steps={} bit_calls={} writes={} halt={:?}", n, bit_calls, writes, cpu.halt_reason);
    let mut s = String::new();
    for a in 0x100000u64..0x100010 {
        s.push_str(&format!("{:02x} ", m.read(a, 1).unwrap_or(0)));
    }
    println!("out100000: {s}");
}
