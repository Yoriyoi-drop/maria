//! iso.rs — loader media ISO9660 + El Torito boot catalog untuk `--boot-iso`
//! (EMULATOR.md §20 R6 x86 real-mode boot: MBR→El Torito→cdboot→kernel).
//!
//! Boot ISO yang benar (real CD): BIOS baca volume descriptors, temukan Boot
//! Record (type 0), baca boot catalog, lalu muat boot image (no-emul) ke
//! 0x7C00 dengan DL = 0xE0 (drive CD). Ini jalur yang dipakai GRUB cdboot —
//! berbeda dari hybrid-MBR (isohdpfx) yang hanya berlaku untuk USB/HDD.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// Sektor per blok CD (El Torito: semua LBA dalam blok 2048-byte).
pub const CD_BLOCK: u64 = 2048;

/// Struktur boot entry El Torito (dari boot catalog, section header 0x88).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElToritoBootEntry {
    /// Media type: 0 = no emulation (boot image dijalankan langsung).
    pub media_type: u8,
    /// Platform/system type (0 = 80x86).
    pub platform_id: u8,
    /// Segment address boot image (0 untuk no-emul @0x7c00).
    pub segment: u16,
    /// Panjang boot image, dalam sektor 512-byte.
    pub len_512: u16,
    /// RBA (LBA dalam blok 2048) boot image.
    pub image_lba: u32,
}

/// Hasil parse El Torito: lokasi catalog + boot entry pertama tidak-2.88M.
#[derive(Debug, Clone)]
pub struct ElToritoBoot {
    /// LBA boot catalog (blok 2048).
    pub catalog_lba: u32,
    /// Boot entry yang dipakai (media no-emul / floppy / hd).
    pub entry: ElToritoBootEntry,
}

/// Cari descriptor Boot Record (type 0) pada volume descriptor set.
/// Volume descriptors mulai sektor 16 (0x10); terminator type 0xFF.
fn find_boot_record(file: &mut File) -> Result<u32, String> {
    let vol_desc_start: u64 = 16;
    for i in 0..128u32 {
        let lba = vol_desc_start + i as u64;
        file.seek(SeekFrom::Start(lba * CD_BLOCK))
            .map_err(|e| format!("seek descriptor {:?}", e))?;
        let mut desc = [0u8; CD_BLOCK as usize];
        file.read_exact(&mut desc)
            .map_err(|e| format!("baca descriptor: {}", e))?;
        if desc[0] == 0xff {
            break; // terminator
        }
        if desc[0] == 0 && &desc[1..6] == b"CD001" {
            // Boot Record: boot catalog LBA (LE 32-bit) di offset 0x47.
            let cat = u32::from_le_bytes(desc[0x47..0x4B].try_into().unwrap());
            return Ok(cat);
        }
    }
    Err("ISO tidak punya Boot Record volume descriptor (El Torito)".into())
}

/// Parse boot catalog: validation entry @0 + section headers (0x88) @32.
/// Ambil entry pertama; prioritas media no-emul (0), lalu floppy/harddisk.
fn parse_catalog(file: &mut File, cat_lba: u32) -> Result<ElToritoBootEntry, String> {
    file.seek(SeekFrom::Start(cat_lba as u64 * CD_BLOCK))
        .map_err(|e| format!("seek catalog: {}", e))?;
    let mut cat = vec![0u8; CD_BLOCK as usize];
    file.read_exact(&mut cat)
        .map_err(|e| format!("baca catalog: {}", e))?;
    // Validation entry @0: type 1 (byte 0). ID "CD001" + version 1 umum,
    // tetapi banyak ISO modern (xorriso) menulisnya nol — BIOS nyata lenient.
    if cat[0] != 1 {
        return Err(format!(
            "boot catalog @LBA {}: validation entry type 0x{:02x} != 1",
            cat_lba, cat[0]
        ));
    }
    let mut fallback: Option<ElToritoBootEntry> = None;
    let mut off = 32;
    while off + 32 <= cat.len() {
        let e = &cat[off..off + 32];
        let ty = e[0];
        if ty == 0xff {
            break; // terminator
        }
        if ty == 0x88 {
            // Section header / boot entry.
            let media = e[1];
            let seg = u16::from_le_bytes([e[2], e[3]]);
            let len512 = u16::from_le_bytes([e[6], e[7]]);
            let lba = u32::from_le_bytes(e[8..12].try_into().unwrap());
            let entry = ElToritoBootEntry {
                media_type: media & 0x0F,
                platform_id: e[4],
                segment: seg,
                len_512: len512,
                image_lba: lba,
            };
            if media == 0 {
                return Ok(entry); // no-emulation → pilihan utama
            }
            if fallback.is_none() {
                fallback = Some(entry);
            }
        }
        off += 32;
    }
    fallback.ok_or_else(|| "boot catalog: tidak ada boot entry".into())
}

