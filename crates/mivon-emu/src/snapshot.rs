//! Machine snapshot — EMULATOR.md §14 (fase R5, slice pertama).
//!
//! Satu snapshot = state CPU (blob biner per-ISA) + seluruh region memori
//! guest + counter langkah. Format file sendiri (little-endian, tanpa
//! dependensi serde/bincode) — lihat `MachineSnapshot::save_file`.
//!
//! Memori di-encode SPARSE per halaman 4 KB: halaman sepenuhnya nol tidak
//! ditulis (RAM 2 GB berisi sedikit data → file kecil). Restore meng-zero
//! region dulu lalu menulis halaman yang ada di snapshot, sehingga state
//! hasil restore identik dengan saat disimpan (deterministik).
//!
//! Blob CPU di-encode oleh tiap implementasi `CpuCore::snapshot` memakai
//! helper `Writer`/`Reader` di file ini (format kecil, versioned per CPU).

use std::path::Path;

/// Magic file snapshot (`MIVSNAP1`).
pub const MAGIC: &[u8; 8] = b"MIVSNAP1";
/// Versi format file (bump bila layout berubah → pesan error jelas).
pub const FORMAT_VERSION: u32 = 1;
/// Ukuran halaman memori untuk encoding sparse.
pub const PAGE: usize = 4096;

// ─── Writer / Reader biner (little-endian) ───

/// Penulis byte little-endian ke buffer.
#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    pub fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    /// Byte array (tanpa prefix — panjang diketahui pemanggil).
    pub fn bytes(&mut self, v: &[u8]) {
        self.buf.extend_from_slice(v);
    }
    /// Panjang + byte (untuk slice berukuran variabel).
    pub fn blob(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.buf.extend_from_slice(v);
    }
    /// String UTF-8 dengan prefix panjang.
    pub fn string(&mut self, s: &str) {
        self.blob(s.as_bytes());
    }
}

/// Pembaca byte little-endian; semua akses bounds-checked → `Err` jelas
/// (blob rusak / versi beda), bukan panic.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.pos + n > self.buf.len() {
            return Err(format!(
                "snapshot: terpotong (butuh {} byte, sisa {})",
                n,
                self.buf.len() - self.pos
            ));
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    pub fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        self.take(n)
    }
    pub fn blob(&mut self) -> Result<&'a [u8], String> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    pub fn string(&mut self) -> Result<String, String> {
        let b = self.blob()?;
        String::from_utf8(b.to_vec()).map_err(|e| format!("snapshot: string bukan UTF-8: {}", e))
    }
    pub fn done(&self) -> bool {
        self.pos >= self.buf.len()
    }
}

// ─── Struktur snapshot ───

/// Satu region memori guest dalam snapshot (halaman non-nol saja).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionSnapshot {
    pub name: String,
    pub base: u64,
    pub size: u64,
    /// (indeks halaman, isi halaman ≤ 4096 byte) — halaman nol tidak ada.
    pub pages: Vec<(u32, Vec<u8>)>,
}

/// Snapshot mesin: state CPU + memori + counter langkah kumulatif.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineSnapshot {
    /// Instruksi kumulatif pada saat snapshot (termasuk run sebelumnya).
    pub steps: u64,
    /// Cycle kumulatif pada saat snapshot.
    pub cycles: u64,
    /// Blob state CPU (format per-ISA, di-encode `CpuCore::snapshot`).
    pub cpu: Vec<u8>,
    pub regions: Vec<RegionSnapshot>,
}

impl MachineSnapshot {
    /// Encode halaman non-nol dari `bytes` (region `size` byte).
    pub fn encode_pages(bytes: &[u8]) -> Vec<(u32, Vec<u8>)> {
        let mut out = Vec::new();
        for (idx, chunk) in bytes.chunks(PAGE).enumerate() {
            if chunk.iter().any(|&b| b != 0) {
                out.push((idx as u32, chunk.to_vec()));
            }
        }
        out
    }

