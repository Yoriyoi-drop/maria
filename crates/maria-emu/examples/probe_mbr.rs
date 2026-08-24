//! Probe: eksekusi MBR ISO asli, cetak langkah terakhir + alasan halt + stack.
use maria_core::intern::Symbol;
use maria_emu::cpu::x86::{FileDisk, X86Cpu};
use maria_emu::cpu::CpuCore;
use maria_emu::mem::{MemoryMap, MemoryPort, RamRegion, RegionKind};

/// Byte ke-N dari core.img ISO (LBA 2673, dimuat ke 0x8200).
fn iso_byte(addr: usize) -> u8 {
    let iso = "/tmp/core.img";
    let data = std::fs::read(iso).unwrap_or_default();
    let off = addr - 0x8200;
    data.get(off).copied().unwrap_or(0)
}

fn main() {
    let mut m = MemoryMap::new();
    // RAM 64MB: GRUB mendekompres dirinya ke alamat tinggi (~0x8320_0000).
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
    for _ in 0..40000000 {
        if cpu.halted {
            break;
        }
        let pc = cpu.pc();
        let b = m.read(pc, 1).unwrap_or(0);
        let _ = cpu.step(&mut m);
        n += 1;
        if n % 500000 == 0 {
            let mut s = String::new();
            for a in 0x100000u64..0x100020 {
                s.push_str(&format!("{:02x} ", m.read(a, 1).unwrap_or(0)));
            }
            println!("[{n}] pc=0x{pc:08x} edi=0x{:08x} out100000={s}", cpu.r32(7));
        }
    }
    println!("HALT: {} (steps={})", cpu.halt_reason, cpu.steps);
    println!("out: {:?}", String::from_utf8_lossy(&cpu.out));
    let esp = cpu.r32(4);
    let mut s = String::new();
    for a in (esp.saturating_sub(0x40))..(esp + 0x20) {
        s.push_str(&format!("{:02x} ", m.read(a as u64, 1).unwrap_or(0)));
        if (a - esp.saturating_sub(0x40) + 1) % 16 == 0 {
            s.push('\n');
        }
    }
    println!("stack around esp=0x{esp:08x}:\n{s}");
    // Bandingkan memori emulator vs ISO core.img di sekitar 0x8c43.
    let mut s = String::new();
    for a in 0x8c40u64..0x8d10 {
        s.push_str(&format!("{:02x} ", m.read(a, 1).unwrap_or(0)));
        if (a - 0x8c40 + 1) % 16 == 0 {
            s.push('\n');
        }
    }
    println!("emu mem[0x8c40..0x8d10]:\n{s}");
    let mut s = String::new();
    for a in 0x8c40usize..0x8d10 {
        s.push_str(&format!("{:02x} ", iso_byte(a)));
        if (a - 0x8c40 + 1) % 16 == 0 {
            s.push('\n');
        }
    }
    println!("iso mem[0x8c40..0x8d10]:\n{s}");
    println!(
        "gdt_base=0x{:08x} limit=0x{:04x} pmode={} cr0=0x{:08x} cs=0x{:04x} ss=0x{:04x}",
        cpu.gdt_base, cpu.gdt_limit, cpu.pmode, cpu.cr[0], cpu.cs, cpu.ss
    );
    for (name, base) in [
        ("out0x100000", 0x100000u64),
        ("out0x101000", 0x101000),
        ("out0x202000", 0x202000),
        ("probtab_0x10c498", 0x10c498),
        ("in_0x8d10", 0x8d10),
        ("pc_region_0x3000", 0x3000),
        ("pc_region_0x8000", 0x8000),
        ("core_at_0x8200", 0x8200),
    ] {
        let mut s = String::new();
        for a in base..base + 0x100 {
            s.push_str(&format!("{:02x} ", m.read(a, 1).unwrap_or(0)));
            if (a - base + 1) % 16 == 0 {
                s.push('\n');
            }
        }
        println!("{name} [0x{base:08x}..]:\n{s}");
    }
}
