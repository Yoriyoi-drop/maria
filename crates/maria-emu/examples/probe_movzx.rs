//! Probe: panjang instruksi movzx (66 0f b6 c6) — ip harus 0x7c05 setelah 1 step.
use maria_emu::cpu::x86::X86Cpu;
use maria_emu::cpu::CpuCore;
use maria_emu::mem::{MemoryMap, MemoryPort, RamRegion, RegionKind};
use maria_core::intern::Symbol;

fn main() {
    let mut m = MemoryMap::new();
    m.add(RamRegion::new(Symbol::intern("ram"), 0x0, 0x10_0000, RegionKind::Ram, false).unwrap()).unwrap();
    // 66 0f b6 c6 88 64 ff: movzx eax, dh; mov [bp-1], ah
    let code = [0x66, 0x0f, 0xb6, 0xc6, 0x88, 0x64, 0xff];
    let mut cpu = X86Cpu::new();
    cpu.load_boot_sector(&mut m, &code).unwrap();
    let _ = cpu.step(&mut m);
    println!("ip=0x{:04x} (harus 0x7c05 utk 4 byte) eax=0x{:08x}", cpu.ip, cpu.r32(0));
    let _ = cpu.step(&mut m);
    println!("ip=0x{:04x} (harus 0x7c08 utk 3 byte) mem[bp-1]=0x{:02x}", cpu.ip, m.read(0x7c00 - 1, 1).unwrap_or(0));
}
