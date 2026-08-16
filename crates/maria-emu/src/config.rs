//! Konfigurasi emulator — file TOML terpisah, dimuat via `--config`.
//!
//! **TIDAK** memakai section `[emu]` di project file `.maria` — ekstensi
//! `.maria` dipakai MICD (`project/.maria/database/`) dan file list; konfigurasi
//! emulator hidup di file sendiri (default ekstensi `.meu`) agar tidak bentrok.
//!
//! Contoh `soc.meu`:
//!
//! ```toml
//! top = "emu_soc"
//! mode = "hybrid"
//! accuracy = "functional"
//! cpu = "riscv32"
//!
//! [ram]
//! base = 0x80000000
//! size = 0x1000
//!
//! [[devices]]              # Direct RTL Device (EMULATOR.md §10)
//! name = "u_uart"
//! rtl = "uart.sv"
//! mmio = 0x10000000
//! size = 0x1000
//! irq = 5
//! ```

use std::path::Path;

/// Region RAM utama (base/size — TOML integer, hex `0x...` didukung).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct RamConfig {
    pub base: u64,
    pub size: u64,
}

/// Satu device dari `[[devices]]` (Direct RTL Device).
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct EmuDeviceConfig {
    /// Nama instance/module untuk match (bila kosong: semua device RTL itu).
    pub name: Option<String>,
    /// File RTL device (informasi saja di fase ini).
    pub rtl: Option<String>,
    pub mmio: Option<u64>,
    pub size: Option<u64>,
    pub irq: Option<u32>,
}

/// Konfigurasi emulator (isi file `.meu`).
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct EmuConfig {
    pub top: Option<String>,
    /// Mode operasi: rtl | sim | emu | hybrid | coemu.
    pub mode: Option<String>,
    /// Akurasi: functional | cycle-accurate.
    pub accuracy: Option<String>,
    pub cpu: Option<String>,
    pub ram: Option<RamConfig>,
    pub console: Option<String>,
    #[serde(default)]
    pub block: Vec<String>,
    pub iso: Option<String>,
    pub firmware: Option<String>,
    pub dtb: Option<String>,
    pub seed: Option<u64>,
    #[serde(default)]
    pub devices: Vec<EmuDeviceConfig>,
}

impl EmuConfig {
    /// Load konfigurasi dari file TOML.
    pub fn load_file(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read '{}': {}", path.display(), e))?;
        Self::from_toml(&content)
            .map_err(|e| format!("config '{}': {}", path.display(), e))
    }

    /// Parse konten TOML menjadi `EmuConfig`.
    pub fn from_toml(content: &str) -> Result<Self, String> {
        toml::from_str(content).map_err(|e| format!("TOML: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: &str = r#"
top = "emu_soc"
mode = "hybrid"
accuracy = "functional"
cpu = "riscv32"
console = "pty"
block = ["rootfs.img", "data.img"]
iso = "debian-riscv64.iso"
firmware = "opensbi.bin"
dtb = "board.dts"
seed = 12345

[ram]
base = 0x80000000
size = 0x40000000

[[devices]]
name = "u_uart"
rtl = "uart.sv"
mmio = 0x10000000
size = 0x1000
irq = 5

[[devices]]
name = "u_timer"
mmio = 0x20000000
size = 0x100
"#;

    #[test]
    fn test_parse_full_config() {
        let cfg = EmuConfig::from_toml(CFG).expect("parse");
        assert_eq!(cfg.top.as_deref(), Some("emu_soc"));
        assert_eq!(cfg.mode.as_deref(), Some("hybrid"));
        assert_eq!(cfg.accuracy.as_deref(), Some("functional"));
        assert_eq!(cfg.cpu.as_deref(), Some("riscv32"));
        assert_eq!(cfg.console.as_deref(), Some("pty"));
        assert_eq!(cfg.block, vec!["rootfs.img".to_string(), "data.img".to_string()]);
        assert_eq!(cfg.iso.as_deref(), Some("debian-riscv64.iso"));
        assert_eq!(cfg.firmware.as_deref(), Some("opensbi.bin"));
        assert_eq!(cfg.dtb.as_deref(), Some("board.dts"));
        assert_eq!(cfg.seed, Some(12345));
        // Hex integer TOML → u64.
        let ram = cfg.ram.expect("ram");
        assert_eq!(ram.base, 0x8000_0000);
        assert_eq!(ram.size, 0x4000_0000);
    }

    #[test]
    fn test_parse_devices() {
        let cfg = EmuConfig::from_toml(CFG).expect("parse");
        assert_eq!(cfg.devices.len(), 2);
        let uart = &cfg.devices[0];
        assert_eq!(uart.name.as_deref(), Some("u_uart"));
        assert_eq!(uart.rtl.as_deref(), Some("uart.sv"));
        assert_eq!(uart.mmio, Some(0x1000_0000));
        assert_eq!(uart.size, Some(0x1000));
        assert_eq!(uart.irq, Some(5));
        let timer = &cfg.devices[1];
        assert_eq!(timer.name.as_deref(), Some("u_timer"));
        assert_eq!(timer.mmio, Some(0x2000_0000));
        assert_eq!(timer.irq, None);
    }

    #[test]
    fn test_parse_minimal_config() {
        let cfg = EmuConfig::from_toml("top = \"bare\"\n").expect("parse");
        assert_eq!(cfg.top.as_deref(), Some("bare"));
        assert_eq!(cfg, EmuConfig { top: Some("bare".into()), ..Default::default() });
    }

    #[test]
    fn test_parse_empty() {
        let cfg = EmuConfig::from_toml("").expect("parse kosong");
        assert_eq!(cfg, EmuConfig::default());
    }

    #[test]
    fn test_parse_invalid_toml_errors() {
        assert!(EmuConfig::from_toml("ram = { base = \"x\" }").is_err());
        assert!(EmuConfig::from_toml("[[[bad").is_err());
    }
}
