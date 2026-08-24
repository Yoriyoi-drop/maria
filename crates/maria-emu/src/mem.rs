//! Memory subsystem — EMULATOR.md §11.
//!
//! Guest memory berlapis (RAM/ROM/MMIO) dengan backend host (anonymous mmap,
//! siap hugepages). Alur akses: `Guest Physical Address → MemoryMap → region`.
//! Fase R1: region RAM/ROM + decode alamat + load bytes. MMIO dispatch ke RTL
//! (co-simulation) menyusul di R4.

use maria_core::intern::Symbol;

/// Jenis region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    Ram,
    Rom,
    Mmio,
}

/// Referensi region hasil decode alamat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionRef {
    pub kind: RegionKind,
    pub index: usize,
}

/// Kesalahan akses memori (di luar region / out-of-bounds / ukuran).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessFault {
    pub addr: u64,
    pub reason: String,
}

impl std::fmt::Display for AccessFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "access fault @0x{:x}: {}", self.addr, self.reason)
    }
}

/// Kontrak akses memori guest (byte-granularity, little-endian).
pub trait MemoryPort {
    fn read(&self, addr: u64, size: u8) -> Result<u64, AccessFault>;
    fn write(&mut self, addr: u64, size: u8, val: u64) -> Result<(), AccessFault>;
    fn region_of(&self, addr: u64) -> Option<RegionRef>;
}

/// Backing store region.
enum Backing {
    Mmap(memmap2::MmapMut),
    Vec(Vec<u8>),
}

impl Backing {
    fn len(&self) -> usize {
        match self {
            Backing::Mmap(m) => m.len(),
            Backing::Vec(v) => v.len(),
        }
    }
}

impl AsRef<[u8]> for Backing {
    fn as_ref(&self) -> &[u8] {
        match self {
            Backing::Mmap(m) => m,
            Backing::Vec(v) => v,
        }
    }
}

impl AsMut<[u8]> for Backing {
    fn as_mut(&mut self) -> &mut [u8] {
        match self {
            Backing::Mmap(m) => m,
            Backing::Vec(v) => v,
        }
    }
}

/// Satu region memori dengan backing store host.
pub struct RamRegion {
    pub name: Symbol,
    pub base: u64,
    pub size: u64,
    pub kind: RegionKind,
    backing: Backing,
}

impl std::fmt::Debug for RamRegion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RamRegion")
            .field("name", &self.name.as_str())
            .field("base", &self.base)
            .field("size", &self.size)
            .field("kind", &self.kind)
            .finish()
    }
}

impl RamRegion {
    /// Buat region RAM/ROM berukuran `size` di `base`.
    /// `mmap=true` → anonymous mmap (hugepages-ready); `false` → Vec (test kecil).
    pub fn new(
        name: Symbol,
        base: u64,
        size: u64,
        kind: RegionKind,
        mmap: bool,
    ) -> Result<Self, String> {
        if size == 0 {
            return Err(format!("region '{}': size 0", name.as_str()));
        }
        let backing = if mmap {
            let m = memmap2::MmapMut::map_anon(size as usize)
                .map_err(|e| format!("mmap anon {} bytes: {}", size, e))?;
            Backing::Mmap(m)
        } else {
            Backing::Vec(vec![0u8; size as usize])
        };
        Ok(Self {
            name,
            base,
            size,
            kind,
            backing,
        })
    }

    /// Alamat offset relatif ke base; None bila di luar region.
    fn offset(&self, addr: u64) -> Option<usize> {
        if addr >= self.base && addr < self.base + self.size {
            Some((addr - self.base) as usize)
        } else {
            None
        }
    }

    pub fn read(&self, addr: u64, size: u8) -> Result<u64, AccessFault> {
        let off = self.offset(addr).ok_or_else(|| AccessFault {
            addr,
            reason: format!("di luar region '{}'", self.name.as_str()),
        })?;
        let size = size as usize;
        if size == 0 || size > 8 || off + size > self.backing.len() {
            return Err(AccessFault {
                addr,
                reason: format!("ukuran {} tidak valid", size),
            });
        }
        let bytes = self.backing.as_ref();
        let mut v = 0u64;
        for i in 0..size {
            v |= (bytes[off + i] as u64) << (8 * i);
        }
        Ok(v)
    }

