// probe: boot El Torito CD - pantau fase: INT 13h call summary, PC sampling,
// VGA/console output, hot loop. Jalur: cdboot → LZMA → GRUB kernel → ...
use std::collections::HashMap;
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

    let mut int13: HashMap<u8, u64> = HashMap::new();
    let mut int10n: u64 = 0;
    let mut hist: HashMap<u64, u64> = HashMap::new();
    let sample = n / 40 + 1;
    // Deteksi INT 13h pada fetch (opcode cd 13 di dua byte pertama).
    let mut prog_last: Option<u64> = None;
    for i in 0..n {
        let pc = cpu.pc();
        *hist.entry(pc).or_insert(0) += 1;
        if cpu.halted {
            println!("HALT {:?} at step {}", cpu.halt_reason, i);
            break;
        }
        let b0 = mem.read(pc, 1).unwrap_or(0);
        let b1 = mem.read(pc + 1, 1).unwrap_or(0);
        if b0 == 0xcd && b1 == 0x13 {
            *int13.entry(cpu.r8h(0)).or_insert(0) += 1;
        } else if b0 == 0xcd && b1 == 0x10 {
            int10n += 1;
        }
        if i % sample == 0 {
            let prev = prog_last.replace(pc);
            let rel = prev.map(|p| pc.wrapping_sub(p) as i64 / sample as i64).unwrap_or(0);
            println!(
                "step={:>9} pc=0x{:08x} delta={:>6} outlen={} vgalen={} cs=0x{:x} pmode={}",
                i, pc, rel, cpu.out.len(), cpu.vga_text().len(), cpu.cs, cpu.pmode
            );
        }
        cpu.step(&mut mem).unwrap();
    }
    println!("final: pc=0x{:x} out={:?} pmode={}", cpu.pc(), String::from_utf8_lossy(&cpu.out), cpu.pmode);
    // Dump kode guest di sekitar pc (flat: linear==ip saat base segmen 0).
    if cpu.pmode {
        let base = (cpu.pc() as i64 - 48).max(0) as u64;
        print!("code @ pc: ");
        for k in 0..96u64 {
            print!("{:02x} ", mem.read(base + k, 1).unwrap_or(0));
        }
        println!();
    }
    println!("INT13 summary: {:?}", int13);
    println!("INT10 count: {}", int10n);
    let txt = cpu.vga_text();
    if !txt.is_empty() {
        println!("VGA text:\n{}", txt);
    }
    let mut v: Vec<(u64, u64)> = hist.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    println!("== top 12 hot PCs ==");
    for (pc, c) in v.iter().take(12) {
        let bs: Vec<u8> = (0..4).map(|k| mem.read(*pc + k, 1).unwrap_or(0) as u8).collect();
        println!("pc=0x{:08x} count={} [{:02x} {:02x} {:02x} {:02x}]", pc, c, bs[0], bs[1], bs[2], bs[3]);
    }
}