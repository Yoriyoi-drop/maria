//! Probe: test `83 E0 00` (and eax, imm8) + `8b 45 fc` etc in isolation.
use maria_core::intern::Symbol;
use maria_emu::cpu::x86::X86Cpu;
use maria_emu::cpu::CpuCore;
use maria_emu::mem::{MemoryMap, MemoryPort, RamRegion, RegionKind};

fn main() {
    let mut m = MemoryMap::new();
    m.add(RamRegion::new(Symbol::intern("ram"), 0x0, 0x100000, RegionKind::Ram, false).unwrap())
        .unwrap();
    // Code at 0x1000: mov eax, 1 ; and eax, 0 ; mov [0x2000], eax ; hlt
    let code = [
        0xb8u8, 0x01, 0x00, 0x00, 0x00, // mov eax, 1
        0x83, 0xe0, 0x00, // and eax, 0
        0xa3, 0x00, 0x20, 0x00, 0x00, // mov [0x2000], eax
        0xf4, // hlt
    ];
    for (i, b) in code.iter().enumerate() {
        m.write(0x1000 + i as u64, 1, *b as u64).unwrap();
    }
    let mut cpu = X86Cpu::new();
    cpu.pmode = true; // 32-bit mode
                      // Flat GDT: entry 0 (selector 0x8) = base 0, limit 4G.
    let mut gdt = vec![0u8; 16];
    gdt[0] = 0xff;
    gdt[1] = 0xff;
    gdt[2] = 0x00;
    gdt[3] = 0x00; // seg0 base 0
    gdt[4] = 0x00;
    gdt[5] = 0x9a;
    gdt[6] = 0xcf;
    gdt[7] = 0x00;
    gdt[8] = 0xff;
    gdt[9] = 0xff;
    gdt[10] = 0x00;
    gdt[11] = 0x00; // seg1 base 0
    gdt[12] = 0x00;
    gdt[13] = 0x92;
    gdt[14] = 0xcf;
    gdt[15] = 0x00;
    cpu.gdt_cache = gdt;
    cpu.cs = 0x8;
    cpu.ip = 0x1000;
    for _ in 0..10 {
        if cpu.halted {
            break;
        }
        let pc = cpu.pc();
        let _ = cpu.step(&mut m);
        println!(
            "step pc=0x{pc:08x} eax=0x{:08x} halted={}",
            cpu.r32(0),
            cpu.halted
        );
    }
    println!(
        "mem[0x2000] = 0x{:08x} (expect 0)",
        m.read(0x2000, 4).unwrap_or(0xffff)
    );
}
