// probe: lacak transisi pasca-dekompresi (step 8.5M+) — cari lompatan ke region zero.
use maria_core::intern::Symbol;
use maria_emu::cpu::x86::X86Cpu;
use maria_emu::cpu::CpuCore;
use maria_emu::iso::{parse_eltorito, read_boot_image};
use maria_emu::mem::{MemoryMap, MemoryPort, RamRegion, RegionKind};

fn main() {
    let iso = std::env::var("ISO").unwrap_or_else(|_| "ubuntu-26.04-desktop-amd64.iso".into());
    let mk = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8_500_000);
    let extra: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(400_000);
    let mut f = std::fs::File::open(&iso).unwrap();
    let boot = parse_eltorito(&mut f).unwrap();
    let image = read_boot_image(&mut f, &boot.entry, 0x10000).unwrap();
    let mut mem = MemoryMap::new();
    mem.add(RamRegion::new(Symbol::intern("ram"), 0, 0x80000000, RegionKind::Ram, true).unwrap())
        .unwrap();
    let mut cpu = X86Cpu::new();
    cpu.disk = Some(Box::new(maria_emu::cpu::x86::FileDisk::open(&iso).unwrap()));
    cpu.load_boot_image(&mut mem, &image, 0xE0).unwrap();

    for _ in 0..mk {
        if cpu.halted {
            println!("HALT {:?}", cpu.halt_reason);
            return;
        }
        cpu.step(&mut mem).unwrap();
    }
    println!("== awal window @{} ==", mk);
    // Window: catat pc + instruksi; tandai saat pc pindah ke > 0x100000 (tinggi).
    let mut hi = 0u64;
    let mut ring = [0u64; 6];
    let mut in_zero = false;
    for i in 0..extra {
        if cpu.halted {
            println!("HALT {:?}", cpu.halt_reason);
            return;
        }
        let pc = cpu.pc();
        let hi_now = pc >= 0x100000;
        if hi_now && hi == 0 {
            hi = pc;
        }
        let b0 = mem.read(pc, 1).unwrap_or(0);
        let b1 = mem.read(pc + 1, 1).unwrap_or(0);
        let b2 = mem.read(pc + 2, 1).unwrap_or(0);
        let b3 = mem.read(pc + 3, 1).unwrap_or(0);
        // Log: selalu di 100 step pertama; lalu tiap 1000; lalu saat pertama kali tinggi.
        let log = i < 100 || i % 1000 == 0;
        if hi_now && hi == pc {
            println!(
                "  [BIG-JUMP] step={} pc=0x{:08x} [0x{:02x} 0x{:02x} 0x{:02x} 0x{:02x}]",
                mk + i,
                pc,
                b0,
                b1,
                b2,
                b3
            );
        }
        // Deteksi pertama kali byte pc = 0 (masuk region nol); cetak konteks.
        let zero_now = mem.read(pc, 1).map(|v| v == 0).unwrap_or(true);
        if zero_now && !in_zero {
            in_zero = true;
            println!("  ==MASUK-ZERO== step={} pc=0x{:08x}", mk + i, pc);
            for (j, p) in ring.iter().enumerate() {
                let bb = mem.read(*p, 1).unwrap_or(0);
                let bb1 = mem.read(*p + 1, 1).unwrap_or(0);
                let bb2 = mem.read(*p + 2, 1).unwrap_or(0);
                println!(
                    "      [{j}] step={} pc=0x{:08x} [0x{:02x} 0x{:02x} 0x{:02x}]",
                    mk + i - ring.len() + j,
                    p,
                    bb,
                    bb1,
                    bb2
                );
            }
        }
        ring[(i % 6) as usize] = pc;
        if log {
            println!(
                "  step={} pc=0x{:08x} [0x{:02x} 0x{:02x} 0x{:02x} 0x{:02x}]",
                mk + i,
                pc,
                b0,
                b1,
                b2,
                b3
            );
        }
        let _ = (b0, b1, b2, b3);
        cpu.step(&mut mem).unwrap();
    }
    println!("== akhir ==");
}
