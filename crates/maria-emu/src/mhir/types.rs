//! Struktur data MHIR (Maria Hardware IR) — EMULATOR.md §4.
//!
//! Setiap node membawa `BackPointer` ke source RTL (file/baris/kolom) agar
//! debugger lintas-lapisan bisa menelusuri: Guest OS → MMIO → device → RTL
//! signal → `uart.sv:143`.

use maria_core::intern::Symbol;
use maria_ir::SignalId;

/// Posisi asal di source RTL (back-pointer) — `uart.sv:143`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BackPointer {
    /// Nama file sumber (bila diketahui).
    pub file: Option<String>,
    /// Baris 1-based; 0 = tidak diketahui.
    pub line: usize,
    /// Kolom 1-based; 0 = tidak diketahui.
    pub col: usize,
}

impl BackPointer {
    pub fn known(file: Option<String>, line: usize, col: usize) -> Self {
        Self { file, line, col }
    }

    /// Format ringkas `file:line` atau `file:line:col` untuk output.
    pub fn display(&self) -> String {
        match (&self.file, self.line) {
            (Some(f), 0) => f.clone(),
            (Some(f), l) if self.col > 0 => format!("{}:{}:{}", f, l, self.col),
            (Some(f), l) => format!("{}:{}", f, l),
            (None, 0) => "-".to_string(),
            (None, l) if self.col > 0 => format!("{}:{}", l, self.col),
            (None, l) => l.to_string(),
        }
    }
}

/// Jenis edge clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockEdgeKind {
    PosEdge,
    NegEdge,
}

/// Deskripsi clock dari proses `Sequential` (`always_ff @(posedge clk)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockDesc {
    pub name: Symbol,
    pub signal_id: SignalId,
    pub edge: ClockEdgeKind,
    /// Clock hierarkis (`posedge b.clk`) bila bukan signal lokal.
    pub hier: Option<Symbol>,
}

/// Deskripsi reset dari proses `Sequential`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetDesc {
    pub signal: Symbol,
    /// true = active-high.
    pub polarity: bool,
    /// true = asynchronous reset.
    pub async_: bool,
}

/// Arah port device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortDir {
    Input,
    Output,
    Inout,
}

/// Satu port device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortDesc {
    pub name: Symbol,
    pub direction: PortDir,
    pub width: usize,
}

/// Register hasil inferensi FF (target NBA di proses Sequential).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MhirRegister {
    pub name: Symbol,
    pub width: usize,
    pub clock: Option<Symbol>,
    pub reset: Option<Symbol>,
    pub back: BackPointer,
}

/// Jenis memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryKind {
    Ram,
    Rom,
}

/// Memory (array signal) — calon region RAM/ROM pada memory map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MhirMemory {
    pub name: Symbol,
    pub elem_width: usize,
    pub depth: usize,
    pub dims: Vec<usize>,
    pub kind: MemoryKind,
    pub back: BackPointer,
}

/// Jenis device (heuristik dari nama module).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Cpu,
    Uart,
    Timer,
    InterruptController,
    Plic,
    Clint,
    Dma,
    Pcie,
    Network,
    Storage,
    Virtio,
    Gpio,
    Spi,
    I2c,
    Other,
}