/// Parse media ISO → struktur El Torito (catalog LBA + boot entry).
pub fn parse_eltorito(file: &mut File) -> Result<ElToritoBoot, String> {
    let cat = find_boot_record(file)?;
    let entry = parse_catalog(file, cat)?;
    Ok(ElToritoBoot {
        catalog_lba: cat,
        entry,
    })
}

/// Baca boot image (byte offset = image_lba * 2048) langsung dari file.
/// Panjang = min(entry.len_512 * 512, cap). Cap menghindari alokasi raksasa
/// bila catalog salah. Untuk no-emul, BIOS biasanya cukup 512 byte (cdboot).
pub fn read_boot_image(
    file: &mut File,
    entry: &ElToritoBootEntry,
    cap: usize,
) -> Result<Vec<u8>, String> {
    // Minimal 512 byte (BIOS muat 1 sektor boot), dibatasi `cap`.
    let len = ((entry.len_512 as usize).saturating_mul(512).max(512)).min(cap.max(512));
    let mut buf = vec![0u8; len];
    file.seek(SeekFrom::Start(entry.image_lba as u64 * CD_BLOCK))
        .map_err(|e| format!("seek boot image: {}", e))?;
    file.read_exact(&mut buf)
        .map_err(|e| format!("baca boot image: {}", e))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn root_of(rel: &str) -> String {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .unwrap()
            .to_path_buf();
        root.join(rel).to_string_lossy().to_string()
    }

    #[test]
    fn test_parse_eltorito_ubuntu_iso() {
        let iso = root_of("ubuntu-26.04-desktop-amd64.iso");
        if !Path::new(&iso).exists() {
            eprintln!("skipped: ISO tidak ada ({iso})");
            return;
        }
        let mut f = File::open(&iso).expect("buka ISO");
        let boot = parse_eltorito(&mut f).expect("parse El Torito");
        // Boot catalog di LBA 666, boot image no-emul di LBA 667 (dianalisis).
        assert_eq!(boot.catalog_lba, 666, "boot catalog harus di LBA 666");
        assert_eq!(boot.entry.media_type, 0, "harus no-emulation");
        assert_eq!(boot.entry.image_lba, 667, "boot image harus di LBA 667");
        assert_eq!(boot.entry.platform_id, 0, "platform 80x86");
        assert!(boot.entry.len_512 >= 1);
        // cdboot byte pertama = call next (e8 00 00).
        let image = read_boot_image(&mut f, &boot.entry, 512).expect("baca boot image");
        assert_eq!(&image[0..3], &[0xe8, 0x00, 0x00], "cdboot: call next");
    }

    #[test]
    fn test_read_boot_image_padding() {
        // entry synthetic: len 0 → minimal 512 byte dibaca.
        let entry = ElToritoBootEntry {
            media_type: 0,
            platform_id: 0,
            segment: 0,
            len_512: 0,
            image_lba: 0,
        };
        // Buka ISO, baca boot image dari LBA 0 (MBR region) → 512 byte.
        let iso = root_of("ubuntu-26.04-desktop-amd64.iso");
        if !Path::new(&iso).exists() {
            return;
        }
        let mut f = File::open(&iso).unwrap();
        let buf = read_boot_image(&mut f, &entry, 512).unwrap();
        assert_eq!(buf.len(), 512);
        // LBA 0 ISO (blok 2048) = area boot info table; byte 0 biasanya 0xFA/0xEB.
        assert!(buf[0] == 0xFA || buf[0] == 0xEB, "boot area byte pertama");
    }
}