    /// Serialisasi ke bytes (format `MIVSNAP1` + versi).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.bytes(MAGIC);
        w.u32(FORMAT_VERSION);
        w.u64(self.steps);
        w.u64(self.cycles);
        w.blob(&self.cpu);
        w.u32(self.regions.len() as u32);
        for r in &self.regions {
            w.string(&r.name);
            w.u64(r.base);
            w.u64(r.size);
            w.u32(r.pages.len() as u32);
            for (idx, data) in &r.pages {
                w.u32(*idx);
                w.blob(data);
            }
        }
        w.buf
    }

    /// Deserialisasi dari bytes; magic/versi/path rusak → `Err` jelas.
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        let mut r = Reader::new(data);
        let magic = r.bytes(8)?;
        if magic != MAGIC {
            return Err(format!(
                "snapshot: magic salah ({} ≠ MIVSNAP1) — bukan file snapshot mivon",
                String::from_utf8_lossy(magic)
            ));
        }
        let ver = r.u32()?;
        if ver != FORMAT_VERSION {
            return Err(format!(
                "snapshot: versi format {} (didukung {})",
                ver, FORMAT_VERSION
            ));
        }
        let steps = r.u64()?;
        let cycles = r.u64()?;
        let cpu = r.blob()?.to_vec();
        let n = r.u32()? as usize;
        let mut regions = Vec::with_capacity(n.min(1024));
        for _ in 0..n {
            let name = r.string()?;
            let base = r.u64()?;
            let size = r.u64()?;
            let np = r.u32()? as usize;
            let mut pages = Vec::with_capacity(np.min(1024));
            for _ in 0..np {
                let idx = r.u32()?;
                pages.push((idx, r.blob()?.to_vec()));
            }
            regions.push(RegionSnapshot {
                name,
                base,
                size,
                pages,
            });
        }
        Ok(MachineSnapshot {
            steps,
            cycles,
            cpu,
            regions,
        })
    }

    /// Tulis ke file (atomik: temp + rename — konsisten dengan MICD).
    pub fn save_file(&self, path: &Path) -> Result<(), String> {
        let bytes = self.to_bytes();
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &bytes)
            .map_err(|e| format!("snapshot: tulis '{}': {}", tmp.display(), e))?;
        std::fs::rename(&tmp, path)
            .map_err(|e| format!("snapshot: rename ke '{}': {}", path.display(), e))?;
        Ok(())
    }

    /// Baca dari file.
    pub fn load_file(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path)
            .map_err(|e| format!("snapshot: baca '{}': {}", path.display(), e))?;
        Self::from_bytes(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_writer_reader_roundtrip() {
        let mut w = Writer::new();
        w.u8(0xab);
        w.u16(0x1234);
        w.u32(0xdead_beef);
        w.u64(0x0123_4567_89ab_cdef);
        w.string("halo");
        w.blob(&[1, 2, 3]);
        let mut r = Reader::new(&w.buf);
        assert_eq!(r.u8().unwrap(), 0xab);
        assert_eq!(r.u16().unwrap(), 0x1234);
        assert_eq!(r.u32().unwrap(), 0xdead_beef);
        assert_eq!(r.u64().unwrap(), 0x0123_4567_89ab_cdef);
        assert_eq!(r.string().unwrap(), "halo");
        assert_eq!(r.blob().unwrap(), &[1, 2, 3]);
        assert!(r.done());
    }

    #[test]
    fn test_reader_truncated_error() {
        // 4 byte dibaca dari buffer 2 byte → error.
        let mut r = Reader::new(&[1, 2]);
        assert!(r.u32().is_err());
        // Blob dengan panjang 0 → valid (kosong).
        let mut r = Reader::new(&[0, 0, 0, 0]);
        assert!(r.blob().unwrap().is_empty());
        assert!(r.u8().is_err(), "buffer habis");
        // Blob yang menuntut 10 byte padahal tak ada data → error.
        let mut r = Reader::new(&[10, 0, 0, 0]);
        assert!(r.blob().is_err());
    }

    #[test]
    fn test_pages_sparse_skips_zero() {
        // 2 halaman: pertama ada data, kedua nol penuh → 1 page.
        let mut bytes = vec![0u8; PAGE * 2];
        bytes[10] = 0x5a;
        let pages = MachineSnapshot::encode_pages(&bytes);
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].0, 0);
        assert_eq!(pages[0].1.len(), PAGE);
        assert_eq!(pages[0].1[10], 0x5a);
        // Region seluruhnya nol → tanpa page (file tetap ada isinya).
        assert!(MachineSnapshot::encode_pages(&vec![0u8; PAGE * 8]).is_empty());
        // Halaman terakhir parsial (size bukan kelipatan PAGE).
        let odd = vec![7u8; 100];
        let pages = MachineSnapshot::encode_pages(&odd);
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].1.len(), 100);
    }

    #[test]
    fn test_snapshot_bytes_roundtrip() {
        let snap = MachineSnapshot {
            steps: 123_456,
            cycles: 654_321,
            cpu: vec![0xc0, 0xff, 0xee],
            regions: vec![RegionSnapshot {
                name: "ram".into(),
                base: 0x8000_0000,
                size: 0x2000,
                pages: vec![(0, vec![1u8; 64]), (1, vec![2u8; PAGE])],
            }],
        };
        let bytes = snap.to_bytes();
        let back = MachineSnapshot::from_bytes(&bytes).expect("roundtrip");
        assert_eq!(back, snap);
    }

    #[test]
    fn test_from_bytes_bad_magic_and_version() {
        assert!(MachineSnapshot::from_bytes(b"BUKANMAGICSISANYA").is_err());
        let mut w = Writer::new();
        w.bytes(MAGIC);
        w.u32(FORMAT_VERSION + 99);
        assert!(MachineSnapshot::from_bytes(&w.buf)
            .unwrap_err()
            .contains("versi format"));
    }

    #[test]
    fn test_save_load_file_roundtrip() {
        let dir = std::env::temp_dir().join(format!("mivon_snap_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.snap");
        let snap = MachineSnapshot {
            steps: 7,
            cycles: 42,
            cpu: b"cpu-blob".to_vec(),
            regions: vec![RegionSnapshot {
                name: "ram".into(),
                base: 0x1000,
                size: PAGE as u64,
                pages: vec![(0, vec![9u8; 16])],
            }],
        };
        snap.save_file(&path).expect("save");
        let back = MachineSnapshot::load_file(&path).expect("load");
        assert_eq!(back, snap);
        // File rusak → error jelas.
        std::fs::write(&path, b"xxxx").unwrap();
        assert!(MachineSnapshot::load_file(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