    pub fn write(&mut self, addr: u64, size: u8, val: u64) -> Result<(), AccessFault> {
        if self.kind == RegionKind::Rom {
            return Err(AccessFault {
                addr,
                reason: format!("write ke ROM '{}'", self.name.as_str()),
            });
        }
        let off = self.offset(addr).ok_or_else(|| AccessFault {
            addr,
            reason: format!("di luar region '{}'", self.name.as_str()),
        })?;
        let size = size as usize;
        if size == 0 || size > 8 || off + size > self.backing.len() {
            return Err(AccessFault {
                addr,
                reason: format!("ukuran {} tidak valid", size),
            });
        }
        let bytes = self.backing.as_mut();
        for i in 0..size {
            bytes[off + i] = ((val >> (8 * i)) & 0xff) as u8;
        }
        Ok(())
    }

    /// Salin byte ke region (untuk loader ELF/ISO). Write ke ROM ditolak.
    pub fn load_bytes(&mut self, addr: u64, data: &[u8]) -> Result<(), AccessFault> {
        if self.kind == RegionKind::Rom {
            return Err(AccessFault {
                addr,
                reason: format!("write ke ROM '{}'", self.name.as_str()),
            });
        }
        let off = self.offset(addr).ok_or_else(|| AccessFault {
            addr,
            reason: format!("di luar region '{}'", self.name.as_str()),
        })?;
        if off + data.len() > self.backing.len() {
            return Err(AccessFault {
                addr,
                reason: format!(
                    "{} byte melebihi region '{}'",
                    data.len(),
                    self.name.as_str()
                ),
            });
        }
        self.backing.as_mut()[off..off + data.len()].copy_from_slice(data);
        Ok(())
    }

    /// Isi utuh region (untuk dump / snapshot).
    pub fn bytes(&self) -> &[u8] {
        self.backing.as_ref()
    }
}

/// Peta alamat: daftar region + decode + dispatch.
pub struct MemoryMap {
    pub regions: Vec<RamRegion>,
}

impl std::fmt::Debug for MemoryMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryMap")
            .field("regions", &self.regions)
            .finish()
    }
}

impl MemoryMap {
    pub fn new() -> Self {
        Self {
            regions: Vec::new(),
        }
    }

    /// Tambah region; tolak overlap dengan region lain (decode deterministik).
    pub fn add(&mut self, region: RamRegion) -> Result<(), String> {
        for r in &self.regions {
            let a = region.base;
            let b = region.base + region.size;
            let c = r.base;
            let d = r.base + r.size;
            if a < d && c < b {
                return Err(format!(
                    "region '{}' (0x{:x}-0x{:x}) overlap dengan '{}' (0x{:x}-0x{:x})",
                    region.name.as_str(),
                    a,
                    b.saturating_sub(1),
                    r.name.as_str(),
                    c,
                    d.saturating_sub(1)
                ));
            }
        }
        self.regions.push(region);
        Ok(())
    }

    pub fn region_at(&self, addr: u64) -> Option<&RamRegion> {
        self.regions
            .iter()
            .find(|r| addr >= r.base && addr < r.base + r.size)
    }

    /// Salin byte ke region yang memuat `addr` (untuk loader ELF/ISO).
    pub fn load_bytes(&mut self, addr: u64, data: &[u8]) -> Result<(), AccessFault> {
        let idx = self
            .regions
            .iter()
            .position(|r| addr >= r.base && addr < r.base + r.size)
            .ok_or_else(|| AccessFault {
                addr,
                reason: "unmapped".into(),
            })?;
        self.regions[idx].load_bytes(addr, data)
    }
}

impl Default for MemoryMap {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryPort for MemoryMap {
    fn read(&self, addr: u64, size: u8) -> Result<u64, AccessFault> {
        match self.region_at(addr) {
            Some(r) => r.read(addr, size),
            None => Err(AccessFault {
                addr,
                reason: "unmapped".into(),
            }),
        }
    }

