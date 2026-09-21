//! Probe: dump kode guest region trampolin GRUB (0x9000-an) setelah boot
//! mencapai jalur biosdisk kedua (crash trampolin). Untuk membandingkan
//! opcode yang dieksekusi emulator vs binari GRUB asli (objdump).
use mivon_core::intern::Symbol;
use mivon_emu::cpu::x86::X86Cpu;
use mivon_emu::cpu::CpuCore;
use mivon_emu::iso::{parse_eltorito, read_boot_image};
use mivon_emu::mem::{MemoryMap, MemoryPort, RamRegion, RegionKind};

fn main() {
    let iso = std::env::var("ISO").unwrap_or_else(|_| "ubuntu-26.04.1-desktop-amd64.iso".into());
    let mut f = std::fs::File::open(&iso).unwrap();
    let boot = parse_eltorito(&mut f).unwrap();
    let image = read_boot_image(&mut f, &boot.entry, 0x10000).unwrap();
    let mut mem = MemoryMap::new();
    mem.add(RamRegion::new(Symbol::intern("ram"), 0, 0x80000000, RegionKind::Ram, true).unwrap())
        .unwrap();
    let mut cpu = X86Cpu::new();
    cpu.disk = Some(Box::new(mivon_emu::cpu::x86::FileDisk::open(&iso).unwrap()));
    cpu.load_boot_image(&mut mem, &image, 0xE0).unwrap();
    for i in 0..16_000_000u64 {
        if cpu.halted {
            println!("HALT {:?} at step {}", cpu.halt_reason, i);
            break;
        }
        cpu.step(&mut mem).unwrap();
    }
    println!(
        "final: steps done pc=0x{:x} cs=0x{:x} pmode={} halted={:?}",
        cpu.pc(),
        cpu.cs,
        cpu.pmode,
        cpu.halted
    );
    // Dump kode di sekitar pc (region kosong / opcode tak dikenal?).
    let mut line = String::new();
    for k in 0..32u64 {
        let b = mem.read(cpu.pc().saturating_add(k), 1).unwrap_or(0xff);
        line.push_str(&format!("{b:02x} "));
    }
    println!("code @pc: {}", line);
    println!("console_bytes: {:02x?}", cpu.out.to_vec());
    // Dump 0x9080..0x9180 (grub_bios_interrupt + prot/real trampolin).
    let mut v = Vec::with_capacity(0x100);
    for k in 0x9080u64..0x9180 {
        v.push(mem.read(k, 1).unwrap_or(0) as u8);
    }
    std::fs::write("/tmp/opencode/grub_tramp.bin", &v).unwrap();
    println!("dump 0x9080..0x9180 -> /tmp/opencode/grub_tramp.bin");
    // Dump variabel handler: [0x9041] (prot_to_real?) & [0x9045] (real_to_prot?).
    for k in 0x9030u64..0x9060 {
        let b = mem.read(k, 1).unwrap_or(0);
        if (k - 0x9030) % 16 == 0 {
            println!("{:04x}: ", k);
        }
        print!("{:02x} ", b);
        if (k - 0x9030) % 16 == 15 {
            println!();
        }
    }
    println!();
    println!(
        "[0x9041] = {:#x}, [0x9045] = {:#x}",
        mem.read(0x9041, 4).unwrap_or(0),
        mem.read(0x9045, 4).unwrap_or(0)
    );
    // Dump stack pmode + save area real di titik crash epilogue (8623668-an).
    for region in [(0x7f700u64, 0x7f790u64), (0x1fb0u64, 0x2020u64)] {
        println!("--- stack {:x}..{:x} ---", region.0, region.1);
        let mut line = String::new();
        for (i, k) in (region.0..region.1).enumerate() {
            line.push_str(&format!("{:02x} ", mem.read(k, 1).unwrap_or(0xee)));
            if i % 16 == 15 {
                println!("{:04x}: {}", k - 15, line);
                line.clear();
            }
        }
        if !line.is_empty() {
            println!("{:04x}: {}", region.1 - (line.len() / 3) as u64, line);
        }
    }
}
