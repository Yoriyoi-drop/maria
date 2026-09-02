// probe: boot CD cepat — tanpa histogram; dump kode guest di akhir + INT 13h count.
use maria_emu::cpu::x86::X86Cpu;
use maria_emu::cpu::CpuCore;
use maria_emu::iso::{parse_eltorito, read_boot_image};
use maria_emu::mem::{MemoryMap, MemoryPort, RamRegion, RegionKind};
use maria_core::intern::Symbol;

fn main() {
    let iso = std::env::var("ISO").unwrap_or_else(|_| "ubuntu-26.04-desktop-amd64.iso".into());
    let n: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(100000);
    let mut f = std::fs::File::open(&iso).unwrap();
    let boot = parse_eltorito(&mut f).unwrap();
    let image = read_boot_image(&mut f, &boot.entry, 0x10000).unwrap();
    let mut mem = MemoryMap::new();
    mem.add(RamRegion::new(Symbol::intern("ram"), 0, 0x80000000, RegionKind::Ram, true).unwrap()).unwrap();
    let mut cpu = X86Cpu::new();
    cpu.disk = Some(Box::new(maria_emu::cpu::x86::FileDisk::open(&iso).unwrap()));
    cpu.load_boot_image(&mut mem, &image, 0xE0).unwrap();
    let mut int13 = 0u64;
    for i in 0..n {
        if cpu.halted {
            println!("HALT {:?} at step {}", cpu.halt_reason, i);
            break;
        }
        let pc = cpu.pc();
        let b0 = mem.read(pc, 1).unwrap_or(0);
        let b1 = mem.read(pc + 1, 1).unwrap_or(0);
        if b0 == 0xcd && b1 == 0x13 {
            int13 += 1;
        }
        cpu.step(&mut mem).unwrap();
    }
    println!("final: steps={} pc=0x{:x} cs=0x{:x} pmode={} int13={} out={:?}",
        n, cpu.pc(), cpu.cs, cpu.pmode, int13, String::from_utf8_lossy(&cpu.out));
    // Dump region busur 0x8f00-0x9800 ke file /tmp/opencode/lowregion.bin
    {
        let mut v = Vec::with_capacity(0x9800 - 0x8f00);
        for k in 0x8f00u64..0x9800 {
            v.push(mem.read(k, 1).unwrap_or(0) as u8);
        }
        let _ = std::fs::write("/tmp/opencode/lowregion.bin", &v);
        println!("low-region 0x8f00-0x9800 disimpan");
    }
    if cpu.pmode {
        // Replikasi gdt_seg_base(cs) dari cache publik (base = byte 2-4 + 7 GDT).
        let sel = cpu.cs;
        let idx = (sel >> 3) as usize;
        let off = idx * 8;
        let base = if off + 7 < cpu.gdt_cache.len() {
            let g = &cpu.gdt_cache[off..off + 8];
            (g[2] as u32) | ((g[3] as u32) << 8) | ((g[4] as u32) << 16) | ((g[7] as u32) << 24)
        } else {
            0
        };
        let linear = (base as u64 + cpu.ip as u64) & 0xffff_ffff;
        println!("gdt_base(sel={:#x})={:#010x} gdt_base_reg={:#010x} linear=0x{:08x}",
            sel, base, cpu.gdt_base, linear);
        print!("code @ linear-64: ");
        for k in 0..160u64 {
            print!("{:02x} ", mem.read((linear as i64 - 64).max(0) as u64 + k, 1).unwrap_or(0));
        }
        println!();
        // Bilangan bukan-nol di sekitar linear?
        let mut nonzero = 0u64;
        let lb = (linear as i64 - 0x8000).max(0) as u64;
        for k in 0..0x10000u64 {
            if mem.read(lb + k, 1).ok().map(|v| v != 0).unwrap_or(false) {
                nonzero += 1;
            }
        }
        println!("nonzero bytes in [linear-32K, linear+32K): {}", nonzero);
    }
}