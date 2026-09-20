// probe: parse ISO9660 volume descriptors → El Torito boot catalog.
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

fn main() {
    let iso = std::env::var("ISO").unwrap_or_else(|_| "ubuntu-26.04-desktop-amd64.iso".into());
    let mut f = File::open(&iso).unwrap();
    let mut buf = vec![0u8; 2048];
    // Volume descriptors start at sector 16 (0x10)
    for lba in 16..60u32 {
        f.seek(SeekFrom::Start(lba as u64 * 2048)).unwrap();
        f.read_exact(&mut buf).unwrap();
        if buf[0] == 0xff {
            break;
        }
        let t = buf[0];
        let id = &buf[1..6];
        let desc = String::from_utf8_lossy(&buf[8..40]);
        println!(
            "LBA {} type={} id={:?} desc='{}'",
            lba,
            t,
            String::from_utf8_lossy(id),
            desc
        );
        if t == 0 {
            // Boot Record
            // bytes 0x47..0x4A = boot catalog LBA
            let cat_lba = buf[0x47] as u64
                | (buf[0x48] as u64) << 8
                | (buf[0x49] as u64) << 16
                | (buf[0x4A] as u64) << 24;
            println!("  → Boot catalog LBA = {}", cat_lba);
            // fileSector berorientasi 2048-blok; baca catalog
            f.seek(SeekFrom::Start(cat_lba * 2048)).unwrap();
            let mut cat = vec![0u8; 2048];
            f.read_exact(&mut cat).unwrap();
            // Validation entry @0
            println!("  Validation: id={:?} version={}", &cat[0..5], cat[6]);
            // iterate 32-byte entries from offset 32
            let mut off = 32;
            loop {
                let e = &cat[off..off + 32];
                let t = e[0];
                if t == 0xff {
                    break;
                } // terminator
                if t == 0x88 {
                    // boot info / section
                    let media = e[1];
                    let seg = e[2] as u16 | (e[3] as u16) << 8;
                    let systs = e[4];
                    let lba = e[8] as u64
                        | (e[9] as u64) << 8
                        | (e[10] as u64) << 16
                        | (e[11] as u64) << 24;
                    let count = e[6] as u64 | (e[7] as u64) << 8;
                    println!(
                        "  Boot section: media={} seg=0x{:04x} systs=0x{:02x} lba={} len512={}",
                        media, seg, systs, lba, count
                    );
                } else {
                    println!("  Entry type {:#04x}", t);
                }
                off += 32;
                if off + 32 > cat.len() {
                    break;
                }
            }
            break;
        } else if t == 1 {
            // PVD — sector size etc.
            println!(
                "  PVD: volsize={} (2048-blocks), filesystem entry at LBA {}",
                buf[0x50] as u64
                    | (buf[0x51] as u64) << 8
                    | (buf[0x52] as u64) << 16
                    | (buf[0x53] as u64) << 24,
                buf[0x9C] as u64
                    | (buf[0x9D] as u64) << 8
                    | (buf[0x9E] as u64) << 16
                    | (buf[0x9F] as u64) << 24
            );
        }
    }
}