    fn write(&mut self, addr: u64, size: u8, val: u64) -> Result<(), AccessFault> {
        // Bounds-checked mutable access: cari index dulu (hindari borrow ganda).
        let idx = self
            .regions
            .iter()
            .position(|r| addr >= r.base && addr < r.base + r.size)
            .ok_or_else(|| AccessFault {
                addr,
                reason: "unmapped".into(),
            })?;
        self.regions[idx].write(addr, size, val)
    }

    fn region_of(&self, addr: u64) -> Option<RegionRef> {
        self.regions
            .iter()
            .position(|r| addr >= r.base && addr < r.base + r.size)
            .map(|i| RegionRef {
                kind: self.regions[i].kind,
                index: i,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> MemoryMap {
        let mut m = MemoryMap::new();
        m.add(
            RamRegion::new(
                Symbol::intern("ram"),
                0x8000_0000,
                0x1000,
                RegionKind::Ram,
                false,
            )
            .unwrap(),
        )
        .unwrap();
        m.add(
            RamRegion::new(
                Symbol::intern("rom"),
                0x0001_0000,
                0x100,
                RegionKind::Rom,
                false,
            )
            .unwrap(),
        )
        .unwrap();
        m
    }

    #[test]
    fn test_read_write_byte_and_word() {
        let mut m = map();
        m.write(0x8000_0010, 4, 0xdead_beef).unwrap();
        assert_eq!(m.read(0x8000_0010, 4).unwrap(), 0xdead_beef);
        m.write(0x8000_0014, 1, 0xab).unwrap();
        assert_eq!(m.read(0x8000_0014, 1).unwrap(), 0xab);
        // Little-endian: byte 0 = 0xef.
        assert_eq!(m.read(0x8000_0010, 1).unwrap(), 0xef);
        assert_eq!(m.read(0x8000_0013, 1).unwrap(), 0xde);
    }

    #[test]
    fn test_unmapped_access_fault() {
        let m = map();
        assert!(m.read(0x9000_0000, 1).is_err());
        assert!(m.read(0x8000_1000, 1).is_err(), "tepat di ujung region");
    }

    #[test]
    fn test_rom_write_rejected() {
        let mut m = map();
        assert!(m.write(0x0001_0010, 1, 1).is_err(), "ROM read-only");
        assert_eq!(m.read(0x0001_0010, 1).unwrap(), 0);
    }

    #[test]
    fn test_region_of_decode() {
        let m = map();
        let r = m.region_of(0x8000_0000).unwrap();
        assert_eq!(r.kind, RegionKind::Ram);
        assert_eq!(r.index, 0);
        let r = m.region_of(0x0001_0050).unwrap();
        assert_eq!(r.kind, RegionKind::Rom);
        assert_eq!(r.index, 1);
        assert!(m.region_of(0xffff_ffff).is_none());
    }

    #[test]
    fn test_load_bytes() {
        let mut m = map();
        m.load_bytes(0x8000_0000, &[1, 2, 3, 4]).unwrap();
        assert_eq!(m.read(0x8000_0000, 4).unwrap(), 0x0403_0201);
        assert!(
            m.load_bytes(0x8000_0ff0, &[0u8; 32]).is_err(),
            "melebihi region"
        );
    }

    #[test]
    fn test_overlap_rejected() {
        let mut m = map();
        let dup = RamRegion::new(
            Symbol::intern("dup"),
            0x8000_0800,
            0x100,
            RegionKind::Ram,
            false,
        )
        .unwrap();
        assert!(m.add(dup).is_err(), "overlap dengan 'ram'");
        // Region berdampingan (tidak overlap) diterima.
        let next = RamRegion::new(
            Symbol::intern("next"),
            0x8000_1000,
            0x100,
            RegionKind::Ram,
            false,
        )
        .unwrap();
        assert!(m.add(next).is_ok());
    }

    #[test]
    fn test_mmap_backing() {
        let mut r =
            RamRegion::new(Symbol::intern("mm"), 0x1000, 64, RegionKind::Ram, true).unwrap();
        r.write(0x1000, 2, 0x1234).unwrap();
        assert_eq!(r.read(0x1000, 2).unwrap(), 0x1234);
    }
}