impl DeviceKind {
    /// Heuristik klasifikasi dari nama module (case-insensitive, substring).
    pub fn from_module_name(name: &str) -> DeviceKind {
        let n = name.to_ascii_lowercase();
        if n.contains("uart") {
            DeviceKind::Uart
        } else if n.contains("cpu")
            || n.contains("core")
            || n.contains("riscv")
            || n.contains("picorv")
            || n.contains("ibex")
            || n.contains("cva6")
            || n.contains("ariane")
            || n.contains("rv32")
            || n.contains("rv64")
            || n.contains("c910")
            || n.contains("c906")
            || n.contains("rocket")
        {
            DeviceKind::Cpu
        } else if n.contains("clint") {
            DeviceKind::Clint
        } else if n.contains("plic") || n.contains("intr") || n.contains("interrupt") {
            DeviceKind::InterruptController
        } else if n.contains("timer") {
            DeviceKind::Timer
        } else if n.contains("dma") {
            DeviceKind::Dma
        } else if n.contains("pcie") || n.contains("pci") {
            DeviceKind::Pcie
        } else if n.contains("virtio") {
            DeviceKind::Virtio
        } else if n.contains("eth") || n.contains("mac") || n.contains("net") {
            DeviceKind::Network
        } else if n.contains("sd")
            || n.contains("flash")
            || n.contains("block")
            || n.contains("mmc")
        {
            DeviceKind::Storage
        } else if n.contains("gpio") {
            DeviceKind::Gpio
        } else if n.contains("spi") {
            DeviceKind::Spi
        } else if n.contains("i2c") {
            DeviceKind::I2c
        } else {
            DeviceKind::Other
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            DeviceKind::Cpu => "cpu",
            DeviceKind::Uart => "uart",
            DeviceKind::Timer => "timer",
            DeviceKind::InterruptController => "interrupt-controller",
            DeviceKind::Plic => "plic",
            DeviceKind::Clint => "clint",
            DeviceKind::Dma => "dma",
            DeviceKind::Pcie => "pcie",
            DeviceKind::Network => "network",
            DeviceKind::Storage => "storage",
            DeviceKind::Virtio => "virtio",
            DeviceKind::Gpio => "gpio",
            DeviceKind::Spi => "spi",
            DeviceKind::I2c => "i2c",
            DeviceKind::Other => "other",
        }
    }
}

/// Region alamat (MMIO/RAM/ROM).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressRegion {
    pub base: u64,
    pub size: u64,
}

/// Hasil ekstraksi MHIR untuk satu module.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MhirModule {
    pub name: Symbol,
    /// Jumlah sinyal di module (ringkasan).
    pub signal_count: usize,
    pub clocks: Vec<ClockDesc>,
    pub resets: Vec<ResetDesc>,
    pub registers: Vec<MhirRegister>,
    pub memories: Vec<MhirMemory>,
    pub devices: Vec<MhirDevice>,
}

/// Device — instance di bawah module (atau module dengan anotasi).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MhirDevice {
    /// Nama instance (`u_uart`).
    pub name: Symbol,
    /// Module asal (`uart`).
    pub module: Symbol,
    pub kind: DeviceKind,
    pub ports: Vec<PortDesc>,
    /// Region MMIO — diisi via `apply_address_map` (`--addr` / `[emu]`).
    pub mmio: Option<AddressRegion>,
    /// Line IRQ (anotasi / `[emu]`) — R0: None sampai mapping disediakan.
    pub irq: Option<u32>,
    pub back: BackPointer,
}

/// MHIR keseluruhan desain.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MhirDesign {
    pub top: Symbol,
    /// Module top (flattened) dulu, lalu definisi module lain (urut nama).
    pub modules: Vec<MhirModule>,
    /// Peta nama (device/memory) → region alamat, diurutkan by base.
    pub address_map: Vec<(Symbol, AddressRegion)>,
    /// File sumber utama (bila diketahui).
    pub source_file: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backpointer_display() {
        assert_eq!(BackPointer::default().display(), "-");
        assert_eq!(
            BackPointer::known(Some("uart.sv".into()), 143, 5).display(),
            "uart.sv:143:5"
        );
        assert_eq!(
            BackPointer::known(Some("top.sv".into()), 12, 0).display(),
            "top.sv:12"
        );
        assert_eq!(BackPointer::known(None, 7, 3).display(), "7:3");
    }

    #[test]
    fn test_device_kind_heuristics() {
        assert_eq!(DeviceKind::from_module_name("uart_16550"), DeviceKind::Uart);
        assert_eq!(DeviceKind::from_module_name("picorv32"), DeviceKind::Cpu);
        assert_eq!(
            DeviceKind::from_module_name("rv_plic"),
            DeviceKind::InterruptController
        );
        assert_eq!(DeviceKind::from_module_name("clint"), DeviceKind::Clint);
        assert_eq!(
            DeviceKind::from_module_name("virtio_mmio"),
            DeviceKind::Virtio
        );
        assert_eq!(DeviceKind::from_module_name("spi_device"), DeviceKind::Spi);
        assert_eq!(
            DeviceKind::from_module_name("weird_thing"),
            DeviceKind::Other
        );
    }
}
